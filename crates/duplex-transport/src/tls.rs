use std::sync::{Arc, Once};

use rcgen::generate_simple_self_signed;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, ServerConfig, SignatureScheme};

pub struct SelfSignedCert {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivateKeyDer<'static>,
}

fn ensure_crypto_provider() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

impl SelfSignedCert {
    /// 生成自签名证书，subject 填本机标识
    pub fn generate(subject: &str) -> Result<Self, String> {
        let certified = generate_simple_self_signed(vec![subject.to_string()])
            .map_err(|e| format!("rcgen error: {}", e))?;

        let cert_der = certified.cert.der().clone();
        let key_der =
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()));

        Ok(Self { cert_der, key_der })
    }
}

/// 构造 Server TLS 配置
pub fn server_tls_config(cert: &SelfSignedCert) -> Result<Arc<ServerConfig>, String> {
    ensure_crypto_provider();

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert_der.clone()], cert.key_der.clone_key())
        .map_err(|e| format!("ServerConfig error: {}", e))?;

    Ok(Arc::new(config))
}

/// 构造 Client TLS 配置
/// 局域网场景：跳过证书链验证，只做传输加密
pub fn client_tls_config() -> Arc<ClientConfig> {
    ensure_crypto_provider();

    let config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipVerifier))
        .with_no_client_auth();

    Arc::new(config)
}

/// 跳过证书验证的 verifier
#[derive(Debug)]
struct SkipVerifier;

impl ServerCertVerifier for SkipVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
