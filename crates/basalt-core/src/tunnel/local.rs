//! 隧道在本机的入口。
//!
//! Unix(macOS、Linux)上是一个 Unix 套接字,放在新建的、只有当前用户能进的(0700)临时目录里:
//! 别的用户的进程连不上,网页也碰不到。127.0.0.1 的端口则是本机谁都能连,会话开着的时候
//! 别的程序可以借跳板机的网络位置摸到数据库。sqlx 在 Unix 套接字上照样协商 TLS,证书校验用的主机名
//! 还是数据库的地址,所以 SSL 设置不受影响。
//!
//! 别的平台(Windows)没有这样的套接字,退回 127.0.0.1 的随机端口。

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use tokio::net::{TcpListener, TcpStream};

/// 套接字路径的上限(macOS 是 104 字节,含结尾的 0),超了就退回 TCP。
#[cfg(unix)]
const MAX_SOCKET_PATH: usize = 100;

/// 驱动该连哪儿。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalEndpoint {
    /// 套接字文件的完整路径。
    Unix(PathBuf),
    Tcp(SocketAddr),
}

pub(super) enum LocalListener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(tokio::net::UnixListener),
}

pub(super) enum LocalStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
}

/// 监听好的入口和它用的临时目录(Unix 套接字才有,关隧道时删掉)。
pub(super) struct Bound {
    pub listener: LocalListener,
    pub endpoint: LocalEndpoint,
    pub socket_dir: Option<PathBuf>,
}

/// `socket_name`:套接字的文件名(PostgreSQL 要求 `.s.PGSQL.<端口>`)。
pub(super) async fn bind(socket_name: &str) -> io::Result<Bound> {
    #[cfg(unix)]
    if let Some(bound) = bind_unix(socket_name)? {
        return Ok(bound);
    }
    let _ = socket_name;
    bind_tcp().await
}

async fn bind_tcp() -> io::Result<Bound> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let endpoint = LocalEndpoint::Tcp(listener.local_addr()?);
    Ok(Bound {
        listener: LocalListener::Tcp(listener),
        endpoint,
        socket_dir: None,
    })
}

/// 路径太长时返回 `None`,由调用方退回 TCP。
#[cfg(unix)]
fn bind_unix(socket_name: &str) -> io::Result<Option<Bound>> {
    use std::os::unix::fs::DirBuilderExt;

    let id = uuid::Uuid::new_v4().simple().to_string();
    let dir = std::env::temp_dir().join(format!("basalt-{}", &id[..12]));
    let path = dir.join(socket_name);
    if path.as_os_str().len() > MAX_SOCKET_PATH {
        return Ok(None);
    }
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    match tokio::net::UnixListener::bind(&path) {
        Ok(listener) => Ok(Some(Bound {
            listener: LocalListener::Unix(listener),
            endpoint: LocalEndpoint::Unix(path),
            socket_dir: Some(dir),
        })),
        Err(error) => {
            remove_socket_dir(&dir);
            Err(error)
        }
    }
}

impl LocalListener {
    pub(super) async fn accept(&self) -> io::Result<LocalStream> {
        match self {
            Self::Tcp(listener) => {
                let (socket, _) = listener.accept().await?;
                if let Err(error) = socket.set_nodelay(true) {
                    log::debug!("设置 TCP_NODELAY 失败：{error}");
                }
                Ok(LocalStream::Tcp(socket))
            }
            #[cfg(unix)]
            Self::Unix(listener) => Ok(LocalStream::Unix(listener.accept().await?.0)),
        }
    }
}

/// 关隧道时连同套接字一起删掉。
pub(super) fn remove_socket_dir(dir: &Path) {
    if let Err(error) = std::fs::remove_dir_all(dir)
        && error.kind() != io::ErrorKind::NotFound
    {
        log::warn!("删不掉 SSH 隧道的临时目录 {}：{error}", dir.display());
    }
}
