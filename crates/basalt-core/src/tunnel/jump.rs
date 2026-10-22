//! 跳板机的写法,和 OpenSSH 的 ProxyJump 一样:`[user@]host[:port]`,多个用逗号隔开,从左到右依次跳;
//! IPv6 地址带端口时写成 `[::1]:2222`。`host` 可以是 `~/.ssh/config` 里的别名。

use super::known_hosts::valid_host;

/// 一跳:用户名、端口没写时由 `~/.ssh/config` 或默认值补上。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JumpSpec {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

/// 解析跳板机清单。空白的清单是没有跳板机。错误说明哪一段写得不对。
pub fn parse_jump_hosts(spec: &str) -> Result<Vec<JumpSpec>, String> {
    spec.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(parse_one)
        .collect()
}

fn parse_one(part: &str) -> Result<JumpSpec, String> {
    let wrong = || format!("跳板机「{part}」的写法不对，应该像 user@bastion:22 这样");
    let (user, rest) = match part.rsplit_once('@') {
        Some((user, rest)) if !user.is_empty() && !user.contains(char::is_whitespace) => {
            (Some(user.to_owned()), rest)
        }
        Some(_) => return Err(wrong()),
        None => (None, part),
    };
    let (host, port) = if let Some(bracketed) = rest.strip_prefix('[') {
        let (host, after) = bracketed.split_once(']').ok_or_else(wrong)?;
        match after.strip_prefix(':') {
            Some(port) => (host, Some(port)),
            None if after.is_empty() => (host, None),
            None => return Err(wrong()),
        }
    } else {
        match rest.split_once(':') {
            // 只有一个冒号才是「主机:端口」;好几个冒号是没加方括号的 IPv6 地址。
            Some((host, port)) if !port.contains(':') => (host, Some(port)),
            _ => (rest, None),
        }
    };
    if host.is_empty() || !valid_host(host) {
        return Err(wrong());
    }
    let port = match port {
        None => None,
        Some(port) => match port.parse::<u16>() {
            Ok(port) if port > 0 => Some(port),
            _ => return Err(format!("跳板机「{part}」的端口不对，范围是 1–65535")),
        },
    };
    Ok(JumpSpec {
        user,
        host: host.to_owned(),
        port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(user: Option<&str>, host: &str, port: Option<u16>) -> JumpSpec {
        JumpSpec {
            user: user.map(str::to_owned),
            host: host.into(),
            port,
        }
    }

    #[test]
    fn reads_the_proxyjump_forms() {
        assert_eq!(parse_jump_hosts("").unwrap(), []);
        assert_eq!(parse_jump_hosts(" , ").unwrap(), []);
        assert_eq!(
            parse_jump_hosts("bastion, ops@10.0.0.2:2222 ,[::1]:22,fe80::1,admin@[::1]").unwrap(),
            [
                spec(None, "bastion", None),
                spec(Some("ops"), "10.0.0.2", Some(2222)),
                spec(None, "::1", Some(22)),
                spec(None, "fe80::1", None),
                spec(Some("admin"), "::1", None),
            ]
        );
    }

    #[test]
    fn explains_what_is_wrong() {
        for bad in ["@host", "user@", "a b", "[::1", "[::1]x", "host,*"] {
            assert!(parse_jump_hosts(bad).is_err(), "{bad}");
        }
        let error = parse_jump_hosts("bastion:99999").unwrap_err();
        assert!(error.contains("端口"), "{error}");
    }
}
