use super::*;

const KEY_A: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINNMJeJ83kLKs6Vx+1LqlWrb+X2iS6OyXmpHnI3zDrbx";
const KEY_A_FINGERPRINT: &str = "SHA256:yJPhXHcJ/CHD/GzQdaieqdFuFbR4bHR7dhJZxohZ/0k";
const KEY_B: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOJ30z4hXFTt9ALnaS2kzQ30YRbDbqXAeNUdfnxFDO3D";
const KEY_C: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBPTrw8DGHhKHYSM2Vvouhq1i/nrRfocjrcOa/JfFDMxFtG2hB/oDf92hITICWkBXs4ooI5TuSdLSe9FB+1r2q5c=";

fn key(text: &str) -> PublicKey {
    PublicKey::from_openssh(text).unwrap()
}

fn store() -> (tempfile::TempDir, KnownHosts) {
    let dir = tempfile::tempdir().unwrap();
    let hosts = KnownHosts::new(dir.path().join("nested").join("known_hosts"));
    (dir, hosts)
}

/// 服务器出示过、等着确认的那一把。
fn presented(hosts: &KnownHosts, host: &str, port: u16, text: &str) -> HostKey {
    let seen = HostKey::new(host, port, &key(text));
    hosts.expect(&seen);
    seen
}

#[test]
fn host_key_describes_the_server_key() {
    let host_key = HostKey::new(" Jump.Example.Test ", 2222, &key(KEY_A));
    assert_eq!(host_key.host, "jump.example.test");
    assert_eq!(host_key.algorithm, "ssh-ed25519");
    assert_eq!(host_key.fingerprint, KEY_A_FINGERPRINT);
    assert!(host_key.public_key.starts_with("ssh-ed25519 AAAAC3"));
    assert_eq!(host_key.address(), "jump.example.test:2222");
    assert_eq!(HostKey::new("::1", 22, &key(KEY_A)).address(), "[::1]:22");
    let json = serde_json::to_value(&host_key).unwrap();
    assert_eq!(json["publicKey"], host_key.public_key);
}

#[test]
fn unknown_until_trusted_then_trusted() {
    let (_dir, hosts) = store();
    assert_eq!(
        hosts.check("jump", 22, &key(KEY_A)).unwrap(),
        HostKeyCheck::Unknown,
        "文件还不存在也算没见过"
    );
    let seen = presented(&hosts, "jump", 22, KEY_A);
    hosts.trust(&seen).unwrap();
    assert_eq!(
        hosts.check("JUMP", 22, &key(KEY_A)).unwrap(),
        HostKeyCheck::Trusted,
        "主机名不分大小写"
    );
    // 同一台机器的另一个端口是另一台服务器。
    assert_eq!(
        hosts.check("jump", 2222, &key(KEY_A)).unwrap(),
        HostKeyCheck::Unknown
    );
    let text = std::fs::read_to_string(hosts.path()).unwrap();
    assert_eq!(text, format!("jump {KEY_A}\n"));
    // 已经记着的再确认一次(同一台服务器同时开了两个连接)算成功,不重复写。
    hosts.trust(&seen).unwrap();
    assert_eq!(std::fs::read_to_string(hosts.path()).unwrap(), text);
}

#[test]
fn non_default_port_is_written_in_brackets() {
    let (_dir, hosts) = store();
    hosts.learn("10.0.0.9", 2222, &key(KEY_A)).unwrap();
    let text = std::fs::read_to_string(hosts.path()).unwrap();
    assert!(text.starts_with("[10.0.0.9]:2222 ssh-ed25519 "), "{text}");
    assert_eq!(
        hosts.check("10.0.0.9", 2222, &key(KEY_A)).unwrap(),
        HostKeyCheck::Trusted
    );
}

#[test]
fn only_the_key_a_server_presented_can_be_trusted() {
    let (_dir, hosts) = store();
    let made_up = HostKey::new("jump", 22, &key(KEY_A));
    let error = hosts.trust(&made_up).unwrap_err();
    assert!(
        error.to_string().contains("不是刚才服务器出示的"),
        "{error}"
    );

    // 服务器出示的是 B,交回来的却是 A。
    presented(&hosts, "jump", 22, KEY_B);
    assert!(hosts.trust(&made_up).is_err());

    // 主机名动了手脚(一行里多写一台主机)也不行。
    let seen = presented(&hosts, "jump", 22, KEY_A);
    let widened = HostKey {
        host: "jump,prod.example.com".into(),
        ..seen.clone()
    };
    let error = hosts.trust(&widened).unwrap_err();
    assert!(error.to_string().contains("不能用的字符"), "{error}");
    assert!(!hosts.path().exists(), "什么都没记");

    // 原样交回来的才记下,而且只能用一次。
    hosts.trust(&seen).unwrap();
    std::fs::write(hosts.path(), "").unwrap();
    assert!(hosts.trust(&seen).is_err(), "等确认的那一把已经取走了");
}

#[test]
fn pending_keys_are_bounded_and_replaced_per_server() {
    let (_dir, hosts) = store();
    presented(&hosts, "jump", 22, KEY_A);
    let newer = presented(&hosts, "jump", 22, KEY_B);
    assert!(hosts.trust(&HostKey::new("jump", 22, &key(KEY_A))).is_err());
    for port in 0..MAX_PENDING as u16 {
        presented(&hosts, "other", 1000 + port, KEY_C);
    }
    assert!(hosts.trust(&newer).is_err(), "最早的一把已经被挤掉了");
}

#[test]
fn changed_key_is_reported_with_both_fingerprints() {
    let (_dir, hosts) = store();
    hosts.learn("jump", 22, &key(KEY_A)).unwrap();
    let check = hosts.check("jump", 22, &key(KEY_B)).unwrap();
    let HostKeyCheck::Changed { line, recorded } = check else {
        panic!("应该是密钥变了：{check:?}");
    };
    assert_eq!(recorded.as_deref(), Some(KEY_A_FINGERPRINT));

    let seen = HostKey::new("jump", 22, &key(KEY_B));
    let message = hosts.changed_message(&seen, line, recorded.as_deref());
    assert!(message.contains(KEY_A_FINGERPRINT), "{message}");
    assert!(message.contains(&seen.fingerprint), "{message}");
    assert!(message.contains(&hosts.path().display().to_string()));
    assert!(message.contains(&format!("第 {line} 行")));
}

#[test]
fn a_recorded_server_cannot_be_overwritten() {
    let (_dir, hosts) = store();
    hosts.learn("jump", 22, &key(KEY_A)).unwrap();
    let seen = presented(&hosts, "jump", 22, KEY_B);
    let error = hosts.trust(&seen).unwrap_err();
    assert_eq!(error.code(), "host_key_changed");
    assert_eq!(
        hosts.check("jump", 22, &key(KEY_A)).unwrap(),
        HostKeyCheck::Trusted,
        "原来的记录不动"
    );
}

#[test]
fn another_key_type_counts_as_a_change_not_a_first_visit() {
    let (_dir, hosts) = store();
    hosts.learn("jump", 22, &key(KEY_A)).unwrap();
    // 记的是 Ed25519,这次拿出来的是 ECDSA:可能是中间人只拿出了服务器没有的那种。
    let check = hosts.check("jump", 22, &key(KEY_C)).unwrap();
    let HostKeyCheck::OtherType { recorded } = check else {
        panic!("应该是换了类型：{check:?}");
    };
    assert_eq!(
        recorded,
        [RecordedKey {
            line: 1,
            algorithm: "ssh-ed25519".into(),
            fingerprint: KEY_A_FINGERPRINT.into(),
        }]
    );
    let seen = HostKey::new("jump", 22, &key(KEY_C));
    let message = hosts.other_type_message(&seen, &recorded);
    for needle in [
        "ECDSA",
        "ED25519",
        KEY_A_FINGERPRINT,
        &seen.fingerprint,
        "已拒绝连接",
    ] {
        assert!(message.contains(needle), "缺「{needle}」：{message}");
    }
    // 也不能经确认流程补记进去。
    hosts.expect(&seen);
    assert_eq!(hosts.trust(&seen).unwrap_err().code(), "host_key_changed");
    assert_eq!(
        hosts.recorded_algorithms("JUMP", 22),
        [russh::keys::Algorithm::Ed25519]
    );
}

#[test]
fn trust_requires_the_fingerprint_the_user_saw() {
    let (_dir, hosts) = store();
    let seen = presented(&hosts, "jump", 22, KEY_A);
    let tampered = HostKey {
        public_key: KEY_B.into(),
        ..seen.clone()
    };
    let error = hosts.trust(&tampered).unwrap_err();
    assert!(error.to_string().contains("指纹对不上"), "{error}");
    let garbage = HostKey {
        public_key: "not a key".into(),
        ..seen
    };
    assert!(hosts.trust(&garbage).is_err());
    assert!(!hosts.path().exists(), "什么都没记");
}

#[test]
fn a_damaged_file_is_refused_not_read_as_empty() {
    let (_dir, hosts) = store();
    std::fs::create_dir_all(hosts.path().parent().unwrap()).unwrap();
    // 写到一半的行:主机名和算法都在,公钥是半截。
    std::fs::write(hosts.path(), "jump ssh-ed25519 AAAAC3Nza").unwrap();
    let error = hosts.check("jump", 22, &key(KEY_A)).unwrap_err();
    assert_eq!(error.code(), "storage");
    assert!(error.to_string().contains("拒绝连接"), "{error}");
}

#[cfg(unix)]
#[test]
fn an_unreadable_file_is_refused_not_read_as_empty() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, hosts) = store();
    hosts.learn("jump", 22, &key(KEY_A)).unwrap();
    std::fs::set_permissions(hosts.path(), std::fs::Permissions::from_mode(0o000)).unwrap();
    let result = hosts.check("elsewhere", 22, &key(KEY_B));
    std::fs::set_permissions(hosts.path(), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(result.unwrap_err().code(), "storage");
}

#[test]
fn writes_replace_the_file_whole_and_keep_earlier_lines() {
    let (_dir, hosts) = store();
    std::fs::create_dir_all(hosts.path().parent().unwrap()).unwrap();
    // 别的程序写的、没有换行结尾的旧内容。
    std::fs::write(hosts.path(), format!("old.example.test {KEY_B}")).unwrap();
    hosts.learn("jump", 22, &key(KEY_A)).unwrap();
    hosts.learn("jump", 2222, &key(KEY_C)).unwrap();
    let text = std::fs::read_to_string(hosts.path()).unwrap();
    assert_eq!(
        text,
        format!("old.example.test {KEY_B}\njump {KEY_A}\n[jump]:2222 {KEY_C}\n")
    );
    let leftovers: Vec<_> = std::fs::read_dir(hosts.path().parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() != "known_hosts")
        .collect();
    assert!(leftovers.is_empty(), "不留临时文件：{leftovers:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(hosts.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn concurrent_writes_do_not_lose_lines() {
    let (_dir, hosts) = store();
    let threads: Vec<_> = (0..8u16)
        .map(|i| {
            let hosts = hosts.clone();
            std::thread::spawn(move || hosts.learn("jump", 3000 + i, &key(KEY_A)).unwrap())
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    for i in 0..8u16 {
        assert_eq!(
            hosts.check("jump", 3000 + i, &key(KEY_A)).unwrap(),
            HostKeyCheck::Trusted
        );
    }
}

#[test]
fn only_plain_host_names_are_accepted() {
    for ok in [
        "jump.example.test",
        "10.0.0.5",
        "fe80::1%en0",
        "db-1_a",
        "跳板机",
    ] {
        assert!(valid_host(ok), "{ok}");
    }
    for bad in [
        "",
        "a,b",
        "a|b",
        "@revoked",
        "*.example",
        "[::1]",
        "a b",
        "a?",
        "!a",
        "#a",
    ] {
        assert!(!valid_host(bad), "{bad}");
    }
}
