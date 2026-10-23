//! 怎么连到 Redis:直连,或者经 SSH 隧道在本机的入口(Unix 套接字或本机端口);要 TLS 就在这条流上握手。
//! 记下这些,之后按库号、按命令行标签页再开新连接时照样连。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use redis::aio::MultiplexedConnection;
use redis::{AsyncConnectionConfig, RedisConnectionInfo};
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use super::tls;
use crate::driver::{CONNECT_TIMEOUT, ConnectParams, Route};
use crate::error::{CoreError, CoreResult, tls_hint};

/// redis 的连接只要求能读能写。
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

#[derive(Debug)]
enum Target {
    Tcp {
        host: String,
        port: u16,
    },
    #[cfg_attr(not(unix), allow(dead_code))]
    Unix(PathBuf),
}

struct Tls {
    config: Arc<ClientConfig>,
    /// 连接配置里的主机名:经隧道时实际连的是本机入口,证书照样按它核对。
    server_name: ServerName<'static>,
}

/// 一条打开的连接。`driver` 负责收发,连接不用了要停掉它。
pub(crate) struct Live {
    pub conn: MultiplexedConnection,
    pub driver: JoinHandle<()>,
}

pub(super) struct Link {
    target: Target,
    tls: Option<Tls>,
    /// 用户名、密码;库号在每次连接时填。
    auth: RedisConnectionInfo,
}

impl Link {
    /// `params` 是连接配置里的原参数,`route` 是实际要连的地方(经隧道时已经换成本机入口)。
    pub(super) fn new(params: &ConnectParams, route: &Route) -> CoreResult<Self> {
        let target = match &route.socket {
            Some(path) => Target::Unix(path.clone()),
            None => Target::Tcp {
                host: route.params.host.clone(),
                port: route.params.port,
            },
        };
        let tls = match tls::client_config(&params.ssl)? {
            Some(config) => Some(Tls {
                config,
                server_name: ServerName::try_from(params.host.trim().to_owned()).map_err(|_| {
                    CoreError::Network(format!("主机名「{}」不能用来核对证书", params.host))
                })?,
            }),
            None => None,
        };
        let mut auth =
            RedisConnectionInfo::default().set_lib_name("basalt", env!("CARGO_PKG_VERSION"));
        if !params.username.trim().is_empty() {
            auth = auth.set_username(params.username.trim());
        }
        if let Some(password) = params.password.as_deref().filter(|p| !p.is_empty()) {
            auth = auth.set_password(password);
        }
        Ok(Self { target, tls, auth })
    }

    /// 连上库 `db`:认证、`SELECT` 都在这里做完。`response_timeout` 为 `None` 时一条命令可以一直等
    /// (命令行里的 BLPOP 这类),要停只能断开连接。
    pub(super) async fn connect(
        &self,
        db: u32,
        response_timeout: Option<Duration>,
    ) -> CoreResult<Live> {
        let stream = self.stream().await?;
        let info = self.auth.clone().set_db(i64::from(db));
        let config = AsyncConnectionConfig::new().set_response_timeout(response_timeout);
        let (conn, driver) = timeout(
            CONNECT_TIMEOUT,
            MultiplexedConnection::new_with_config(&info, stream, config),
        )
        .await
        .map_err(|_| CoreError::Timeout)??;
        Ok(Live {
            conn,
            driver: tokio::spawn(driver),
        })
    }

    async fn stream(&self) -> CoreResult<Box<dyn Io>> {
        let plain = self.plain().await?;
        let Some(tls) = &self.tls else {
            return Ok(plain);
        };
        let connector = TlsConnector::from(tls.config.clone());
        match timeout(
            CONNECT_TIMEOUT,
            connector.connect(tls.server_name.clone(), plain),
        )
        .await
        {
            Ok(Ok(stream)) => Ok(Box::new(stream)),
            Ok(Err(error)) => Err(CoreError::Network(format!(
                "TLS 握手失败：{}",
                tls_hint(&error.to_string())
            ))),
            // 对方不说 TLS 时常常一直不回话,等到超时。
            Err(_) => Err(CoreError::Network(
                "TLS 握手超时：对方可能没开 TLS，或者 TLS 在另一个端口".into(),
            )),
        }
    }

    async fn plain(&self) -> CoreResult<Box<dyn Io>> {
        let network = |e: std::io::Error| CoreError::Network(e.to_string());
        match &self.target {
            Target::Tcp { host, port } => {
                let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), *port)))
                    .await
                    .map_err(|_| CoreError::Timeout)?
                    .map_err(network)?;
                stream.set_nodelay(true).map_err(network)?;
                Ok(Box::new(stream))
            }
            #[cfg(unix)]
            Target::Unix(path) => Ok(Box::new(
                tokio::net::UnixStream::connect(path)
                    .await
                    .map_err(network)?,
            )),
            #[cfg(not(unix))]
            Target::Unix(_) => Err(CoreError::Network("这个系统不支持 Unix 套接字".into())),
        }
    }
}
