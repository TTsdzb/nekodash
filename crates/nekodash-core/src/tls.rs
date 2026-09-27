use crate::{Error, Result};
use rustls::{
    ClientConfig, RootCertStore,
    pki_types::{CertificateDer, pem::PemObject},
};
use std::sync::Arc;

pub(crate) fn config(additional_ca_pem: &[String]) -> Result<Arc<ClientConfig>> {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for pem in additional_ca_pem {
        let mut count = 0;
        for cert in CertificateDer::pem_slice_iter(pem.as_bytes()) {
            let cert = cert.map_err(|_| Error::invalid("invalid PEM certificate"))?;
            roots
                .add(cert)
                .map_err(|_| Error::invalid("invalid CA certificate"))?;
            count += 1;
        }
        if count == 0 {
            return Err(Error::invalid("PEM contains no certificates"));
        }
    }
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|_| Error::invalid("invalid TLS protocol configuration"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
    Ok(Arc::new(config))
}
