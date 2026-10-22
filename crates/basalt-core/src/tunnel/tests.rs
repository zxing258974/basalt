//! 不靠容器的隧道测试:在测试进程里用 russh 的服务端起一个 SSH 服务器,
//! 再起一个回显服务当「数据库」,经隧道连过去收发数据。真 sshd 的测试在 tests/ssh_tunnel.rs。

use std::net::Ipv4Addr;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use russh::keys::{PrivateKey, PublicKey};
use russh::server::{self, Auth, ChannelOpenHandle, Msg, Session};
use russh::{Channel, ChannelOpenFailure};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

/// 测试里隧道入口(Unix 套接字)的文件名。
const SOCKET: &str = "test.sock";

trait Duplex: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Duplex for T {}

/// 像驱动那样连隧道在本机的入口。
async fn connect_local(tunnel: &Tunnel) -> std::io::Result<Box<dyn Duplex>> {
    Ok(match tunnel.endpoint() {
        #[cfg(unix)]
        LocalEndpoint::Unix(path) => Box::new(tokio::net::UnixStream::connect(path).await?),
        #[cfg(not(unix))]
        LocalEndpoint::Unix(_) => unreachable!("只有 Unix 上才有套接字入口"),
        LocalEndpoint::Tcp(address) => Box::new(TcpStream::connect(address).await?),
    })
}

const PASSWORD: &str = "tunnel-pass";
const PASSPHRASE: &str = "basalt-key-pass";

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/ssh")
        .join(name)
}

fn fixture_path(name: &str) -> String {
    fixture(name).display().to_string()
}

fn public_key(name: &str) -> PublicKey {
    let text = std::fs::read_to_string(fixture(&format!("{name}.pub"))).unwrap();
    PublicKey::from_openssh(text.trim()).unwrap()
}

#[derive(Clone)]
struct ServerRules {
    allow_forwarding: bool,
    /// 只开键盘交互、不收 password 方式(像只开了 PAM 的 sshd)。
    interactive_only: bool,
    /// 键盘交互答对密码以后再要一个动态口令。
    otp_after_password: bool,
    authorized: Vec<PublicKey>,
    /// 前几个转发请求不回话(像半死不活的 SSH 连接)。
    stall_channels: Arc<AtomicUsize>,
    /// 登录成功的次数。
    logins: Arc<AtomicUsize>,
}

struct TestServer {
    rules: ServerRules,
}

impl server::Handler for TestServer {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        if self.rules.interactive_only {
            return Ok(Auth::Reject {
                proceed_with_methods: Some(russh::MethodSet::from(
                    &[russh::MethodKind::KeyboardInteractive][..],
                )),
                partial_success: false,
            });
        }
        Ok(if password == PASSWORD {
            self.rules.logins.fetch_add(1, Ordering::SeqCst);
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<server::Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let Some(response) = response else {
            return Ok(Auth::Partial {
                name: "".into(),
                instructions: "".into(),
                prompts: vec![("Password: ".into(), false)].into(),
            });
        };
        let answers: Vec<_> = response
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .collect();
        if answers == [PASSWORD] && self.rules.otp_after_password {
            return Ok(Auth::Partial {
                name: "".into(),
                instructions: "".into(),
                prompts: vec![("Verification code: ".into(), false)].into(),
            });
        }
        Ok(if answers == [PASSWORD] {
            self.rules.logins.fetch_add(1, Ordering::SeqCst);
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        let known = self
            .rules
            .authorized
            .iter()
            .any(|k| k.key_data() == key.key_data());
        Ok(if known { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _originator: &str,
        _originator_port: u32,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let stalled = self
            .rules
            .stall_channels
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if stalled {
            // 既不接受也不拒绝,就这么挂着(丢掉 reply 会自动拒绝)。
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(30)).await;
                drop(reply);
            });
            return Ok(());
        }
        if !self.rules.allow_forwarding {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        let Ok(mut target) = TcpStream::connect((host, port as u16)).await else {
            reply.reject(ChannelOpenFailure::ConnectFailed).await;
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut target, &mut stream).await;
        });
        Ok(())
    }
}

/// 起一个 SSH 服务器,返回端口和它的主机密钥。
async fn ssh_server(rules: ServerRules) -> (u16, PublicKey) {
    let host_key = russh::keys::load_secret_key(fixture("host_ed25519"), None).unwrap();
    let public = host_key.public_key().clone();
    let config = Arc::new(server::Config {
        keys: vec![host_key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..server::Config::default()
    });
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let handler = TestServer {
                rules: rules.clone(),
            };
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    (port, public)
}

/// 「数据库」:把收到的字节加上前缀回过去。
async fn echo_server() -> u16 {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 256];
                while let Ok(n) = socket.read(&mut buf).await {
                    if n == 0 || socket.write_all(b"echo:").await.is_err() {
                        return;
                    }
                    if socket.write_all(&buf[..n]).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    port
}

fn rules() -> ServerRules {
    ServerRules {
        allow_forwarding: true,
        interactive_only: false,
        otp_after_password: false,
        stall_channels: Arc::new(AtomicUsize::new(0)),
        logins: Arc::new(AtomicUsize::new(0)),
        authorized: vec![
            public_key("id_ed25519"),
            public_key("id_ed25519_passphrase"),
            public_key("id_rsa_pem"),
        ],
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    params: SshParams,
    server_key: PublicKey,
}

/// 已经信任了服务器主机密钥的参数。
async fn trusted(rules: ServerRules, auth: SshAuth) -> Fixture {
    let (port, server_key) = ssh_server(rules).await;
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = KnownHosts::new(dir.path().join("known_hosts"));
    known_hosts.learn("127.0.0.1", port, &server_key).unwrap();
    Fixture {
        _dir: dir,
        params: SshParams {
            host: "127.0.0.1".into(),
            port,
            username: "basalt".into(),
            auth,
            known_hosts,
        },
        server_key,
    }
}

async fn round_trip(tunnel: &Tunnel, message: &[u8]) -> Vec<u8> {
    let mut socket = connect_local(tunnel).await.unwrap();
    socket.write_all(message).await.unwrap();
    let mut reply = vec![0u8; message.len() + 5];
    tokio::time::timeout(Duration::from_secs(5), socket.read_exact(&mut reply))
        .await
        .expect("经隧道读回复超时")
        .unwrap();
    reply
}

#[tokio::test]
async fn forwards_through_password_login() {
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let target = echo_server().await;
    let tunnel = Tunnel::open(&fixture.params, "127.0.0.1", target, SOCKET)
        .await
        .unwrap();
    assert_eq!(round_trip(&tunnel, b"hello").await, b"echo:hello");
    // 同一个隧道可以同时转发好几条连接(连接池)。
    let (a, b) = tokio::join!(round_trip(&tunnel, b"one"), round_trip(&tunnel, b"two"));
    assert_eq!(
        (a.as_slice(), b.as_slice()),
        (&b"echo:one"[..], &b"echo:two"[..])
    );
    assert!(tunnel.last_failure().is_none());
    assert!(format!("{tunnel:?}").contains("Tunnel("));

    tunnel.close().await;
    assert!(
        connect_local(&tunnel).await.is_err(),
        "关掉以后本机入口不再监听"
    );
}

#[tokio::test]
async fn private_keys_with_and_without_passphrase_log_in() {
    let target = echo_server().await;
    for (key, passphrase) in [
        ("id_ed25519", None),
        ("id_ed25519_passphrase", Some(PASSPHRASE)),
        ("id_rsa_pem", None),
    ] {
        let auth = SshAuth::PrivateKey {
            path: fixture_path(key),
            passphrase: passphrase.map(str::to_owned),
        };
        let fixture = trusted(rules(), auth).await;
        let tunnel = Tunnel::open(&fixture.params, "127.0.0.1", target, SOCKET)
            .await
            .unwrap_or_else(|e| panic!("{key}：{e}"));
        assert_eq!(round_trip(&tunnel, b"k").await, b"echo:k", "{key}");
        tunnel.close().await;
    }
}

#[tokio::test]
async fn unknown_host_key_is_handed_back_for_confirmation() {
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let fresh = tempfile::tempdir().unwrap();
    let params = SshParams {
        known_hosts: KnownHosts::new(fresh.path().join("known_hosts")),
        ..fixture.params.clone()
    };
    let error = Tunnel::open(&params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    let host_key = error.host_key().cloned().expect("应该带着主机密钥");
    assert_eq!(error.code(), "host_key_unknown");
    assert_eq!(host_key.fingerprint, fingerprint(&fixture.server_key));
    assert!(error.to_string().contains(&host_key.fingerprint));

    // 用户确认之后就能连上。
    params.known_hosts.trust(&host_key).unwrap();
    let target = echo_server().await;
    let tunnel = Tunnel::open(&params, "127.0.0.1", target, SOCKET)
        .await
        .unwrap();
    tunnel.close().await;
}

#[tokio::test]
async fn changed_host_key_is_refused() {
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = KnownHosts::new(dir.path().join("known_hosts"));
    // 记着的是另一把钥匙(客户端那把,冒充「上次的主机密钥」)。
    known_hosts
        .learn("127.0.0.1", fixture.params.port, &public_key("id_ed25519"))
        .unwrap();
    let params = SshParams {
        known_hosts,
        ..fixture.params
    };
    let error = Tunnel::open(&params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "host_key_changed");
    assert!(error.to_string().contains("已拒绝连接"), "{error}");
}

#[tokio::test]
async fn login_failures_say_which_secret_is_wrong() {
    let wrong = trusted(rules(), SshAuth::Password(Some("nope".into()))).await;
    let error = Tunnel::open(&wrong.params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ssh_authentication");

    let missing = trusted(rules(), SshAuth::Password(None)).await;
    let error = Tunnel::open(&missing.params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ssh_secret_required");

    let encrypted = |passphrase: Option<&str>| SshAuth::PrivateKey {
        path: fixture_path("id_ed25519_passphrase"),
        passphrase: passphrase.map(str::to_owned),
    };
    let no_passphrase = trusted(rules(), encrypted(None)).await;
    let error = Tunnel::open(&no_passphrase.params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ssh_secret_required");
    let bad_passphrase = trusted(rules(), encrypted(Some("wrong"))).await;
    let error = Tunnel::open(&bad_passphrase.params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ssh_authentication");
    assert!(error.to_string().contains("口令不正确"), "{error}");

    let not_authorized = trusted(
        ServerRules {
            authorized: vec![],
            ..rules()
        },
        SshAuth::PrivateKey {
            path: fixture_path("id_ed25519"),
            passphrase: None,
        },
    )
    .await;
    let error = Tunnel::open(&not_authorized.params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ssh_authentication");
}

#[tokio::test]
async fn keyboard_interactive_servers_get_the_password() {
    let interactive = ServerRules {
        interactive_only: true,
        ..rules()
    };
    let fixture = trusted(
        interactive.clone(),
        SshAuth::Password(Some(PASSWORD.into())),
    )
    .await;
    let target = echo_server().await;
    let tunnel = Tunnel::open(&fixture.params, "127.0.0.1", target, SOCKET)
        .await
        .unwrap();
    assert_eq!(round_trip(&tunnel, b"pam").await, b"echo:pam");
    tunnel.close().await;

    let wrong = trusted(interactive, SshAuth::Password(Some("nope".into()))).await;
    let error = Tunnel::open(&wrong.params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ssh_authentication");
}

#[tokio::test]
async fn refused_forwarding_is_explained() {
    let prohibited = trusted(
        ServerRules {
            allow_forwarding: false,
            ..rules()
        },
        SshAuth::Password(Some(PASSWORD.into())),
    )
    .await;
    let tunnel = Tunnel::open(&prohibited.params, "db.internal", 3306, SOCKET)
        .await
        .unwrap();
    let mut socket = connect_local(&tunnel).await.unwrap();
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
        .await
        .unwrap();
    assert!(matches!(read, Ok(0) | Err(_)), "转发不了时本机连接直接断开");
    let failure = tunnel.last_failure().expect("记下了原因");
    assert!(failure.contains("AllowTcpForwarding"), "{failure}");
    let explained = tunnel.explain(CoreError::Network("connection reset".into()));
    assert_eq!(explained.code(), "ssh");
    tunnel.close().await;

    // 目标在 SSH 服务器那边连不上。
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let closed_port = {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let tunnel = Tunnel::open(&fixture.params, "127.0.0.1", closed_port, SOCKET)
        .await
        .unwrap();
    let mut socket = connect_local(&tunnel).await.unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
        .await
        .unwrap();
    let failure = tunnel.last_failure().expect("记下了原因");
    assert!(failure.contains("连不上"), "{failure}");
    tunnel.close().await;
}

#[tokio::test]
async fn reconnects_when_the_ssh_connection_drops() {
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let target = echo_server().await;
    let tunnel = Tunnel::open(&fixture.params, "127.0.0.1", target, SOCKET)
        .await
        .unwrap();
    // 模拟断线:SSH 连接没了,隧道本身还在。
    let old = tunnel.shared.ssh.lock().await.take().unwrap();
    old.disconnect(russh::Disconnect::ByApplication, "", "")
        .await
        .unwrap();
    *tunnel.shared.ssh.lock().await = Some(old);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(round_trip(&tunnel, b"again").await, b"echo:again");
    tunnel.close().await;
}

#[tokio::test]
async fn unreachable_ssh_server_is_a_plain_ssh_error() {
    let closed_port = {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let dir = tempfile::tempdir().unwrap();
    let params = SshParams {
        host: "127.0.0.1".into(),
        port: closed_port,
        username: "u".into(),
        auth: SshAuth::Password(Some("p".into())),
        known_hosts: KnownHosts::new(dir.path().join("known_hosts")),
    };
    let error = Tunnel::open(&params, "db", 3306, SOCKET).await.unwrap_err();
    assert_eq!(error.code(), "ssh");
    assert!(error.to_string().contains("连不上 SSH 服务器"), "{error}");
}

#[test]
fn params_from_config_pick_the_secret_for_the_login_method() {
    let known_hosts = KnownHosts::new("/tmp/known_hosts");
    let tunnel = SshTunnel {
        host: "jump".into(),
        port: 22,
        username: "deploy".into(),
        auth: SshAuthMethod::Password,
        private_key_path: None,
        save_secret: true,
    };
    let params = SshParams::from_config(&tunnel, Some("pw".into()), known_hosts.clone());
    assert_eq!(params.auth, SshAuth::Password(Some("pw".into())));
    let debug = format!("{params:?}");
    assert!(!debug.contains("pw\""), "{debug}");
    assert!(debug.contains("***"));

    let key = SshTunnel {
        auth: SshAuthMethod::PrivateKey,
        private_key_path: Some("/k".into()),
        ..tunnel
    };
    let params = SshParams::from_config(&key, Some(String::new()), known_hosts);
    assert_eq!(
        params.auth,
        SshAuth::PrivateKey {
            path: "/k".into(),
            passphrase: None
        },
        "空口令当作没有口令"
    );
    assert!(format!("{params:?}").contains("passphrase: None"));
}

#[tokio::test]
async fn unreadable_or_foreign_key_files_are_reported() {
    let missing = auth::load_private_key("/nonexistent/basalt-key", None)
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "file");

    let dir = tempfile::tempdir().unwrap();
    let ppk = dir.path().join("key.ppk");
    std::fs::write(&ppk, "PuTTY-User-Key-File-3: ssh-ed25519\n").unwrap();
    let error = auth::load_private_key(&ppk.display().to_string(), None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("PuTTY"), "{error}");

    let garbage = dir.path().join("garbage");
    std::fs::write(&garbage, "not a key").unwrap();
    let error = auth::load_private_key(&garbage.display().to_string(), Some("x"))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "unsupported", "{error}");
    let _: PrivateKey = auth::load_private_key(&fixture_path("id_ed25519"), Some(""))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_stalled_ssh_connection_is_dropped_and_the_next_one_logs_in_again() {
    let rules = ServerRules {
        stall_channels: Arc::new(AtomicUsize::new(1)),
        ..rules()
    };
    let logins = rules.logins.clone();
    let fixture = trusted(rules, SshAuth::Password(Some(PASSWORD.into()))).await;
    let target = echo_server().await;
    let tunnel = Tunnel::open_with(
        &fixture.params,
        "127.0.0.1",
        target,
        SOCKET,
        Duration::from_millis(300),
    )
    .await
    .unwrap();
    assert_eq!(logins.load(Ordering::SeqCst), 1);

    // 第一条转发没人理:超时后本机连接断开,记下原因。
    let mut socket = connect_local(&tunnel).await.unwrap();
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
        .await
        .unwrap();
    assert!(matches!(read, Ok(0) | Err(_)));
    assert!(tunnel.last_failure().unwrap().contains("没有响应"));

    // 下一条重新登录,照常转发。
    assert_eq!(round_trip(&tunnel, b"fresh").await, b"echo:fresh");
    assert_eq!(logins.load(Ordering::SeqCst), 2);
    tunnel.close().await;
}

#[tokio::test]
async fn a_closed_tunnel_never_logs_in_again() {
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let tunnel = Tunnel::open(&fixture.params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap();
    tunnel.close().await;
    let Err(error) = tunnel.shared.handle().await else {
        panic!("关了的隧道不该再登录");
    };
    assert_eq!(error.code(), "ssh");
}

#[tokio::test]
async fn the_password_is_not_offered_as_a_one_time_code() {
    let rules = ServerRules {
        interactive_only: true,
        otp_after_password: true,
        ..rules()
    };
    let fixture = trusted(rules, SshAuth::Password(Some(PASSWORD.into()))).await;
    let error = Tunnel::open(&fixture.params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "ssh_authentication");
    assert!(error.to_string().contains("动态口令"), "{error}");
}

#[cfg(unix)]
#[tokio::test]
async fn the_local_entry_is_a_socket_only_this_user_can_reach() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let target = echo_server().await;
    let tunnel = Tunnel::open(&fixture.params, "127.0.0.1", target, ".s.PGSQL.5432")
        .await
        .unwrap();
    let LocalEndpoint::Unix(path) = tunnel.endpoint().clone() else {
        panic!("Unix 上应该是套接字：{:?}", tunnel.endpoint());
    };
    assert!(path.ends_with(".s.PGSQL.5432"));
    let dir = path.parent().unwrap().to_path_buf();
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700, "只有当前用户能进");
    assert_eq!(round_trip(&tunnel, b"sock").await, b"echo:sock");

    tunnel.close().await;
    assert!(!dir.exists(), "关隧道时连同临时目录一起删掉");
}

#[tokio::test]
async fn too_many_forwards_at_once_are_turned_away() {
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let target = echo_server().await;
    let tunnel = Tunnel::open(&fixture.params, "127.0.0.1", target, SOCKET)
        .await
        .unwrap();
    // 先占满:每条都收发一次,确认转发已经开好。
    let mut held = Vec::new();
    for i in 0..MAX_FORWARDS {
        let mut socket = connect_local(&tunnel).await.unwrap();
        socket.write_all(b"x").await.unwrap();
        let mut reply = [0u8; 6];
        socket.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"echo:x", "第 {i} 条");
        held.push(socket);
    }
    let mut extra = connect_local(&tunnel).await.unwrap();
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(5), extra.read(&mut buf))
        .await
        .unwrap();
    assert!(matches!(read, Ok(0) | Err(_)), "多出来的一条直接断开");
    assert!(
        tunnel
            .last_failure()
            .unwrap()
            .contains(&MAX_FORWARDS.to_string())
    );

    // 关掉一条就又能连了。
    drop(held.pop());
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(round_trip(&tunnel, b"again").await, b"echo:again");
    tunnel.close().await;
}

#[tokio::test]
async fn a_key_of_another_type_than_recorded_is_refused() {
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let dir = tempfile::tempdir().unwrap();
    let known_hosts = KnownHosts::new(dir.path().join("known_hosts"));
    // 记下的是 RSA,服务器(或者冒充它的人)拿出来的是 Ed25519。
    known_hosts
        .learn("127.0.0.1", fixture.params.port, &public_key("id_rsa_pem"))
        .unwrap();
    let params = SshParams {
        known_hosts,
        ..fixture.params
    };
    let error = Tunnel::open(&params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    assert_eq!(error.code(), "host_key_changed", "{error}");
    assert!(error.to_string().contains("RSA"), "{error}");
    assert!(error.host_key().is_none(), "不弹「第一次连接」的确认框");
}

#[tokio::test]
async fn only_the_presented_key_can_be_trusted_after_a_first_visit() {
    let fixture = trusted(rules(), SshAuth::Password(Some(PASSWORD.into()))).await;
    let fresh = tempfile::tempdir().unwrap();
    let params = SshParams {
        known_hosts: KnownHosts::new(fresh.path().join("known_hosts")),
        ..fixture.params.clone()
    };
    let error = Tunnel::open(&params, "127.0.0.1", 1, SOCKET)
        .await
        .unwrap_err();
    let presented = error.host_key().cloned().unwrap();
    // 网页里伪造一把别的钥匙交回来:拒绝。
    let forged = HostKey::new("127.0.0.1", params.port, &public_key("id_ed25519"));
    assert!(params.known_hosts.trust(&forged).is_err());
    params.known_hosts.trust(&presented).unwrap();
}

#[test]
fn recorded_key_types_go_first_in_the_offer() {
    let defaults = russh::Preferred::default().key.into_owned();
    let rsa_first = prefer_recorded(&defaults, &[Algorithm::Rsa { hash: None }]);
    assert!(matches!(rsa_first[0], Algorithm::Rsa { .. }));
    assert!(matches!(rsa_first[1], Algorithm::Rsa { .. }));
    assert_eq!(rsa_first.len(), defaults.len());
    assert_eq!(prefer_recorded(&defaults, &[]), defaults);
}
