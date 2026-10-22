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
    hosts.trust(&HostKey::new("jump", 22, &key(KEY_A))).unwrap();
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
    assert!(text.trim_start().starts_with("jump ssh-ed25519 "), "{text}");
}

#[test]
fn non_default_port_is_written_in_brackets() {
    let (_dir, hosts) = store();
    hosts
        .trust(&HostKey::new("10.0.0.9", 2222, &key(KEY_A)))
        .unwrap();
    let text = std::fs::read_to_string(hosts.path()).unwrap();
    assert!(
        text.trim_start()
            .starts_with("[10.0.0.9]:2222 ssh-ed25519 "),
        "{text}"
    );
    assert_eq!(
        hosts.check("10.0.0.9", 2222, &key(KEY_A)).unwrap(),
        HostKeyCheck::Trusted
    );
}

#[test]
fn changed_key_is_reported_with_both_fingerprints() {
    let (_dir, hosts) = store();
    hosts.trust(&HostKey::new("jump", 22, &key(KEY_A))).unwrap();
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
fn trust_refuses_to_overwrite_a_different_recorded_key() {
    let (_dir, hosts) = store();
    hosts.trust(&HostKey::new("jump", 22, &key(KEY_A))).unwrap();
    let error = hosts
        .trust(&HostKey::new("jump", 22, &key(KEY_B)))
        .unwrap_err();
    assert_eq!(error.code(), "host_key_changed");
    assert_eq!(
        hosts.check("jump", 22, &key(KEY_A)).unwrap(),
        HostKeyCheck::Trusted,
        "原来的记录不动"
    );
}

#[test]
fn another_algorithm_is_a_separate_entry() {
    let (_dir, hosts) = store();
    hosts.trust(&HostKey::new("jump", 22, &key(KEY_A))).unwrap();
    assert_eq!(
        hosts.check("jump", 22, &key(KEY_C)).unwrap(),
        HostKeyCheck::Unknown
    );
    hosts.trust(&HostKey::new("jump", 22, &key(KEY_C))).unwrap();
    assert_eq!(
        hosts.check("jump", 22, &key(KEY_C)).unwrap(),
        HostKeyCheck::Trusted
    );
}

#[test]
fn trust_requires_the_fingerprint_the_user_saw() {
    let (_dir, hosts) = store();
    let tampered = HostKey {
        public_key: KEY_B.into(),
        ..HostKey::new("jump", 22, &key(KEY_A))
    };
    let error = hosts.trust(&tampered).unwrap_err();
    assert!(error.to_string().contains("指纹对不上"), "{error}");
    let garbage = HostKey {
        public_key: "not a key".into(),
        ..HostKey::new("jump", 22, &key(KEY_A))
    };
    assert!(hosts.trust(&garbage).is_err());
    assert!(!hosts.path().exists(), "什么都没记");
}
