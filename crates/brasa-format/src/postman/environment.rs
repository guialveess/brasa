//! Mapping a standalone Postman environment export (`*.postman_environment.json`) into an
//! [`ImportedEnvironment`].

use brasa_core::KeyValue;

use super::error::PostmanError;
use super::model::RawEnvironment;
use super::value::value_to_text;

/// The name used when a Postman environment export has no `name` field.
const DEFAULT_NAME: &str = "imported";

/// The result of importing a Postman environment export: the variables split into the ones that
/// go to the plain `.env` file and the ones (Postman's `"secret"` type) that go to `.local.env`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedEnvironment {
    /// The environment name (`name` in the export), used as the base file name.
    pub name: String,
    /// The non-secret variables, in export order, meant for `environments/<name>.env`.
    pub base: Vec<KeyValue>,
    /// The variables of type `"secret"`, in export order, meant for
    /// `environments/<name>.local.env`.
    pub secret: Vec<KeyValue>,
    /// Everything that could not be mapped faithfully, in the order it was found.
    pub warnings: Vec<String>,
}

/// Parses the text of a Postman environment export into an [`ImportedEnvironment`].
///
/// A disabled value (`"enabled": false`) is left out entirely: the `.env` format has no concept of
/// a disabled entry, so keeping it would silently turn it on.
pub fn parse_environment(text: &str) -> Result<ImportedEnvironment, PostmanError> {
    let raw: RawEnvironment =
        serde_json::from_str(text).map_err(PostmanError::InvalidEnvironmentJson)?;
    let mut warnings = Vec::new();
    let mut base = Vec::new();
    let mut secret = Vec::new();

    for value in raw.values {
        if !value.enabled {
            continue;
        }
        let context = format!("value \"{}\"", value.key);
        let text = value_to_text(value.value.as_ref(), &context, &mut warnings);
        let entry = KeyValue::new(value.key, text);
        if value.kind.as_deref() == Some("secret") {
            secret.push(entry);
        } else {
            base.push(entry);
        }
    }

    let name = match raw.name {
        Some(name) if !name.is_empty() => name,
        _ => {
            warnings.push(format!("environment has no name, using \"{DEFAULT_NAME}\""));
            DEFAULT_NAME.to_string()
        }
    };

    Ok(ImportedEnvironment {
        name,
        base,
        secret,
        warnings,
    })
}
