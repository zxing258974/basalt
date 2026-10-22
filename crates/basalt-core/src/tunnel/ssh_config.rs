//! 读 `~/.ssh/config`:连接对话框里按 Host 别名填表,跳板机按别名找到真正的地址、用户和私钥,
//! ssh-agent 按 `IdentityAgent` 找套接字(比如 1Password 的)。只读,读不了或写得不对时当作没有。

use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::Serialize;
use ssh2_config::{ParseRule, SshConfig};

/// 某个主机名(或别名)在配置里的设置,按 OpenSSH 的规矩:匹配的 Host 段里第一次出现的值算数。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostSettings {
    pub host_name: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_files: Vec<String>,
    /// ProxyJump 原样(逗号分隔);`none` 当作没有。
    pub proxy_jump: Option<String>,
    /// ssh-agent 的套接字;`SSH_AUTH_SOCK`、`none` 和没写一样,用环境变量里的。
    pub identity_agent: Option<String>,
}

/// 给连接对话框选的一个 Host 别名和它解析出来的值。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigHost {
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    /// 第一个 IdentityFile;没写时为空,用 ssh-agent 登录。
    pub private_key_path: Option<String>,
    pub jump_hosts: Option<String>,
}

pub struct SshConfigFile {
    config: Option<SshConfig>,
    /// 展开 `~` 用的主目录。
    home: PathBuf,
}

impl SshConfigFile {
    pub fn empty() -> Self {
        Self {
            config: None,
            home: PathBuf::new(),
        }
    }

    /// 读当前用户的 `~/.ssh/config`(连同它 Include 的文件)。
    pub fn load() -> Self {
        let Some(home) = std::env::home_dir() else {
            return Self::empty();
        };
        let path = home.join(".ssh/config");
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("读不了 {}：{error}", path.display());
                }
                return Self::empty();
            }
        };
        Self::read(&mut BufReader::new(file), &home).unwrap_or_else(|error| {
            log::warn!("{} 读不懂，当作没有：{error}", path.display());
            Self::empty()
        })
    }

    /// `home` 用来展开 `~` 和相对路径的 Include。
    pub fn parse(text: &str, home: &Path) -> Result<Self, String> {
        Self::read(&mut text.as_bytes(), home)
    }

    fn read(reader: &mut impl std::io::BufRead, home: &Path) -> Result<Self, String> {
        let rules = ParseRule::ALLOW_UNKNOWN_FIELDS | ParseRule::ALLOW_UNSUPPORTED_FIELDS;
        SshConfig::default()
            .home_dir(home)
            .parse(reader, rules)
            .map(|config| Self {
                config: Some(config),
                home: home.to_path_buf(),
            })
            .map_err(|error| error.to_string())
    }

    pub fn lookup(&self, name: &str) -> HostSettings {
        let Some(config) = &self.config else {
            return HostSettings::default();
        };
        let params = config.query(name);
        let identity_agent = config.intersecting_hosts(name).find_map(|host| {
            host.params
                .unsupported_fields
                .iter()
                .find(|(field, _)| field.eq_ignore_ascii_case("identityagent"))
                .map(|(_, args)| self.unquote(&args.join(" ")))
        });
        HostSettings {
            host_name: params.host_name,
            port: params.port,
            user: params.user,
            identity_files: params
                .identity_file
                .unwrap_or_default()
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            proxy_jump: params
                .proxy_jump
                .map(|hops| hops.join(","))
                .filter(|jump| !jump.eq_ignore_ascii_case("none")),
            identity_agent: identity_agent.filter(|agent| {
                !agent.eq_ignore_ascii_case("none")
                    && !agent.trim_start_matches('$').eq("SSH_AUTH_SOCK")
            }),
        }
    }

    /// 值两边的双引号去掉,开头的 `~` 按配置文件的主目录展开。
    fn unquote(&self, value: &str) -> String {
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        match value.strip_prefix("~/") {
            Some(rest) => self.home.join(rest).display().to_string(),
            None => value.to_owned(),
        }
    }

    /// 写明了名字的 Host 别名(带 `*` `?` 和 `!` 的通配段不算),按文件里的顺序。
    pub fn hosts(&self) -> Vec<SshConfigHost> {
        let Some(config) = &self.config else {
            return Vec::new();
        };
        let mut aliases: Vec<&str> = Vec::new();
        for host in config.get_hosts() {
            for clause in &host.pattern {
                let concrete = !clause.negated && !clause.pattern.contains(['*', '?']);
                if concrete && !aliases.contains(&clause.pattern.as_str()) {
                    aliases.push(&clause.pattern);
                }
            }
        }
        aliases
            .into_iter()
            .map(|alias| {
                let settings = self.lookup(alias);
                SshConfigHost {
                    alias: alias.to_owned(),
                    host: settings.host_name.unwrap_or_else(|| alias.to_owned()),
                    port: settings.port.unwrap_or(crate::config::DEFAULT_SSH_PORT),
                    username: settings.user,
                    private_key_path: settings.identity_files.into_iter().next(),
                    jump_hosts: settings.proxy_jump,
                }
            })
            .collect()
    }
}

/// 没写 IdentityFile 时 OpenSSH 依次试的私钥。
pub(crate) fn default_identity_files() -> Vec<PathBuf> {
    let Some(home) = std::env::home_dir() else {
        return Vec::new();
    };
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|name| home.join(".ssh").join(name))
        .collect()
}

#[cfg(test)]
#[path = "ssh_config_tests.rs"]
mod tests;
