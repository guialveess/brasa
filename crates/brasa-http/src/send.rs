//! The `send` function and the [`Client`] type: hand a [`ResolvedRequest`] to `ureq` and turn
//! the result back into plain `brasa-core` types.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Instant;

use brasa_core::{ResolvedBody, ResolvedField, ResolvedRequest, Response, log_safe};

use crate::error::HttpError;
use crate::options::{InvalidCertificates, SendOptions};

/// Sends a fully resolved request over HTTP and returns the response, on a connection of its
/// own: it is opened for this request (with its TLS handshake over HTTPS) and closed afterwards.
/// [`Client::send`] keeps connections open between requests instead.
///
/// 4xx and 5xx status codes are returned as a normal [`Response`], not as an [`HttpError`]:
/// only failures that prevent us from getting any response at all (an invalid request, a
/// timeout, too many redirects, a connection failure, ...) are errors. The returned response's
/// `time` is the total time spent sending the request and reading the whole response body.
///
/// With [`InvalidCertificates::SendWithWarning`] and a server certificate that is rejected
/// (self-signed, expired, wrong host, ...), the request is sent once more with verification
/// disabled, and the returned response carries a [`Response::tls_warning`]. This is safe for
/// any method: a certificate error aborts the TLS handshake, before any request byte is written.
pub fn send(request: &ResolvedRequest, options: &SendOptions) -> Result<Response, HttpError> {
    // A client of its own, dropped with its connection once the response is read.
    Client::new(options.clone()).send(request)
}

/// Sends requests with keep-alive: the connection a request opens stays open after its response
/// and is reused by the next request to the same scheme, host and port, skipping the TCP connect
/// and the TLS handshake.
///
/// A client holds one connection pool. `ureq` keeps at most 3 idle connections per host and 10
/// overall in it, so a client is meant to be used by one sender at a time (a load test creates
/// one per virtual user), not shared by many threads sending at once: most of their connections
/// would be closed when they go back to the pool.
///
/// With [`InvalidCertificates::SendWithWarning`], a host whose certificate is rejected once is
/// remembered, and its later requests go straight to the connection without verification, with
/// the same [`Response::tls_warning`].
pub struct Client {
    options: SendOptions,
    /// Sends every request, verifying certificates unless they are always accepted.
    agent: ureq::Agent,
    /// Sends without verification, built the first time a certificate is rejected.
    unverified: OnceLock<ureq::Agent>,
    /// The warning of each origin (`scheme://host:port`) whose certificate was rejected.
    unverified_origins: Mutex<HashMap<String, String>>,
}

impl Client {
    /// A client that sends with `options`. No connection is opened until the first request.
    #[must_use]
    pub fn new(options: SendOptions) -> Self {
        let verify = options.invalid_certificates != InvalidCertificates::Accept;
        Self {
            agent: build_agent(&options, verify),
            options,
            unverified: OnceLock::new(),
            unverified_origins: Mutex::new(HashMap::new()),
        }
    }

    /// Sends a fully resolved request and returns the response, reusing an open connection to
    /// its origin when there is one. Errors and certificate handling are those of [`send`].
    pub fn send(&self, request: &ResolvedRequest) -> Result<Response, HttpError> {
        if log::log_enabled!(log::Level::Trace) {
            log_request(request);
        }
        let http_request = build_http_request(request)?;
        let started = Instant::now();

        let fallback = self.options.invalid_certificates == InvalidCertificates::SendWithWarning;
        let origin = origin_of(&http_request);
        let known_warning = if fallback {
            self.origins().get(&origin).cloned()
        } else {
            None
        };
        let (response, tls_warning) = match known_warning {
            Some(warning) => {
                let response = run(self.unverified_agent(), &http_request);
                (response.map_err(map_ureq_error)?, Some(warning))
            }
            None => match run(&self.agent, &http_request) {
                Err(error) if fallback && is_certificate_error(&error) => {
                    let warning = format!(
                        "TLS certificate not verified: {}",
                        certificate_reason(&error)
                    );
                    log::debug!("{warning}, sending again without verification");
                    self.origins().insert(origin, warning.clone());
                    let retry = run(self.unverified_agent(), &http_request);
                    (retry.map_err(map_ureq_error)?, Some(warning))
                }
                other => (other.map_err(map_ureq_error)?, None),
            },
        };
        finish(
            response,
            started,
            tls_warning,
            self.options.max_response_size,
        )
    }

    /// The agent that sends without verifying certificates.
    fn unverified_agent(&self) -> &ureq::Agent {
        self.unverified
            .get_or_init(|| build_agent(&self.options, false))
    }

    /// The origins whose certificate was rejected. Recovers from a poisoned lock: the map is
    /// only read and inserted into, so it is never left half updated.
    fn origins(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.unverified_origins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// The `scheme://host:port` a request goes to, the unit connections are reused for.
fn origin_of(request: &ureq::http::Request<Vec<u8>>) -> String {
    let uri = request.uri();
    format!(
        "{}://{}",
        uri.scheme_str().unwrap_or_default(),
        uri.authority().map_or("", |authority| authority.as_str())
    )
}

/// Logs the request line and headers at Trace, with sensitive header values redacted and the
/// query string left out.
fn log_request(request: &ResolvedRequest) {
    log::trace!(
        "sending {} {}",
        request.method,
        log_safe::url_for_log(&request.url)
    );
    for header in &request.headers {
        log::trace!(
            "  {}: {}",
            header.name,
            log_safe::header_value_for_log(&header.name, &header.value)
        );
    }
}

/// Runs one request on `agent`.
fn run(
    agent: &ureq::Agent,
    request: &ureq::http::Request<Vec<u8>>,
) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    agent.run(request.clone())
}

/// Reads the whole response body, up to `max_response_size` bytes, and turns it into a
/// [`Response`].
fn finish(
    mut response: ureq::http::Response<ureq::Body>,
    started: Instant,
    tls_warning: Option<String>,
    max_response_size: u64,
) -> Result<Response, HttpError> {
    let status = response.status().as_u16();
    let headers = collect_headers(response.headers());
    let mut body = response
        .body_mut()
        .with_config()
        .limit(max_response_size)
        .read_to_vec()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(limit) => HttpError::BodyTooLarge(limit),
            other => map_ureq_error(other),
        })?;
    // The Vec grew by doubling: give the spare capacity back, a big body is kept for as long as
    // its response is shown.
    body.shrink_to_fit();
    let time = started.elapsed();
    let size = body.len();

    Ok(Response {
        status,
        headers,
        body,
        time,
        size,
        tls_warning,
    })
}

/// The `rustls` error inside a `ureq` error, if any. `ureq` reports handshake failures either
/// as [`ureq::Error::Rustls`] or as an I/O error wrapping the `rustls` one.
fn rustls_error(error: &ureq::Error) -> Option<&rustls::Error> {
    match error {
        ureq::Error::Rustls(inner) => Some(inner),
        ureq::Error::Io(io) => io.get_ref()?.downcast_ref::<rustls::Error>(),
        _ => None,
    }
}

/// Whether `error` means the server's certificate was rejected (as opposed to any other TLS or
/// network failure).
fn is_certificate_error(error: &ureq::Error) -> bool {
    matches!(
        rustls_error(error),
        Some(rustls::Error::InvalidCertificate(_))
    )
}

/// A short description of why the certificate was rejected.
fn certificate_reason(error: &ureq::Error) -> String {
    match rustls_error(error) {
        Some(rustls::Error::InvalidCertificate(reason)) => match reason {
            rustls::CertificateError::UnknownIssuer => "unknown issuer (self-signed?)".to_string(),
            rustls::CertificateError::Expired | rustls::CertificateError::ExpiredContext { .. } => {
                "certificate expired".to_string()
            }
            rustls::CertificateError::NotValidYet
            | rustls::CertificateError::NotValidYetContext { .. } => {
                "certificate not valid yet".to_string()
            }
            rustls::CertificateError::NotValidForName
            | rustls::CertificateError::NotValidForNameContext { .. } => {
                "certificate does not match the host name".to_string()
            }
            other => format!("{other:?}"),
        },
        _ => error.to_string(),
    }
}

/// Builds a `ureq` agent configured from `options`.
///
/// This disables `ureq`'s default behavior of treating 4xx/5xx as errors, allows the
/// non-standard HTTP methods a `.brasa` file may use, and applies the timeout, redirect and
/// TLS verification settings.
fn build_agent(options: &SendOptions, verify_tls: bool) -> ureq::Agent {
    let root_certs = if options.system_certificates {
        system_roots()
    } else {
        ureq::tls::RootCerts::WebPki
    };
    let tls_config = ureq::tls::TlsConfig::builder()
        .root_certs(root_certs)
        .disable_verification(!verify_tls)
        .build();

    let max_redirects = if options.follow_redirects {
        options.max_redirects
    } else {
        0
    };

    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .max_redirects(max_redirects)
        .timeout_global(Some(options.timeout))
        .tls_config(tls_config)
        .build();

    config.new_agent()
}

/// The roots of [`SendOptions::system_certificates`] on Windows and macOS: the platform verifier,
/// which asks the OS to verify each certificate and costs nothing to create.
#[cfg(not(all(unix, not(target_vendor = "apple"))))]
fn system_roots() -> ureq::tls::RootCerts {
    ureq::tls::RootCerts::PlatformVerifier
}

/// The roots of [`SendOptions::system_certificates`] on Linux and the BSDs: the system bundle
/// (`SSL_CERT_FILE` and `SSL_CERT_DIR` when set), which is all the platform verifier uses there.
/// The platform verifier reads and parses the bundle again for every agent, about 3.5 ms, and
/// [`send`] creates one per request, so a load test without keep-alive would pay it on every
/// request (measured on 2026-10-18, #114). The bundle is read once per process instead. Without
/// any readable system certificate, the bundled Mozilla roots are used.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn system_roots() -> ureq::tls::RootCerts {
    static ROOTS: OnceLock<ureq::tls::RootCerts> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let result = rustls_native_certs::load_native_certs();
            for error in &result.errors {
                log::warn!("could not load a system CA certificate: {error}");
            }
            if result.certs.is_empty() {
                log::warn!("no system CA certificates found, using the bundled roots");
                return ureq::tls::RootCerts::WebPki;
            }
            log::debug!("loaded {} system CA certificates", result.certs.len());
            let certs = result
                .certs
                .iter()
                .map(|cert| ureq::tls::Certificate::from_der(cert).to_owned())
                .collect();
            ureq::tls::RootCerts::Specific(std::sync::Arc::new(certs))
        })
        .clone()
}

/// Turns a [`ResolvedRequest`] into an `http` crate request ready to hand to the `ureq` agent.
fn build_http_request(
    request: &ResolvedRequest,
) -> Result<ureq::http::Request<Vec<u8>>, HttpError> {
    let method = request.method.to_string();
    let mut builder = ureq::http::Request::builder()
        .method(method.as_str())
        .uri(&request.url);

    for header in &request.headers {
        builder = builder.header(&header.name, &header.value);
    }

    let body = match &request.body {
        ResolvedBody::None => Vec::new(),
        ResolvedBody::Bytes(bytes) => bytes.clone(),
    };

    builder
        .body(body)
        .map_err(|error| HttpError::InvalidRequest(error.to_string()))
}

/// Reads the response headers into the plain [`ResolvedField`] list `brasa-core` expects.
///
/// Note: the `http` crate preserves insertion order for repeated values of the same header
/// name, but its documented iteration order across different header names is not guaranteed to
/// match the exact order the bytes arrived on the wire.
fn collect_headers(headers: &ureq::http::HeaderMap) -> Vec<ResolvedField> {
    headers
        .iter()
        .map(|(name, value)| ResolvedField {
            name: name.as_str().to_string(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect()
}

/// Maps a `ureq` error to our own [`HttpError`], giving the cases `brasa-http` cares about a
/// dedicated variant and keeping everything else under [`HttpError::Network`].
fn map_ureq_error(error: ureq::Error) -> HttpError {
    match error {
        ureq::Error::Timeout(_) => HttpError::Timeout,
        ureq::Error::TooManyRedirects => HttpError::TooManyRedirects,
        other if is_certificate_error(&other) => HttpError::Certificate(certificate_reason(&other)),
        other => HttpError::Network(other),
    }
}
