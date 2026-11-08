//! The error type returned by [`super::parse_collection`] and [`super::parse_environment`].

/// Everything that can go wrong while parsing a Postman export.
#[derive(Debug, thiserror::Error)]
pub enum PostmanError {
    /// The collection text is not valid JSON, or not shaped like a Postman collection.
    #[error("invalid Postman collection JSON: {0}")]
    InvalidCollectionJson(#[source] serde_json::Error),
    /// The environment text is not valid JSON, or not shaped like a Postman environment export.
    #[error("invalid Postman environment JSON: {0}")]
    InvalidEnvironmentJson(#[source] serde_json::Error),
}
