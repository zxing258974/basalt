//! SSH 隧道(本地端口转发):登录 SSH 服务器,在 127.0.0.1 的随机端口上监听,
//! 每进来一条连接就开一个 direct-tcpip 通道转发到数据库。驱动只管连本机这个端口,不知道有隧道。
//!
//! 隧道归会话所有:会话关闭时先关连接池,再关隧道。SSH 连接断了(网络切换、服务器重启)不用重开会话,
//! 下一条新连接进来时重新登录;主机密钥照样核对。

mod auth;
mod known_hosts;

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use russh::client;
use russh::keys::PublicKeyOrCertificate;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::{JoinHandle, JoinSet};

pub use known_hosts::{HostKey, HostKeyCheck, KnownHosts, fingerprint};

use crate::config::{SshAuthMethod, SshTunnel};
use crate::driver::CONNECT_TIMEOUT;
use crate::error::{CoreError, CoreResult};

/// 空闲时每隔多久发一次心跳,免得路由器、防火墙把安静的连接掐掉;连着 3 次没回应算断开。
const KEEPALIVE: Duration = Duration::from_secs(30);
/// 本机端口 accept 出错(比如文件句柄用完)后歇一会儿再接,不空转。
const ACCEPT_BACKOFF: Duration = Duration::from_millis(200);

/// SSH 登录方式和凭据。`Debug` 不输出密码和口令。
#[derive(Clone, PartialEq, Eq)]
pub enum SshAuth {
    /// `None`:没存也没问到密码。
    Password(Option<String>),
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
}

/// 建隧道所需的全部参数。
#[derive(Clone, PartialEq, Eq)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    pub known_hosts: KnownHosts,
}

impl SshParams {
    /// `secret` 是 SSH 密码或私钥口令。
    pub fn from_config(
        tunnel: &SshTunnel,
        secret: Option<String>,
        known_hosts: KnownHosts,
    ) -> Self {
        let auth = match tunnel.auth {
            SshAuthMethod::Password => SshAuth::Password(secret),
            SshAuthMethod::PrivateKey => SshAuth::PrivateKey {
                path: tunnel.private_key_path.clone().unwrap_or_default(),
                passphrase: secret.filter(|s| !s.is_empty()),
            },
        };
        Self {
            host: tunnel.host.clone(),
            port: tunnel.port,
            username: tunnel.username.clone(),
            auth,
            known_hosts,
        }
    }

    fn address(&self) -> String {
        HostKey {
            host: self.host.clone(),
            port: self.port,
            algorithm: String::new(),
            fingerprint: String::new(),
            public_key: String::new(),
        }
        .address()
    }
}

impl fmt::Debug for SshAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let redacted = |secret: &Option<String>| if secret.is_some() { "***" } else { "None" };
        match self {
            Self::Password(password) => write!(f, "Password({})", redacted(password)),
            Self::PrivateKey { path, passphrase } => {
                write!(
                    f,
                    "PrivateKey({path}, passphrase: {})",
                    redacted(passphrase)
                )
            }
        }
    }
}

impl fmt::Debug for SshParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("auth", &self.auth)
            .field("known_hosts", &self.known_hosts.path())
            .finish()
    }
}

pub struct Tunnel {
    local: SocketAddr,
    shared: Arc<Shared>,
    forwarder: JoinHandle<()>,
}

impl Tunnel {
    /// 登录 SSH 服务器并开始在本机端口上转发到 `target_host:target_port`(从 SSH 服务器那边看的地址)。
    pub async fn open(params: &SshParams, target_host: &str, target_port: u16) -> CoreResult<Self> {
        Self::open_with(params, target_host, target_port, CONNECT_TIMEOUT).await
    }

    /// `channel_timeout`:开一个转发通道最多等多久(测试里调短)。
    async fn open_with(
        params: &SshParams,
        target_host: &str,
        target_port: u16,
        channel_timeout: Duration,
    ) -> CoreResult<Self> {
        let handle = connect(params).await?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| CoreError::Ssh(format!("没能在本机开转发端口：{error}")))?;
        let local = listener
            .local_addr()
            .map_err(|error| CoreError::Ssh(format!("没能在本机开转发端口：{error}")))?;
        let shared = Arc::new(Shared {
            params: params.clone(),
            target_host: target_host.to_owned(),
            target_port,
            ssh: Mutex::new(Some(Arc::new(handle))),
            last_failure: StdMutex::new(None),
            closed: AtomicBool::new(false),
            channel_timeout,
        });
        let forwarder = tokio::spawn(forward(listener, shared.clone()));
        log::info!(
            "SSH 隧道已建立：127.0.0.1:{} → {} → {target_host}:{target_port}",
            local.port(),
            params.address()
        );
        Ok(Self {
            local,
            shared,
            forwarder,
        })
    }

    pub fn local_port(&self) -> u16 {
        self.local.port()
    }

    /// 最近一次转发失败的原因(SSH 服务器拒绝转发、连不上目标……)。数据库那边只看到连接被断开,
    /// 驱动拿它换成说得清楚的错误。
    pub fn last_failure(&self) -> Option<String> {
        self.shared.last_failure.lock().ok()?.clone()
    }

    /// 连数据库失败时,有转发失败的记录就报那个,说的是真正的原因。
    pub fn explain(&self, error: CoreError) -> CoreError {
        match self.last_failure() {
            Some(failure) => CoreError::Ssh(failure),
            None => error,
        }
    }

    /// 停止接新连接、断开已有的转发,再礼貌地断开 SSH。
    pub async fn close(&self) {
        // 先立牌子:已经排在锁上的转发看到它就不会再去重新登录。
        self.shared.closed.store(true, Ordering::SeqCst);
        self.forwarder.abort();
        let handle = self.shared.ssh.lock().await.take();
        let Some(handle) = handle else {
            return;
        };
        if let Err(error) = handle
            .disconnect(russh::Disconnect::ByApplication, "", "")
            .await
        {
            log::debug!("断开 SSH 时出错（可以忽略）：{error}");
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // 没走 close() 就被丢掉(比如打开到一半失败):至少别让本机端口和转发留着。
        self.forwarder.abort();
    }
}

impl fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Tunnel(127.0.0.1:{} → {:?})",
            self.local.port(),
            self.shared.params
        )
    }
}

struct Shared {
    params: SshParams,
    target_host: String,
    target_port: u16,
    ssh: Mutex<Option<Arc<client::Handle<Client>>>>,
    last_failure: StdMutex<Option<String>>,
    /// 隧道已经关了:不再重新登录。
    closed: AtomicBool,
    channel_timeout: Duration,
}

impl Shared {
    /// 当前的 SSH 连接;断了就重新登录。
    async fn handle(&self) -> CoreResult<Arc<client::Handle<Client>>> {
        let mut slot = self.ssh.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(CoreError::Ssh("SSH 隧道已经关闭".into()));
        }
        if let Some(handle) = slot.as_ref().filter(|h| !h.is_closed()) {
            return Ok(handle.clone());
        }
        log::info!("SSH 连接已断开，重新登录 {}", self.params.address());
        let handle = Arc::new(connect(&self.params).await?);
        *slot = Some(handle.clone());
        Ok(handle)
    }

    /// 开一个转发通道。SSH 连接半死不活(睡眠唤醒、NAT 超时)时心跳要两分钟才认定它断了,
    /// 这期间开通道会一直等:超时就丢掉这条 SSH 连接,下一条转发重新登录。
    async fn open_channel(&self, peer: SocketAddr) -> CoreResult<russh::Channel<client::Msg>> {
        let handle = self.handle().await?;
        let opening = handle.channel_open_direct_tcpip(
            self.target_host.clone(),
            u32::from(self.target_port),
            peer.ip().to_string(),
            u32::from(peer.port()),
        );
        match tokio::time::timeout(self.channel_timeout, opening).await {
            Ok(opened) => opened.map_err(|error| CoreError::Ssh(self.channel_error(&error))),
            Err(_) => {
                self.forget(&handle).await;
                Err(CoreError::Ssh(format!(
                    "SSH 服务器 {} 没有响应，连接可能已经断了；下一次连接会重新登录",
                    self.params.address()
                )))
            }
        }
    }

    /// 丢掉这条 SSH 连接(别的转发已经换上新的就不动)。
    async fn forget(&self, stale: &Arc<client::Handle<Client>>) {
        let mut slot = self.ssh.lock().await;
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, stale))
        {
            *slot = None;
        }
    }

    fn channel_error(&self, error: &russh::Error) -> String {
        let target = format!("{}:{}", self.target_host, self.target_port);
        let server = self.params.address();
        match error {
            russh::Error::ChannelOpenFailure(
                russh::ChannelOpenFailure::AdministrativelyProhibited,
            ) => format!(
                "SSH 服务器 {server} 不允许端口转发，转发不到 {target}（服务器的 sshd_config 里 AllowTcpForwarding 被关掉了？）"
            ),
            russh::Error::ChannelOpenFailure(russh::ChannelOpenFailure::ConnectFailed) => format!(
                "SSH 服务器 {server} 连不上 {target}。数据库的主机和端口要填从 SSH 服务器那边能访问的地址"
            ),
            other => format!("SSH 服务器 {server} 没能打开到 {target} 的转发：{other}"),
        }
    }

    fn record(&self, failure: String) {
        log::warn!("SSH 转发失败：{failure}");
        if let Ok(mut slot) = self.last_failure.lock() {
            *slot = Some(failure);
        }
    }
}

async fn forward(listener: TcpListener, shared: Arc<Shared>) {
    // 每条转发一个任务,放在 JoinSet 里:隧道关闭(这个任务被 abort)时它们跟着一起结束。
    let mut relays = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((socket, peer)) => {
                    relays.spawn(relay(shared.clone(), socket, peer));
                }
                Err(error) => {
                    log::warn!("SSH 隧道接受本机连接失败：{error}");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
            Some(_) = relays.join_next(), if !relays.is_empty() => {}
        }
    }
}

async fn relay(shared: Arc<Shared>, mut socket: TcpStream, peer: SocketAddr) {
    let channel = match shared.open_channel(peer).await {
        Ok(channel) => channel,
        Err(error) => {
            // 本机这条连接直接关掉,数据库驱动看到的是连接被断开;原因记下来给驱动解释。
            shared.record(error.to_string());
            return;
        }
    };
    if let Err(error) = socket.set_nodelay(true) {
        log::debug!("设置 TCP_NODELAY 失败：{error}");
    }
    let mut stream = channel.into_stream();
    if let Err(error) = tokio::io::copy_bidirectional(&mut socket, &mut stream).await {
        log::debug!("SSH 转发的连接结束：{error}");
    }
}

/// 连上 SSH 服务器、核对主机密钥、登录,整体受连接超时约束。
async fn connect(params: &SshParams) -> CoreResult<client::Handle<Client>> {
    let address = params.address();
    let attempt = async {
        let verdict = Arc::new(StdMutex::new(None));
        let handler = Client {
            host: params.host.clone(),
            port: params.port,
            known_hosts: params.known_hosts.clone(),
            verdict: verdict.clone(),
        };
        let config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE),
            nodelay: true,
            ..client::Config::default()
        });
        let mut handle =
            match client::connect(config, (params.host.as_str(), params.port), handler).await {
                Ok(handle) => handle,
                Err(error) => {
                    let rejected = verdict.lock().ok().and_then(|mut v| v.take());
                    return Err(rejected.unwrap_or_else(|| connect_error(&address, error)));
                }
            };
        auth::authenticate(&mut handle, params).await?;
        Ok(handle)
    };
    tokio::time::timeout(CONNECT_TIMEOUT, attempt)
        .await
        .map_err(|_| {
            CoreError::Ssh(format!(
                "连接 SSH 服务器 {address} 超时，请检查地址、端口和网络"
            ))
        })?
}

fn connect_error(address: &str, error: russh::Error) -> CoreError {
    match error {
        russh::Error::IO(io) => CoreError::Ssh(format!("连不上 SSH 服务器 {address}：{io}")),
        russh::Error::NoCommonAlgo { .. } => CoreError::Ssh(format!(
            "和 SSH 服务器 {address} 没有共同支持的加密算法：{error}"
        )),
        other => CoreError::Ssh(format!("和 SSH 服务器 {address} 握手失败：{other}")),
    }
}

/// 登录之后的 SSH 错误(连接中途断开等)。
fn session_error(error: russh::Error) -> CoreError {
    CoreError::Ssh(format!("SSH 连接出错：{error}"))
}

/// russh 的回调:只核对主机密钥。核对不过时把原因放进 `verdict`,连接本身报的是笼统的 UnknownKey。
struct Client {
    host: String,
    port: u16,
    known_hosts: KnownHosts,
    verdict: Arc<StdMutex<Option<CoreError>>>,
}

impl Client {
    fn judge(&self, key: &russh::keys::PublicKey) -> Option<CoreError> {
        let seen = HostKey::new(&self.host, self.port, key);
        match self.known_hosts.check(&self.host, self.port, key) {
            Ok(HostKeyCheck::Trusted) => None,
            Ok(HostKeyCheck::Unknown) => Some(CoreError::HostKeyUnknown(Box::new(seen))),
            Ok(HostKeyCheck::Changed { line, recorded }) => Some(CoreError::HostKeyChanged(
                self.known_hosts
                    .changed_message(&seen, line, recorded.as_deref()),
            )),
            Err(error) => Some(error),
        }
    }
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // 带证书的主机密钥按证书里的那把公钥核对,不认 CA(和 known_hosts 里没写 @cert-authority 时一样)。
        let key = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };
        let problem = self.judge(&key);
        let trusted = problem.is_none();
        if let (Some(problem), Ok(mut verdict)) = (problem, self.verdict.lock()) {
            *verdict = Some(problem);
        }
        Ok(trusted)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
