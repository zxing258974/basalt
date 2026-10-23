//! Redis 的 TLS:在建好的连接流上自己握手,证书按连接配置里的原主机名核对(经 SSH 隧道也一样)。
//!
//! 档位和 MySQL 那边一致:「必须」只加密不校验证书,选了 CA 证书就升成「校验 CA」;
//! 「校验 CA」校验证书链、不核对主机名;「校验主机名」两样都校验。Redis 没有明文和 TLS 之间的协商,
//! 「禁用」「优先」都是明文。

use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, ring};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};

use crate::config::{SslMode, SslSettings};
use crate::error::{CoreError, CoreResult};

enum Verify {
    /// 只加密。
    Nothing,
    /// 校验证书链,不核对主机名。
    Chain,
    /// 证书链和主机名都校验。
    Full,
}

fn blank(path: &Option<String>) -> bool {
    path.as_deref().is_none_or(|p| p.trim().is_empty())
}

/// 不加密时为 `None`。
pub(super) fn client_config(ssl: &SslSettings) -> CoreResult<Option<Arc<ClientConfig>>> {
    let verify = match ssl.mode {
        SslMode::Disabled | SslMode::Preferred => return Ok(None),
        SslMode::Required if blank(&ssl.ca_path) => Verify::Nothing,
        SslMode::Required | SslMode::VerifyCa => Verify::Chain,
        SslMode::VerifyIdentity => Verify::Full,
    };
    let provider = Arc::new(ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| CoreError::Network(format!("TLS 配置有误：{e}")))?;
    let builder = match verify {
        Verify::Nothing => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(
                provider.signature_verification_algorithms,
            ))),
        Verify::Chain => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(IgnoreName(webpki(ssl, &provider)?))),
        Verify::Full => builder.with_webpki_verifier(webpki(ssl, &provider)?),
    };
    let config = match (&ssl.client_cert_path, &ssl.client_key_path) {
        (Some(cert), Some(key))
            if !blank(&ssl.client_cert_path) && !blank(&ssl.client_key_path) =>
        {
            let certs = read_certs(cert, "客户端证书")?;
            let key = PrivateKeyDer::from_pem_file(key)
                .map_err(|e| CoreError::File(format!("读不了客户端私钥文件 {key}：{e}")))?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| CoreError::Network(format!("客户端证书和私钥用不了：{e}")))?
        }
        _ => builder.with_no_client_auth(),
    };
    Ok(Some(Arc::new(config)))
}

/// 选了 CA 证书就只认它,否则认常见的公共 CA。
fn webpki(
    ssl: &SslSettings,
    provider: &Arc<CryptoProvider>,
) -> CoreResult<Arc<WebPkiServerVerifier>> {
    let mut roots = RootCertStore::empty();
    match ssl.ca_path.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(path) => {
            for cert in read_certs(path, "CA 证书")? {
                roots
                    .add(cert)
                    .map_err(|e| CoreError::File(format!("CA 证书 {path} 用不了：{e}")))?;
            }
        }
        None => roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned()),
    }
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .build()
        .map_err(|e| CoreError::File(format!("CA 证书用不了：{e}")))
}

fn read_certs(path: &str, label: &str) -> CoreResult<Vec<CertificateDer<'static>>> {
    let certs = CertificateDer::pem_file_iter(path)
        .and_then(Iterator::collect::<Result<Vec<_>, _>>)
        .map_err(|e| CoreError::File(format!("读不了{label}文件 {path}：{e}")))?;
    if certs.is_empty() {
        return Err(CoreError::File(format!("{label}文件 {path} 里没有证书")));
    }
    Ok(certs)
}

/// 「校验 CA」:证书链照常校验,只有主机名对不上这一种失败放行。
#[derive(Debug)]
struct IgnoreName(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for IgnoreName {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}

/// 「必须」:不看证书是谁签的,握手签名照样验(证明对方确实持有证书的私钥)。
#[derive(Debug)]
struct AcceptAny(WebPkiSupportedAlgorithms);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}
