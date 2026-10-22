//! SSH 隧道(本地端口转发):登录 SSH 服务器,在本机开一个入口(Unix 上是只有当前用户能连的
//! Unix 套接字,见 [`local`]),每进来一条连接就开一个 direct-tcpip 通道转发到数据库。
//! 驱动只管连这个入口,不知道有隧道。
//!
//! 有跳板机时依次登录:先连第一台,经它的 direct-tcpip 通道连下一台,一直到 SSH 服务器,
//! 每一台的主机密钥都核对(和 OpenSSH 的 ProxyJump 一样)。
//!
//! 隧道归会话所有:会话关闭时先关连接池,再关隧道。SSH 连接断了(网络切换、服务器重启)不用重开会话,
//! 下一条新连接进来时重新登录;主机密钥照样核对。

mod auth;
mod jump;
mod known_hosts;
mod local;
mod ssh_config;

use std::borrow::Cow;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use russh::client;
use russh::keys::{Algorithm, PublicKeyOrCertificate};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Mutex;
use tokio::task::{JoinHandle, JoinSet};

pub use jump::{JumpSpec, parse_jump_hosts};
pub(crate) use known_hosts::valid_host;
pub use known_hosts::{HostKey, HostKeyCheck, KnownHosts, RecordedKey, fingerprint};
pub use local::LocalEndpoint;
use local::{LocalListener, LocalStream};
pub use ssh_config::{HostSettings, SshConfigFile, SshConfigHost};

use crate::config::{SshAuthMethod, SshTunnel};
use crate::driver::CONNECT_TIMEOUT;
use crate::error::{CoreError, CoreResult};

/// 空闲时每隔多久发一次心跳,免得路由器、防火墙把安静的连接掐掉;连着 3 次没回应算断开。
const KEEPALIVE: Duration = Duration::from_secs(30);
/// 本机入口 accept 出错(比如文件句柄用完)后歇一会儿再接,不空转。
const ACCEPT_BACKOFF: Duration = Duration::from_millis(200);
/// 同时经隧道的连接最多几条。连接池、查询标签页、取消用的连接加起来远到不了;
/// 超了多半是有别的程序在乱连,不再为它开通道。
const MAX_FORWARDS: usize = 64;

/// SSH 登录方式和凭据。`Debug` 不输出密码和口令。
#[derive(Clone, PartialEq, Eq)]
pub enum SshAuth {
    /// `None`:没存也没问到密码。
    Password(Option<String>),
    PrivateKey {
        path: String,
        passphrase: Option<String>,
    },
    /// ssh-agent 里的私钥。
    Agent,
}

/// 一台跳板机:地址、用户名已经按 `~/.ssh/config` 补齐。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpHop {
    pub host: String,
    pub port: u16,
    pub username: String,
    /// `~/.ssh/config` 里给它配的私钥;没配时试 OpenSSH 默认的那几个。
    pub identity_files: Vec<String>,
    /// 它的 ssh-agent 套接字;`None` 用 SSH_AUTH_SOCK。
    pub agent: Option<String>,
}

/// 建隧道所需的全部参数。
#[derive(Clone, PartialEq, Eq)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: SshAuth,
    /// 依次经过的跳板机,最后一台再连 SSH 服务器。
    pub jumps: Vec<JumpHop>,
    /// SSH 服务器用的 ssh-agent 套接字;`None` 用 SSH_AUTH_SOCK。
    pub agent: Option<String>,
    pub known_hosts: KnownHosts,
}

impl SshParams {
    /// `secret` 是 SSH 密码或私钥口令。跳板机的别名、ssh-agent 套接字按当前用户的 `~/.ssh/config` 找。
    pub fn from_config(
        tunnel: &SshTunnel,
        secret: Option<String>,
        known_hosts: KnownHosts,
    ) -> Self {
        Self::from_config_with(tunnel, secret, known_hosts, &SshConfigFile::load())
    }

    pub fn from_config_with(
        tunnel: &SshTunnel,
        secret: Option<String>,
        known_hosts: KnownHosts,
        ssh_config: &SshConfigFile,
    ) -> Self {
        let auth = match tunnel.auth {
            SshAuthMethod::Password => SshAuth::Password(secret),
            SshAuthMethod::PrivateKey => SshAuth::PrivateKey {
                path: tunnel.private_key_path.clone().unwrap_or_default(),
                passphrase: secret.filter(|s| !s.is_empty()),
            },
            SshAuthMethod::Agent => SshAuth::Agent,
        };
        // 保存时校验过写法;读到坏的(手改了配置文件)就当没有跳板机,登录失败时会说清楚连的是谁。
        let specs = tunnel
            .jump_hosts
            .as_deref()
            .map(parse_jump_hosts)
            .unwrap_or(Ok(Vec::new()))
            .unwrap_or_else(|error| {
                log::warn!("跳板机写法不对，忽略：{error}");
                Vec::new()
            });
        let jumps = specs
            .into_iter()
            .map(|spec| {
                let settings = ssh_config.lookup(&spec.host);
                JumpHop {
                    host: settings.host_name.unwrap_or(spec.host),
                    port: spec
                        .port
                        .or(settings.port)
                        .unwrap_or(crate::config::DEFAULT_SSH_PORT),
                    // 没写用户名时和 SSH 服务器用同一个:跳板机多半是同一套账号。
                    username: spec
                        .user
                        .or(settings.user)
                        .unwrap_or_else(|| tunnel.username.clone()),
                    identity_files: settings.identity_files,
                    agent: settings.identity_agent,
                }
            })
            .collect();
        Self {
            host: tunnel.host.clone(),
            port: tunnel.port,
            username: tunnel.username.clone(),
            auth,
            jumps,
            agent: ssh_config.lookup(&tunnel.host).identity_agent,
            known_hosts,
        }
    }

    fn address(&self) -> String {
        hop_address(&self.host, self.port)
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
            Self::Agent => write!(f, "Agent"),
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
            .field("jumps", &self.jumps)
            .field("agent", &self.agent)
            .field("known_hosts", &self.known_hosts.path())
            .finish()
    }
}

pub struct Tunnel {
    endpoint: LocalEndpoint,
    /// Unix 套接字所在的临时目录,关隧道时删掉。
    socket_dir: Option<PathBuf>,
    shared: Arc<Shared>,
    forwarder: JoinHandle<()>,
}

impl Tunnel {
    /// 登录 SSH 服务器并开始把本机入口转发到 `target_host:target_port`(从 SSH 服务器那边看的地址)。
    /// `socket_name` 是 Unix 套接字的文件名,由驱动定(PostgreSQL 要 `.s.PGSQL.<端口>`)。
    pub async fn open(
        params: &SshParams,
        target_host: &str,
        target_port: u16,
        socket_name: &str,
    ) -> CoreResult<Self> {
        Self::open_with(
            params,
            target_host,
            target_port,
            socket_name,
            CONNECT_TIMEOUT,
        )
        .await
    }

    /// `channel_timeout`:开一个转发通道最多等多久(测试里调短)。
    async fn open_with(
        params: &SshParams,
        target_host: &str,
        target_port: u16,
        socket_name: &str,
        channel_timeout: Duration,
    ) -> CoreResult<Self> {
        let handle = connect(params).await?;
        let bound = local::bind(socket_name)
            .await
            .map_err(|error| CoreError::Ssh(format!("没能在本机开转发入口：{error}")))?;
        let shared = Arc::new(Shared {
            params: params.clone(),
            target_host: target_host.to_owned(),
            target_port,
            ssh: Mutex::new(Some(Arc::new(handle))),
            last_failure: StdMutex::new(None),
            closed: AtomicBool::new(false),
            channel_timeout,
        });
        let forwarder = tokio::spawn(forward(bound.listener, shared.clone()));
        log::info!(
            "SSH 隧道已建立：{:?} → {} → {target_host}:{target_port}",
            bound.endpoint,
            params.address()
        );
        Ok(Self {
            endpoint: bound.endpoint,
            socket_dir: bound.socket_dir,
            shared,
            forwarder,
        })
    }

    /// 驱动该连的本机入口。
    pub fn endpoint(&self) -> &LocalEndpoint {
        &self.endpoint
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
        if let Some(dir) = &self.socket_dir {
            local::remove_socket_dir(dir);
        }
        let chain = self.shared.ssh.lock().await.take();
        if let Some(chain) = chain {
            chain.disconnect().await;
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        // 没走 close() 就被丢掉(比如打开到一半失败):至少别让本机入口和转发留着。
        self.forwarder.abort();
        if let Some(dir) = &self.socket_dir {
            local::remove_socket_dir(dir);
        }
    }
}

impl fmt::Debug for Tunnel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tunnel({:?} → {:?})", self.endpoint, self.shared.params)
    }
}

struct Shared {
    params: SshParams,
    target_host: String,
    target_port: u16,
    ssh: Mutex<Option<Arc<SshChain>>>,
    last_failure: StdMutex<Option<String>>,
    /// 隧道已经关了:不再重新登录。
    closed: AtomicBool,
    channel_timeout: Duration,
}

impl Shared {
    /// 当前的 SSH 连接;断了(连同哪一台跳板机断了)就重新登录。
    async fn handle(&self) -> CoreResult<Arc<SshChain>> {
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
    async fn open_channel(&self) -> CoreResult<russh::Channel<client::Msg>> {
        let handle = self.handle().await?;
        let opening = handle.server.channel_open_direct_tcpip(
            self.target_host.clone(),
            u32::from(self.target_port),
            // 发起方地址只是给服务器记日志的;本机入口是套接字时没有端口。
            "127.0.0.1".to_owned(),
            0,
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
    async fn forget(&self, stale: &Arc<SshChain>) {
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

async fn forward(listener: LocalListener, shared: Arc<Shared>) {
    // 每条转发一个任务,放在 JoinSet 里:隧道关闭(这个任务被 abort)时它们跟着一起结束。
    let mut relays = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(_) if relays.len() >= MAX_FORWARDS => {
                    shared.record(format!("同时经 SSH 隧道的连接超过 {MAX_FORWARDS} 条，这一条没有转发"));
                }
                Ok(LocalStream::Tcp(socket)) => {
                    relays.spawn(relay(shared.clone(), socket));
                }
                #[cfg(unix)]
                Ok(LocalStream::Unix(socket)) => {
                    relays.spawn(relay(shared.clone(), socket));
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

async fn relay<S: AsyncRead + AsyncWrite + Unpin>(shared: Arc<Shared>, mut socket: S) {
    let channel = match shared.open_channel().await {
        Ok(channel) => channel,
        Err(error) => {
            // 本机这条连接直接关掉,数据库驱动看到的是连接被断开;原因记下来给驱动解释。
            shared.record(error.to_string());
            return;
        }
    };
    let mut stream = channel.into_stream();
    if let Err(error) = tokio::io::copy_bidirectional(&mut socket, &mut stream).await {
        log::debug!("SSH 转发的连接结束：{error}");
    }
}

/// 登录好的一串 SSH 连接:先是各台跳板机,最后是 SSH 服务器。后面的连接走在前一台的通道里,
/// 前面的必须一直留着。
struct SshChain {
    jumps: Vec<client::Handle<Client>>,
    server: client::Handle<Client>,
}

impl SshChain {
    /// 哪一台断了,整串都用不了了。
    fn is_closed(&self) -> bool {
        self.server.is_closed() || self.jumps.iter().any(client::Handle::is_closed)
    }

    /// 从里往外礼貌地断开。
    async fn disconnect(&self) {
        for handle in std::iter::once(&self.server).chain(self.jumps.iter().rev()) {
            if let Err(error) = handle
                .disconnect(russh::Disconnect::ByApplication, "", "")
                .await
            {
                log::debug!("断开 SSH 时出错（可以忽略）：{error}");
            }
        }
    }
}

/// 连上(经跳板机)SSH 服务器、每一台都核对主机密钥并登录,整体受连接超时约束(每多一跳多给一份)。
async fn connect(params: &SshParams) -> CoreResult<SshChain> {
    let address = params.address();
    let attempt = async {
        let mut jumps: Vec<client::Handle<Client>> = Vec::with_capacity(params.jumps.len());
        for jump in &params.jumps {
            let mut handle = open_hop(params, jumps.last(), &jump.host, jump.port).await?;
            auth::authenticate_jump(&mut handle, params, jump).await?;
            jumps.push(handle);
        }
        let mut server = open_hop(params, jumps.last(), &params.host, params.port).await?;
        auth::authenticate(&mut server, params).await?;
        Ok(SshChain { jumps, server })
    };
    let hops = u32::try_from(params.jumps.len())
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    tokio::time::timeout(CONNECT_TIMEOUT * hops, attempt)
        .await
        .map_err(|_| {
            CoreError::Ssh(format!(
                "连接 SSH 服务器 {address} 超时，请检查地址、端口和网络"
            ))
        })?
}

/// 连上一台 SSH 服务器并核对主机密钥:第一台直接连,之后的经前一台的 direct-tcpip 通道连。
async fn open_hop(
    params: &SshParams,
    via: Option<&client::Handle<Client>>,
    host: &str,
    port: u16,
) -> CoreResult<client::Handle<Client>> {
    let address = hop_address(host, port);
    let verdict = Arc::new(StdMutex::new(None));
    let handler = Client {
        host: host.to_owned(),
        port,
        known_hosts: params.known_hosts.clone(),
        verdict: verdict.clone(),
    };
    let mut preferred = russh::Preferred::default();
    preferred.key = Cow::Owned(prefer_recorded(
        &preferred.key,
        &params.known_hosts.recorded_algorithms(host, port),
    ));
    let config = Arc::new(client::Config {
        keepalive_interval: Some(KEEPALIVE),
        nodelay: true,
        preferred,
        ..client::Config::default()
    });
    let connected = match via {
        None => client::connect(config, (host, port), handler).await,
        Some(jump) => {
            let channel = jump
                .channel_open_direct_tcpip(
                    host.to_owned(),
                    u32::from(port),
                    "127.0.0.1".to_owned(),
                    0,
                )
                .await
                .map_err(|error| {
                    CoreError::Ssh(format!("跳板机连不上下一台 SSH 服务器 {address}：{error}"))
                })?;
            client::connect_stream(config, channel.into_stream(), handler).await
        }
    };
    connected.map_err(|error| {
        let rejected = verdict.lock().ok().and_then(|mut v| v.take());
        rejected.unwrap_or_else(|| connect_error(&address, error))
    })
}

fn hop_address(host: &str, port: u16) -> String {
    HostKey {
        host: host.to_owned(),
        port,
        algorithm: String::new(),
        fingerprint: String::new(),
        public_key: String::new(),
    }
    .address()
}

/// 主机密钥算法按记下的类型优先:服务器有好几种主机密钥时(新加了 Ed25519,原来记的是 RSA),
/// 先拿出我们认得的那种,不会因为换了类型被当成「密钥变了」拒绝。
fn prefer_recorded(defaults: &[Algorithm], recorded: &[Algorithm]) -> Vec<Algorithm> {
    let known = |algorithm: &Algorithm| {
        recorded.iter().any(|r| {
            r == algorithm
                || (matches!(r, Algorithm::Rsa { .. })
                    && matches!(algorithm, Algorithm::Rsa { .. }))
        })
    };
    let (first, rest): (Vec<_>, Vec<_>) = defaults.iter().cloned().partition(|a| known(a));
    first.into_iter().chain(rest).collect()
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
            Ok(HostKeyCheck::Unknown) => {
                // 只有服务器真出示过的这一把,用户确认后才能记进去。
                self.known_hosts.expect(&seen);
                Some(CoreError::HostKeyUnknown(Box::new(seen)))
            }
            Ok(HostKeyCheck::Changed { line, recorded }) => Some(CoreError::HostKeyChanged(
                self.known_hosts
                    .changed_message(&seen, line, recorded.as_deref()),
            )),
            Ok(HostKeyCheck::OtherType { recorded }) => Some(CoreError::HostKeyChanged(
                self.known_hosts.other_type_message(&seen, &recorded),
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
