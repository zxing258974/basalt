//! 登录 SSH 服务器:密码(服务器只开了键盘交互时也用这个密码答),或者私钥(可带口令)。

use std::sync::Arc;

use russh::MethodKind;
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{PrivateKey, PrivateKeyWithHashAlg};

use super::{SshAuth, SshParams};
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
