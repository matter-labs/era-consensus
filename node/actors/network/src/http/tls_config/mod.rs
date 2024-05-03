use std::sync::Arc;
use tokio_rustls::rustls::{
    pki_types::{CertificateDer, PrivateKeyDer},
    ServerConfig,
};

const CERT: &[u8] = include_bytes!("local.cert");
const PKEY: &[u8] = include_bytes!("local.key");

pub type Acceptor = tokio_rustls::TlsAcceptor;

fn tls_acceptor_impl(key_der: &[u8], cert_der: &[u8]) -> Acceptor {
    let key = PrivateKeyDer::Pkcs1(key_der.to_owned().into());
    let cert = CertificateDer::from(cert_der).into_owned();
    Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .unwrap(),
    )
    .into()
}

pub fn tls_acceptor() -> Acceptor {
    tls_acceptor_impl(PKEY, CERT)
}