//! Types produced by sending a request: the resolved request actually sent, the response
//! received, and the outcome of post-response scripts.

use std::time::{Duration, SystemTime};

use crate::method::Method;

/// A single header or form field after interpolation, with disabled entries already removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedField {
    /// The field name, after interpolation.
    pub name: String,
    /// The field value, after interpolation.
    pub value: String,
}

/// The body of a [`ResolvedRequest`], after interpolation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ResolvedBody {
    /// No body.
    #[default]
    None,
    /// A body ready to send, already encoded to bytes (for example a JSON string, or an
    /// urlencoded form).
    Bytes(Vec<u8>),
}

/// A [`crate::Request`] after variable interpolation: only enabled headers and query entries,
/// the query string appended to the URL, everything resolved to plain values. This is what
/// `brasa-http` actually sends.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedRequest {
    /// The HTTP method.
    pub method: Method,
    /// The final URL, including the appended, interpolated query string.
    pub url: String,
    /// The headers to send, interpolated, disabled entries removed.
    pub headers: Vec<ResolvedField>,
    /// The body to send.
    pub body: ResolvedBody,
}

/// An HTTP response, as received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The HTTP status code.
    pub status: u16,
    /// The response headers, in the order received.
    pub headers: Vec<ResolvedField>,
    /// The raw response body.
    pub body: Vec<u8>,
    /// Total time spent sending the request and receiving the response.
    pub time: Duration,
    /// The response body size in bytes, equal to `body.len()`.
    pub size: usize,
    /// Set when the server's TLS certificate could not be verified and the request was sent
    /// anyway without verification. Holds a human readable message with the reason.
    pub tls_warning: Option<String>,
}

/// The outcome of a single `test(name, fn)` call in a `::: post` script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    /// The test name, as passed to `test()`.
    pub name: String,
    /// Whether every `expect()` assertion in the test passed.
    pub passed: bool,
    /// The failure message, if any. `None` when the test passed.
    pub message: Option<String>,
}

/// The severity of a [`ConsoleLine`], matching the `console.*` method used in a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleLevel {
    /// `console.log`
    Log,
    /// `console.info`
    Info,
    /// `console.warn`
    Warn,
    /// `console.error`
    Error,
}

/// The script a [`ConsoleLine`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStage {
    /// The `::: pre` script, run before the request is sent.
    Pre,
    /// The `::: post` script, run after the response arrives.
    Post,
}

/// A single line captured from `console.log/info/warn/error` in a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleLine {
    /// The console method used.
    pub level: ConsoleLevel,
    /// The logged text.
    pub text: String,
    /// The script that logged it.
    pub stage: ScriptStage,
    /// When it was logged. `None` when the script's clock could not be read.
    pub time: Option<SystemTime>,
    /// The line of the script that logged it, from 1, when the engine's stack trace has it.
    pub line: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn resolved_request_default_has_no_body() {
        let request = ResolvedRequest::default();
        assert_eq!(request.method, Method::Get);
        assert_eq!(request.body, ResolvedBody::None);
        assert!(request.headers.is_empty());
    }
}
