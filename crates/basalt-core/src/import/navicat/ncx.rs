//! Navicat「导出连接」的 .ncx:`<Connections><Connection ConnectionName=… ConnType=… …/></Connections>`。

use std::collections::HashMap;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use super::transport::{RawSsh, RawSsl};
use super::{HTTP_TUNNEL_WARNING, RawConnection, decrypt_password, into_sources};
use crate::error::{CoreError, CoreResult};
use crate::import::{SourceConnection, SourcePassword};

pub fn parse_ncx(xml: &str) -> CoreResult<Vec<SourceConnection>> {
    let mut reader = Reader::from_str(xml.trim_start_matches('\u{feff}'));
    let mut has_root = false;
    let mut raws = Vec::new();
    loop {
        // 出错信息只带位置:内容里有能还原出密码的密文,不能回显。
        let event = reader.read_event().map_err(|_| {
            not_ncx(&format!(
                "XML 格式有误（第 {} 字节附近）",
                reader.error_position()
            ))
        })?;
        match event {
            Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
                "Connections" => has_root = true,
                "Connection" if has_root => raws.push(raw_connection(&attributes(&element)?)),
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    if !has_root {
        return Err(not_ncx("没有 <Connections> 根元素"));
    }
    Ok(into_sources(raws))
}

fn not_ncx(detail: &str) -> CoreError {
    CoreError::Unsupported(format!("不是有效的 Navicat 导出文件（.ncx）：{detail}"))
}

fn attributes(element: &BytesStart<'_>) -> CoreResult<HashMap<String, String>> {
    element
        .attributes()
        .map(|attribute| {
            let attribute = attribute.map_err(|_| not_ncx("属性格式有误"))?;
            let key = attribute.key.local_name().as_ref().to_owned();
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|_| not_ncx(&format!("属性 {key} 的取值有误")))?
                .into_owned();
            Ok((key, value))
        })
        .collect()
}

fn raw_connection(attrs: &HashMap<String, String>) -> RawConnection {
    let text = |key: &str| attrs.get(key).cloned().unwrap_or_default();
    let flag = |key: &str| {
        attrs
            .get(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
    };
    // 没勾「保存密码」时密文即使有值也不该用。
    let secret = |value: &str, saved: &str| {
        let cipher = text(value);
        if !flag(saved) || cipher.trim().is_empty() {
            SourcePassword::Absent
        } else {
            decrypt_password(&cipher).map_or(SourcePassword::Undecryptable, SourcePassword::Plain)
        }
    };
    let ssh = flag("SSH").then(|| RawSsh {
        host: text("SSH_Host"),
        port: text("SSH_Port"),
        username: text("SSH_UserName"),
        auth_method: text("SSH_AuthenMethod"),
        private_key: text("SSH_PrivateKey"),
        password: secret("SSH_Password", "SSH_SavePassword"),
        passphrase: secret("SSH_Passphrase", "SSH_SavePassphrase"),
    });
    let ssl = flag("SSL").then(|| RawSsl {
        pg_mode: text("SSL_PGSSLMode"),
        verify_ca: flag("SSL_VerifyCACert"),
        allow_invalid_host_name: attrs
            .get("SSL_AllowInvalidHostName")
            .map(|v| v.eq_ignore_ascii_case("true")),
        client_auth: flag("SSL_Authen"),
        ca_cert: text("SSL_CACert"),
        client_cert: text("SSL_ClientCert"),
        client_key: text("SSL_ClientKey"),
        client_key_password: !text("SSL_PEMClientKeyPassword").trim().is_empty(),
        // Navicat 导出时就把这个属性名拼成了 Clpher。
        cipher: text("SSL_Clpher"),
        crl: text("SSL_PGSSLCRL"),
    });
    let notes = if flag("HTTP") {
        vec![HTTP_TUNNEL_WARNING.to_owned()]
    } else {
        Vec::new()
    };
    RawConnection {
        name: text("ConnectionName"),
        conn_type: text("ConnType"),
        host: text("Host"),
        port: text("Port"),
        username: text("UserName"),
        database: text("Database"),
        file_path: text("DatabaseFileName"),
        password: secret("Password", "SavePassword"),
        ssh,
        ssl,
        notes,
    }
}

#[cfg(test)]
#[path = "ncx_tests.rs"]
mod tests;
