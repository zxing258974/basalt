//! 登录 SSH 服务器:密码(服务器只开了键盘交互时也用这个密码答)、私钥(可带口令),或者 ssh-agent。
//! 跳板机不问密码:依次试 ssh-agent、~/.ssh/config 给它配的私钥(没配就 OpenSSH 默认的那几个)、
//! SSH 服务器用的私钥。

use std::sync::Arc;

use russh::MethodKind;
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};

use super::ssh_config::default_identity_files;
use super::{JumpHop, SshAuth, SshParams};
use crate::error::{CoreError, CoreResult};

/// 键盘交互最多来回几轮(空的提问也算一轮)。
const MAX_INTERACTIVE_ROUNDS: usize = 4;

const MORE_FACTORS: &str = "SSH 服务器还要求别的验证（比如动态口令），Basalt 暂不支持";

pub(super) async fn authenticate<H: client::Handler>(
    handle: &mut client::Handle<H>,
    params: &SshParams,
) -> CoreResult<()> {
    let user = params.username.as_str();
    match &params.auth {
        SshAuth::Password(None) => Err(CoreError::SshSecretRequired(format!(
            "需要 {user}@{} 的 SSH 密码",
            params.host
        ))),
        SshAuth::Password(Some(password)) => password_login(handle, user, password).await,
        SshAuth::PrivateKey { path, passphrase } => {
            let key = load_private_key(path, passphrase.as_deref()).await?;
            let hash = handle
                .best_supported_rsa_hash()
                .await
                .map_err(super::session_error)?
                .flatten();
            let result = handle
                .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(super::session_error)?;
            check(result, || {
                format!(
                    "SSH 服务器不接受这个私钥登录 {user}（服务器上 {user} 的 authorized_keys 里没有对应的公钥？）"
                )
            })
        }
        SshAuth::Agent => {
            let mut agent = agent::connect(params.agent.as_deref()).await?;
            if agent::login(handle, user, &mut agent).await? {
                Ok(())
            } else {
                Err(CoreError::SshAuthentication(format!(
                    "SSH 服务器不接受 ssh-agent 里的私钥登录 {user}（用 ssh-add -l 看看加进去的是哪几把）"
                )))
            }
        }
    }
}

/// 登录一台跳板机:不问密码,依次试 ssh-agent、配置里给它的私钥、SSH 服务器用的私钥。
pub(super) async fn authenticate_jump<H: client::Handler>(
    handle: &mut client::Handle<H>,
    params: &SshParams,
    jump: &JumpHop,
) -> CoreResult<()> {
    let user = jump.username.as_str();
    let mut tried: Vec<String> = Vec::new();
    match agent::connect(jump.agent.as_deref()).await {
        Ok(mut agent) => {
            if agent::login(handle, user, &mut agent).await? {
                return Ok(());
            }
            tried.push("ssh-agent".into());
        }
        Err(error) => log::debug!("跳板机不用 ssh-agent：{error}"),
    }
    let files: Vec<String> = if jump.identity_files.is_empty() {
        default_identity_files()
            .into_iter()
            .filter(|path| path.is_file())
            .map(|path| path.display().to_string())
            .collect()
    } else {
        jump.identity_files.clone()
    };
    for path in &files {
        // 读不了、带口令的跳过:跳板机不问口令。
        let Ok(key) = load_private_key(path, None).await else {
            continue;
        };
        if publickey_login(handle, user, key).await? {
            return Ok(());
        }
        tried.push(path.clone());
    }
    if let SshAuth::PrivateKey { path, passphrase } = &params.auth
        && !files.contains(path)
        && let Ok(key) = load_private_key(path, passphrase.as_deref()).await
    {
        if publickey_login(handle, user, key).await? {
            return Ok(());
        }
        tried.push(path.clone());
    }
    let tried = if tried.is_empty() {
        "没有可用的 ssh-agent 和私钥".to_owned()
    } else {
        format!("试过 {}", tried.join("、"))
    };
    Err(CoreError::SshAuthentication(format!(
        "跳板机 {user}@{} 不让登录：{tried}。跳板机要用 ssh-agent 或私钥登录（可以在 ~/.ssh/config 里给它配 IdentityFile）",
        super::hop_address(&jump.host, jump.port)
    )))
}

/// 用一把私钥登录;服务器不接受返回 false。
async fn publickey_login<H: client::Handler>(
    handle: &mut client::Handle<H>,
    user: &str,
    key: PrivateKey,
) -> CoreResult<bool> {
    let hash = handle
        .best_supported_rsa_hash()
        .await
        .map_err(super::session_error)?
        .flatten();
    let result = handle
        .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
        .await
        .map_err(super::session_error)?;
    accepted(result)
}

/// 服务器接受了就是 true;要再验证一道(动态口令)是 Basalt 处理不了的错误。
fn accepted(result: client::AuthResult) -> CoreResult<bool> {
    match result {
        client::AuthResult::Success => Ok(true),
        client::AuthResult::Failure {
            partial_success: true,
            ..
        } => Err(CoreError::SshAuthentication(MORE_FACTORS.into())),
        client::AuthResult::Failure { .. } => Ok(false),
    }
}

/// ssh-agent:Unix 上是 SSH_AUTH_SOCK(或 IdentityAgent)指着的套接字。
#[cfg(unix)]
mod agent {
    use russh::client;
    use russh::keys::agent::AgentIdentity;
    use russh::keys::agent::client::AgentClient;
    use tokio::net::UnixStream;

    use crate::error::{CoreError, CoreResult};

    pub(super) async fn connect(socket: Option<&str>) -> CoreResult<AgentClient<UnixStream>> {
        let path = match socket {
            Some(path) => path.to_owned(),
            None => std::env::var("SSH_AUTH_SOCK").map_err(|_| {
                CoreError::Ssh(
                    "找不到 ssh-agent（没有 SSH_AUTH_SOCK）。先启动 ssh-agent 并用 ssh-add 加上私钥，或者改用私钥文件登录"
                        .into(),
                )
            })?,
        };
        AgentClient::connect_uds(&path)
            .await
            .map_err(|error| CoreError::Ssh(format!("连不上 ssh-agent（{path}）：{error}")))
    }

    /// 把 ssh-agent 里的私钥挨个试一遍;都不被接受返回 false。agent 里一把私钥都没有时直接说。
    pub(super) async fn login<H: client::Handler>(
        handle: &mut client::Handle<H>,
        user: &str,
        agent: &mut AgentClient<UnixStream>,
    ) -> CoreResult<bool> {
        let identities = agent
            .request_identities()
            .await
            .map_err(|error| CoreError::Ssh(format!("读 ssh-agent 里的私钥失败：{error}")))?;
        let keys: Vec<_> = identities
            .into_iter()
            .filter_map(|identity| match identity {
                AgentIdentity::PublicKey { key, .. } => Some(key),
                // OpenSSH 证书登录不支持,跳过。
                AgentIdentity::Certificate { .. } => None,
            })
            .collect();
        if keys.is_empty() {
            return Err(CoreError::SshAuthentication(
                "ssh-agent 里没有私钥，先用 ssh-add 加进去".into(),
            ));
        }
        let hash = handle
            .best_supported_rsa_hash()
            .await
            .map_err(super::super::session_error)?
            .flatten();
        for key in keys {
            let result = handle
                .authenticate_publickey_with(user, key, hash, agent)
                .await
                .map_err(|error| CoreError::Ssh(format!("ssh-agent 签名失败：{error}")))?;
            if super::accepted(result)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// 不是 Unix 的系统上不支持 ssh-agent。
#[cfg(not(unix))]
mod agent {
    use russh::client;

    use crate::error::{CoreError, CoreResult};

    pub(super) struct NoAgent;

    pub(super) async fn connect(_socket: Option<&str>) -> CoreResult<NoAgent> {
        Err(CoreError::Unsupported(
            "这个系统上不支持 ssh-agent 登录".into(),
        ))
    }

    pub(super) async fn login<H: client::Handler>(
        _handle: &mut client::Handle<H>,
        _user: &str,
        _agent: &mut NoAgent,
    ) -> CoreResult<bool> {
        Ok(false)
    }
}

async fn password_login<H: client::Handler>(
    handle: &mut client::Handle<H>,
    user: &str,
    password: &str,
) -> CoreResult<()> {
    let result = handle
        .authenticate_password(user, password)
        .await
        .map_err(super::session_error)?;
    let wrong = || "SSH 用户名或密码不正确".to_owned();
    let client::AuthResult::Failure {
        remaining_methods,
        partial_success: false,
    } = &result
    else {
        return check(result, wrong);
    };
    // 只开了 PAM 键盘交互的服务器不认 password 方式:用同一个密码回答它的提问。
    if !remaining_methods.contains(&MethodKind::KeyboardInteractive) {
        return check(result, wrong);
    }
    if keyboard_interactive(handle, user, password).await? {
        Ok(())
    } else {
        Err(CoreError::SshAuthentication(wrong()))
    }
}

async fn keyboard_interactive<H: client::Handler>(
    handle: &mut client::Handle<H>,
    user: &str,
    password: &str,
) -> CoreResult<bool> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await
        .map_err(super::session_error)?;
    let mut answered = false;
    for _ in 0..MAX_INTERACTIVE_ROUNDS {
        let prompts = match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => prompts,
        };
        // 密码只答一轮。之后还有提问,多半是在要动态口令:拿密码去答会白白耗掉它的出错次数。
        if answered && !prompts.is_empty() {
            return Err(CoreError::SshAuthentication(MORE_FACTORS.into()));
        }
        answered |= !prompts.is_empty();
        // 不回显的提问是在要密码;回显的(问用户名之类)答空。没有提问的一轮(只是提示信息)回空。
        let answers = prompts
            .iter()
            .map(|p| {
                if p.echo {
                    String::new()
                } else {
                    password.to_owned()
                }
            })
            .collect();
        response = handle
            .authenticate_keyboard_interactive_respond(answers)
            .await
            .map_err(super::session_error)?;
    }
    Ok(false)
}

fn check(result: client::AuthResult, rejected: impl FnOnce() -> String) -> CoreResult<()> {
    match result {
        client::AuthResult::Success => Ok(()),
        client::AuthResult::Failure {
            partial_success: true,
            ..
        } => Err(CoreError::SshAuthentication(MORE_FACTORS.into())),
        client::AuthResult::Failure { .. } => Err(CoreError::SshAuthentication(rejected())),
    }
}

/// 读私钥。口令为空字符串当作没有口令。错误里不带口令和私钥内容。
pub(super) async fn load_private_key(
    path: &str,
    passphrase: Option<&str>,
) -> CoreResult<PrivateKey> {
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| CoreError::File(format!("读不了私钥文件 {path}：{error}")))?;
    let passphrase = passphrase.filter(|p| !p.is_empty());
    if text.trim_start().starts_with("PuTTY-User-Key-File") {
        return Err(CoreError::Unsupported(format!(
            "{path} 是 PuTTY 的 .ppk 私钥，Basalt 读不了。请用 PuTTYgen「Conversions → Export OpenSSH key」导出成 OpenSSH 格式再选它"
        )));
    }
    let error = match russh::keys::decode_secret_key(&text, passphrase) {
        Ok(key) => return Ok(key),
        Err(error) => error,
    };
    let encrypted = matches!(error, russh::keys::Error::KeyIsEncrypted)
        || (passphrase.is_some() && is_encrypted(&text));
    Err(match (encrypted, passphrase) {
        (true, None) => CoreError::SshSecretRequired(format!("私钥 {path} 有口令，请输入私钥口令")),
        (true, Some(_)) => CoreError::SshAuthentication(format!("私钥口令不正确（{path}）")),
        (false, _) => CoreError::Unsupported(format!(
            "认不出私钥文件 {path} 的格式：{error}。支持 OpenSSH、PKCS#8 和 PEM 格式的 Ed25519、ECDSA、RSA 私钥"
        )),
    })
}

/// 不带口令解一次:说「有口令」,说明前面失败是口令不对,而不是格式认不出。
fn is_encrypted(text: &str) -> bool {
    matches!(
        russh::keys::decode_secret_key(text, None),
        Err(russh::keys::Error::KeyIsEncrypted)
    )
}
