use super::*;

const CONFIG: &str = r#"
# 别的工具加的
Include conf.d/*

Host bastion jump-*
    HostName bastion.example.com
    User ops
    Port 2222
    IdentityFile ~/.ssh/ops_key

Host db-prod
    HostName 10.0.3.7
    ProxyJump bastion
    User deploy

Host !nope *.internal
    User someone

Host *
    IdentityAgent "~/Library/Group Containers/agent.sock"
    ServerAliveInterval 30
"#;

fn parsed(home: &Path) -> SshConfigFile {
    SshConfigFile::parse(CONFIG, home).unwrap()
}

fn home_with_include() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".ssh/conf.d")).unwrap();
    std::fs::write(
        home.path().join(".ssh/conf.d/orbstack"),
        "Host orb\n    HostName 127.0.0.1\n    Port 32222\n    User default\n",
    )
    .unwrap();
    home
}

#[test]
fn looks_up_a_host_like_openssh_first_value_wins() {
    let home = home_with_include();
    let config = parsed(home.path());
    let agent = home
        .path()
        .join("Library/Group Containers/agent.sock")
        .display()
        .to_string();
    let bastion = config.lookup("bastion");
    assert_eq!(bastion.host_name.as_deref(), Some("bastion.example.com"));
    assert_eq!(bastion.port, Some(2222));
    assert_eq!(bastion.user.as_deref(), Some("ops"));
    assert_eq!(
        bastion.identity_files,
        [home.path().join(".ssh/ops_key").display().to_string()]
    );
    assert_eq!(bastion.identity_agent.as_deref(), Some(agent.as_str()));

    let prod = config.lookup("db-prod");
    assert_eq!(prod.proxy_jump.as_deref(), Some("bastion"));
    assert_eq!(prod.user.as_deref(), Some("deploy"));

    // 没写的名字只拿到通配段的设置。
    let other = config.lookup("10.9.9.9");
    assert_eq!(other.host_name, None);
    assert_eq!(other.identity_agent.as_deref(), Some(agent.as_str()));
    assert_eq!(config.lookup("x.internal").user.as_deref(), Some("someone"));
}

#[test]
fn lists_named_hosts_for_the_connection_dialog() {
    let home = home_with_include();
    let hosts = parsed(home.path()).hosts();
    let aliases: Vec<_> = hosts.iter().map(|h| h.alias.as_str()).collect();
    assert_eq!(aliases, ["orb", "bastion", "db-prod"]);
    assert_eq!(
        hosts[0],
        SshConfigHost {
            alias: "orb".into(),
            host: "127.0.0.1".into(),
            port: 32222,
            username: Some("default".into()),
            private_key_path: None,
            jump_hosts: None,
        }
    );
    assert_eq!(hosts[2].host, "10.0.3.7");
    assert_eq!(hosts[2].port, 22);
    assert_eq!(hosts[2].jump_hosts.as_deref(), Some("bastion"));
}

#[test]
fn agent_and_jump_set_to_none_mean_none() {
    let home = tempfile::tempdir().unwrap();
    let config = SshConfigFile::parse(
        "Host a\n  ProxyJump none\n  IdentityAgent SSH_AUTH_SOCK\nHost b\n  IdentityAgent none\n",
        home.path(),
    )
    .unwrap();
    assert_eq!(config.lookup("a").proxy_jump, None);
    assert_eq!(config.lookup("a").identity_agent, None);
    assert_eq!(config.lookup("b").identity_agent, None);
}

#[test]
fn a_missing_or_broken_file_is_treated_as_empty() {
    let empty = SshConfigFile::empty();
    assert_eq!(empty.lookup("anything"), HostSettings::default());
    assert!(empty.hosts().is_empty());
    assert!(SshConfigFile::parse("Host\n", Path::new("/nonexistent")).is_err());
}
