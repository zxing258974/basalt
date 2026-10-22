//! Navicat 连接里的 SSH 隧道和 SSL 设置 → Basalt 的 [`SshTunnel`] / [`SslSettings`]。
//! 能对上的照搬,对不上的(HTTP 隧道、加密套件、吊销列表、带口令的客户端私钥、别的电脑上的文件路径……)
//! 不猜,写进提醒让用户导入后自己补。

use std::path::Path;

use crate::config::{DEFAULT_SSH_PORT, DriverKind, SshAuthMethod, SshTunnel, SslMode, SslSettings};
use crate::import::SourcePassword;

/// .ncx 里 `SSH="true"` 时的那些属性,原样文本。
#[derive(Debug, Default)]
pub(super) struct RawSsh {
    pub host: String,
    pub port: String,
    pub username: String,
    /// PASSWORD 或 PUBLICKEY,老的导出文件里可能没有。
    pub auth_method: String,
    pub private_key: String,
    pub password: SourcePassword,
    pub passphrase: SourcePassword,
}

/// .ncx 里 `SSL="true"` 时的那些属性。
#[derive(Clone, Debug, Default)]
pub(super) struct RawSsl {
    /// PostgreSQL 的 sslmode:REQUIRE、VERIFY-CA、VERIFY-FULL……
    pub pg_mode: String,
    /// MySQL 的「验证 CA 证书」。
    pub verify_ca: bool,
    /// 「允许无效的主机名」;老的导出文件没有这一项时为 `None`(不知道当时核不核对主机名)。
    pub allow_invalid_host_name: Option<bool>,
    /// 「使用验证」:用客户端证书登录。
    pub client_auth: bool,
    pub ca_cert: String,
    pub client_cert: String,
    pub client_key: String,
    pub client_key_password: bool,
    pub cipher: String,
    pub crl: String,
}

/// 映射结果。`ssh_secret` 是 SSH 密码或私钥口令。
#[derive(Debug, Default)]
pub(super) struct Transport {
    pub ssh: Option<SshTunnel>,
    pub ssh_secret: SourcePassword,
    pub ssl: SslSettings,
    pub warnings: Vec<String>,
}

pub(super) fn map_transport(
    driver: DriverKind,
    ssh: Option<&RawSsh>,
    ssl: Option<&RawSsl>,
) -> Transport {
    let mut warnings = Vec::new();
    let (ssh, ssh_secret) = match ssh {
        Some(raw) => map_ssh(raw, &mut warnings),
        None => (None, SourcePassword::Absent),
    };
    let ssl = match ssl {
        Some(raw) => map_ssl(driver, raw, ssh.is_some(), &mut warnings),
        None => SslSettings::default(),
    };
    Transport {
        ssh,
        ssh_secret,
        ssl,
        warnings,
    }
}

/// 这台电脑上能用的绝对路径。别的系统上导出的(Windows 的 `C:\…`)在这里用不了。
fn usable_path(path: &str) -> Option<String> {
    let path = path.trim();
    (!path.is_empty() && Path::new(path).is_absolute()).then(|| path.to_owned())
}

fn map_ssh(raw: &RawSsh, warnings: &mut Vec<String>) -> (Option<SshTunnel>, SourcePassword) {
    let host = raw.host.trim();
    let username = raw.username.trim();
    if host.is_empty() || host.chars().any(char::is_whitespace) || username.is_empty() {
        warnings.push("SSH 隧道的主机或用户名没填全，隧道没有迁移，导入后请在「SSH」页补上".into());
        return (None, SourcePassword::Absent);
    }
    let port = match raw.port.trim() {
        "" => DEFAULT_SSH_PORT,
        text => text
            .parse::<u16>()
            .ok()
            .filter(|p| *p > 0)
            .unwrap_or_else(|| {
                warnings.push(format!("SSH 端口「{text}」无效，已改用 {DEFAULT_SSH_PORT}"));
                DEFAULT_SSH_PORT
            }),
    };
    // PUBLICKEY(注册表里是 saPublicKey);没写或别的都按密码登录,Navicat 默认就是密码。
    let wants_key = raw.auth_method.to_ascii_uppercase().contains("KEY");
    let key_path = usable_path(&raw.private_key);
    let (auth, private_key_path, secret) = match (wants_key, key_path) {
        (false, _) => (SshAuthMethod::Password, None, raw.password.clone()),
        (true, Some(path)) => (
            SshAuthMethod::PrivateKey,
            Some(path),
            raw.passphrase.clone(),
        ),
        (true, None) => {
            let shown = raw.private_key.trim();
            warnings.push(if shown.is_empty() {
                "SSH 用私钥登录，但没有私钥文件的路径；先改成了密码登录，请在「SSH」页选择私钥".into()
            } else {
                format!("SSH 私钥 {shown} 在这台电脑上用不了；先改成了密码登录，请在「SSH」页重新选择私钥")
            });
            (SshAuthMethod::Password, None, SourcePassword::Absent)
        }
    };
    if secret == SourcePassword::Undecryptable {
        warnings.push("SSH 密码（或私钥口令）解不开，首次连接时需要输入".into());
    }
    let tunnel = SshTunnel {
        host: host.to_owned(),
        port,
        username: username.to_owned(),
        auth,
        private_key_path,
        save_secret: matches!(secret, SourcePassword::Plain(_)),
    };
    (Some(tunnel), secret)
}

fn map_ssl(
    driver: DriverKind,
    raw: &RawSsl,
    tunneled: bool,
    warnings: &mut Vec<String>,
) -> SslSettings {
    let mut mode = match driver {
        DriverKind::Postgres => pg_mode(&raw.pg_mode, warnings),
        // Navicat 校验 CA、又明确不允许无效主机名:原来是连主机名一起核对的,不能导进来就变弱。
        _ if raw.verify_ca && raw.allow_invalid_host_name == Some(false) => SslMode::VerifyIdentity,
        _ if raw.verify_ca => SslMode::VerifyCa,
        _ => SslMode::Required,
    };
    if mode == SslMode::VerifyIdentity && raw.allow_invalid_host_name == Some(true) {
        mode = SslMode::VerifyCa;
    }
    if mode == SslMode::VerifyIdentity && tunneled {
        warnings.push("经 SSH 隧道时没法核对证书里的主机名，SSL 已改成「校验 CA」".into());
        mode = SslMode::VerifyCa;
    }

    let mut file = |label: &str, path: &str| -> Option<String> {
        let usable = usable_path(path);
        if usable.is_none() && !path.trim().is_empty() {
            warnings.push(format!(
                "{label} {} 在这台电脑上用不了，没有迁移，请在「SSL」页重新选择",
                path.trim()
            ));
        }
        usable
    };
    let ca_path = file("CA 证书", &raw.ca_cert);
    let (client_cert_path, client_key_path) = if raw.client_auth {
        let cert = file("客户端证书", &raw.client_cert);
        let key = file("客户端私钥", &raw.client_key);
        match (cert, key) {
            (Some(cert), Some(key)) => (Some(cert), Some(key)),
            (None, None) => (None, None),
            _ => {
                warnings
                    .push("客户端证书和私钥只迁移了一个，Basalt 要两个一起用，都没有迁移".into());
                (None, None)
            }
        }
    } else {
        (None, None)
    };
    if raw.client_key_password {
        warnings.push("客户端私钥的口令没有迁移：Basalt 只能用没有口令的客户端私钥".into());
    }
    if !raw.cipher.trim().is_empty() {
        warnings.push(format!("指定的加密套件（{}）没有迁移", raw.cipher.trim()));
    }
    if !raw.crl.trim().is_empty() {
        warnings.push("证书吊销列表（CRL）没有迁移".into());
    }
    SslSettings {
        mode,
        ca_path,
        client_cert_path,
        client_key_path,
    }
}

fn pg_mode(raw: &str, warnings: &mut Vec<String>) -> SslMode {
    match raw.trim().to_ascii_uppercase().as_str() {
        "DISABLE" => SslMode::Disabled,
        "ALLOW" | "PREFER" => SslMode::Preferred,
        "" | "REQUIRE" => SslMode::Required,
        "VERIFY-CA" => SslMode::VerifyCa,
        "VERIFY-FULL" => SslMode::VerifyIdentity,
        other => {
            warnings.push(format!("认不出 SSL 模式「{other}」，已按「必须」导入"));
            SslMode::Required
        }
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
