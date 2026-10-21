//! SSH 隧道相关的命令和保存连接时对两条密码的处理:数据库密码与 SSH 密码(或私钥口令)
//! 各占密钥库里的一个键,一起改、一起回滚。

use basalt_core::CoreError;
use basalt_core::config::{ConnectionConfig, SshTunnel};
use basalt_core::secret::ssh_secret_key;
use basalt_core::tunnel::HostKey;

use super::{CommandResult, SecretChange, restore_secret};
use crate::state::AppState;

/// 一个密钥库键这次怎么改、改之前是什么(回滚用)。
pub(super) struct SecretEdit {
    key: String,
    change: SecretChange,
    previous: Option<String>,
}

impl SecretEdit {
    /// 原来就没存(`had_saved` 为假)时不去读:读了也是空,还可能弹授权框。
    /// 要删的本来就没有,当作不变。
    pub(super) fn plan(
        state: &AppState,
        key: String,
        change: SecretChange,
        had_saved: bool,
    ) -> CommandResult<Self> {
        let previous = match change {
            SecretChange::Keep => None,
            _ if !had_saved => None,
            _ => state.secrets.get(&key)?,
        };
        let change = match change {
            SecretChange::Delete if previous.is_none() => SecretChange::Keep,
            other => other,
        };
        Ok(Self {
            key,
            change,
            previous,
        })
    }

    pub(super) fn change(&self) -> &SecretChange {
        &self.change
    }

    fn apply(&self, state: &AppState) -> CommandResult<()> {
        match &self.change {
            SecretChange::Keep => {}
            SecretChange::Set(secret) => state.secrets.set(&self.key, secret)?,
            SecretChange::Delete => state.secrets.delete(&self.key)?,
        }
        Ok(())
    }

    fn revert(&self, state: &AppState) {
        if self.change != SecretChange::Keep {
            restore_secret(state, &self.key, self.previous.as_deref());
        }
    }
}

/// 按顺序写;中途失败就把已经写了的恢复原样。
pub(super) fn apply_all(state: &AppState, edits: &[SecretEdit]) -> CommandResult<()> {
    for (done, edit) in edits.iter().enumerate() {
        if let Err(error) = edit.apply(state) {
            revert_all(state, &edits[..done]);
            return Err(error);
        }
    }
    Ok(())
}

pub(super) fn revert_all(state: &AppState, edits: &[SecretEdit]) {
    for edit in edits.iter().rev() {
        edit.revert(state);
    }
}

/// 隧道这次的密码怎么处理:没有隧道了就删掉存着的。
pub(super) fn ssh_secret_change(ssh: Option<&SshTunnel>, secret: Option<String>) -> SecretChange {
    match ssh {
        Some(tunnel) => super::secret_change(tunnel.save_secret, secret),
        None => SecretChange::Delete,
    }
}

/// 连到哪儿的设置变了没有(隧道换了服务器、换了登录方式,SSL 改了)。只改「是否保存密码」不算。
pub(super) fn route_changed(previous: &ConnectionConfig, next: &ConnectionConfig) -> bool {
    let tunnel = |config: &ConnectionConfig| {
        config.ssh.clone().map(|t| SshTunnel {
            save_secret: false,
            ..t
        })
    };
    tunnel(previous) != tunnel(next) || previous.ssl != next.ssl
}

/// 密码框里勾了「记住」、而且已经连上了:只写密钥库和 saveSecret,不碰会话。
pub async fn remember_ssh_secret(state: &AppState, id: &str, secret: String) -> CommandResult<()> {
    let store = state.store().await;
    let list = store.load()?;
    let config = list
        .get(id)
        .cloned()
        .ok_or_else(|| CoreError::ConnectionNotFound(id.to_owned()))?;
    let Some(tunnel) = config.ssh.clone() else {
        return Err(CoreError::Unsupported("这个连接没有用 SSH 隧道".into()).into());
    };
    let key = ssh_secret_key(id);
    let previous = state.secrets.get(&key)?;
    state.secrets.set(&key, &secret)?;
    if tunnel.save_secret {
        return Ok(());
    }
    let next = ConnectionConfig {
        ssh: Some(SshTunnel {
            save_secret: true,
            ..tunnel
        }),
        ..config
    };
    if let Err(error) = store.save(&list.upserted(next)) {
        restore_secret(state, &key, previous.as_deref());
        return Err(error.into());
    }
    Ok(())
}

/// 用户核对过指纹,记下这台 SSH 服务器的主机密钥。
pub async fn trust_host_key(state: &AppState, host_key: &HostKey) -> CommandResult<()> {
    state.known_hosts.trust(host_key)?;
    log::info!(
        "已信任 SSH 服务器 {} 的主机密钥 {}",
        host_key.address(),
        host_key.fingerprint
    );
    Ok(())
}

#[cfg(test)]
#[path = "tunnel_tests.rs"]
mod tests;
