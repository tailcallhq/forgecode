//! Centralized [`reqwest::Client`] construction for the workspace.
//!
//! `reqwest 0.13` removed the compiled-in `rustls-tls-webpki-roots` feature
//! and switched its default trust source to `rustls-platform-verifier`, which
//! synchronously parses the OS trust store on every `Client::build()`
//! (~38 ms on Linux). Per the upstream `0.13.0` changelog, the recommended
//! replacement is to call `ClientBuilder::tls_certs_only(your_roots)`.
//!
//! All `reqwest::Client`s in the codebase should be built from
//! [`builder`] so the trust-source decision lives in one place and the
//! platform-verifier cost is avoided on cold-start paths.

use reqwest::ClientBuilder;
use reqwest::tls::Certificate;

/// Returns a [`reqwest::ClientBuilder`] preconfigured with the bundled
/// Mozilla webpki root CAs.
pub fn builder() -> ClientBuilder {
    reqwest::Client::builder().tls_certs_only(webpki_root_certs())
}

fn webpki_root_certs() -> impl IntoIterator<Item = Certificate> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|der| Certificate::from_der(der.as_ref()).ok())
}
