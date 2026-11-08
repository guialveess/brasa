//! Options controlling how [`crate::send`] performs a single request.

use std::time::Duration;

/// Options for [`crate::send`].
///
/// The defaults match what most HTTP clients do out of the box: a generous timeout, redirects
/// followed up to a sane limit, and TLS certificates verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendOptions {
    /// The maximum time to wait for the whole request, from opening the connection to reading
    /// the last byte of the response body. This includes time spent following redirects.
    ///
    /// Defaults to 30 seconds.
    pub timeout: Duration,

    /// Whether to follow HTTP redirects (3xx responses with a `Location` header).
    ///
    /// When `false`, the redirect response itself is returned as a normal
    /// [`brasa_core::Response`], instead of being followed.
    ///
    /// Defaults to `true`.
    pub follow_redirects: bool,

    /// The maximum number of redirects to follow before giving up with
    /// [`crate::HttpError::TooManyRedirects`]. Ignored when `follow_redirects` is `false`.
    ///
    /// Defaults to 10.
    pub max_redirects: u32,

    /// What to do when an `https://` server presents an invalid TLS certificate (self-signed,
    /// expired, for another host, ...). See [`InvalidCertificates`].
    ///
    /// Defaults to [`InvalidCertificates::SendWithWarning`].
    pub invalid_certificates: InvalidCertificates,

    /// Whether certificates are verified against the operating system's certificate store (the
    /// Windows certificate store, the macOS Keychain, the system bundle on Linux), like a
    /// browser does, instead of the Mozilla roots bundled with Brasa. The system store also
    /// has the company CAs an administrator installs, so internal APIs and networks that
    /// inspect TLS verify without a warning. A certificate the store rejects is then handled by
    /// [`Self::invalid_certificates`].
    ///
    /// Defaults to `true`.
    pub system_certificates: bool,

    /// The largest response body, in bytes, that is read. A bigger one fails with
    /// [`crate::HttpError::BodyTooLarge`] instead of filling the memory.
    ///
    /// Defaults to [`DEFAULT_MAX_RESPONSE_SIZE`].
    pub max_response_size: u64,
}

/// The default [`SendOptions::max_response_size`]: 20 MB (decimal, like the sizes the app shows).
///
/// Measured on 2026-10-06 (#35): a 15 MB JSON response takes about 475 MB of extra RSS in the
/// app (the raw bytes, the text, the pretty printed copy and the editor buffer), so memory
/// grows about 30 times faster than the body. The app lets the user raise it in Settings.
pub const DEFAULT_MAX_RESPONSE_SIZE: u64 = 20_000_000;

/// What [`crate::send`] does when the server's TLS certificate is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InvalidCertificates {
    /// Verify the certificate. When it is rejected, send the request once more without
    /// verification and set [`brasa_core::Response::tls_warning`] on the response.
    #[default]
    SendWithWarning,
    /// Verify the certificate. When it is rejected, fail with an error and do not retry.
    Reject,
    /// Never verify the certificate: any certificate is accepted, with no warning.
    Accept,
}

impl Default for SendOptions {
    /// Thirty second timeout, redirects followed up to 10 times, certificates verified against
    /// the system store, requests to servers with an invalid TLS certificate sent anyway with a
    /// warning, and response bodies up to 20 MB.
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            follow_redirects: true,
            max_redirects: 10,
            invalid_certificates: InvalidCertificates::SendWithWarning,
            system_certificates: true,
            max_response_size: DEFAULT_MAX_RESPONSE_SIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn defaults_match_the_documented_values() {
        let options = SendOptions::default();
        assert_eq!(options.timeout, Duration::from_secs(30));
        assert!(options.follow_redirects);
        assert_eq!(options.max_redirects, 10);
        assert_eq!(
            options.invalid_certificates,
            InvalidCertificates::SendWithWarning
        );
        assert!(options.system_certificates);
        assert_eq!(options.max_response_size, 20_000_000);
    }
}
