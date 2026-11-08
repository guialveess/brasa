//! [`StatusKey`], how a single load test sample is classified.

use serde::{Deserialize, Serialize};

/// How a single sample of a load test run is classified, for the "Responses by status"
/// breakdown and the per-target error rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StatusKey {
    /// A response was received with this HTTP status code.
    Code(u16),
    /// The request timed out (`brasa_http::HttpError::Timeout`).
    Timeout,
    /// The request could not be completed for any other reason: a pre-request script exception,
    /// or a send failure other than a timeout.
    Failed,
}

impl StatusKey {
    /// Whether a sample classified as `self` counts as an error, no response at all
    /// ([`StatusKey::Timeout`], [`StatusKey::Failed`]), or an HTTP status of 400 or above.
    #[must_use]
    pub fn is_error(&self) -> bool {
        match self {
            StatusKey::Code(code) => *code >= 400,
            StatusKey::Timeout | StatusKey::Failed => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_success_code_is_not_an_error() {
        assert!(!StatusKey::Code(200).is_error());
        assert!(!StatusKey::Code(301).is_error());
        assert!(!StatusKey::Code(399).is_error());
    }

    #[test]
    fn a_client_or_server_error_code_is_an_error() {
        assert!(StatusKey::Code(400).is_error());
        assert!(StatusKey::Code(404).is_error());
        assert!(StatusKey::Code(500).is_error());
    }

    #[test]
    fn timeout_and_failed_are_always_errors() {
        assert!(StatusKey::Timeout.is_error());
        assert!(StatusKey::Failed.is_error());
    }
}
