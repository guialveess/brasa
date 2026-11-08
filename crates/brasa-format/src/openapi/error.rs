//! The error type of [`super::parse_spec`].

/// Why a text could not be read as an OpenAPI or Swagger spec. Smaller problems (an unresolved
/// reference, an unsupported auth flow) are warnings on the result instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpenApiError {
    /// The text is neither valid JSON nor valid YAML.
    #[error("not valid JSON or YAML: {0}")]
    Syntax(String),
    /// The document parses but has no `openapi` or `swagger` version field.
    #[error("not an OpenAPI or Swagger document (no \"openapi\" or \"swagger\" field)")]
    NotOpenApi,
    /// The version field names a version this importer does not read.
    #[error("unsupported spec version {0}, expected OpenAPI 3.0, 3.1 or Swagger 2.0")]
    UnsupportedVersion(String),
}
