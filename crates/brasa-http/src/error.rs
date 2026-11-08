//! The error type returned by [`crate::send`].

/// Everything that can go wrong while sending a request with `brasa-http`.
///
/// Note that HTTP responses with a 4xx or 5xx status code are *not* errors: they are returned
/// as a normal [`brasa_core::Response`]. This type only covers failures that prevent us from
/// getting a response at all.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// The request could not be built, for example because the URL, a header name or a header
    /// value contains characters that are not valid in an HTTP request.
    #[error("could not build the request: {0}")]
    InvalidRequest(String),

    /// The request did not complete within [`crate::SendOptions::timeout`].
    #[error("the request timed out")]
    Timeout,

    /// The response followed more redirects than [`crate::SendOptions::max_redirects`] allows.
    #[error("too many redirects")]
    TooManyRedirects,

    /// The server's TLS certificate was rejected and [`crate::InvalidCertificates::Reject`] is
    /// in effect. Holds a short reason, such as "unknown issuer (self-signed?)".
    #[error("invalid TLS certificate: {0}")]
    Certificate(String),

    /// The response body is bigger than [`crate::SendOptions::max_response_size`]. Holds that
    /// limit in bytes.
    #[error("the response body is larger than the limit of {} MB", .0 / 1_000_000)]
    BodyTooLarge(u64),

    /// Any other failure while talking to the server: DNS resolution, connecting, TLS,
    /// reading or writing the socket, or a malformed HTTP response.
    #[error("network error: {0}")]
    Network(#[source] ureq::Error),
}
