//! Telling spec files apart from other YAML and JSON files, and reading Backstage
//! `catalog-info.yaml` files, whose API entities point to the specs of a repository.

use serde::Deserialize;
use serde_json::Value;

use super::error::OpenApiError;

/// How much of a file [`looks_like_spec`] reads: the version field comes first in practice.
const SNIFF_BYTES: usize = 16 * 1024;

/// Whether `text` looks like the root of an OpenAPI or Swagger spec: an `openapi` or `swagger`
/// key at the top level, near the start. A cheap test to pick spec files out of a folder before
/// parsing them; [`super::parse_spec`] makes the real check.
pub fn looks_like_spec(text: &str) -> bool {
    let mut end = text.len().min(SNIFF_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = text[..end].trim_start_matches('\u{feff}');
    let json = head.trim_start().starts_with('{');
    head.lines().any(|line| {
        if json {
            let line = line.trim_start();
            line.starts_with("\"openapi\"") || line.starts_with("\"swagger\"") || {
                // A minified file: the key right after the opening brace.
                let line = line.trim_start_matches('{').trim_start();
                line.starts_with("\"openapi\"") || line.starts_with("\"swagger\"")
            }
        } else {
            // Top level YAML keys start at the first column.
            [
                "openapi:",
                "swagger:",
                "\"openapi\":",
                "\"swagger\":",
                "'openapi':",
                "'swagger':",
            ]
            .iter()
            .any(|key| line.starts_with(key))
        }
    })
}

/// Whether `text` is a Backstage catalog file: an `apiVersion` of `backstage.io/...`.
pub fn looks_like_catalog(text: &str) -> bool {
    text.lines()
        .any(|line| line.starts_with("apiVersion:") && line.contains("backstage.io/"))
}

/// An OpenAPI API entity of a catalog file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogApi {
    /// `metadata.name`.
    pub name: String,
    /// Where its spec is.
    pub definition: CatalogDefinition,
}

/// The `spec.definition` of an API entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogDefinition {
    /// The spec text itself.
    Inline(String),
    /// A substitution (`$text`, `$yaml`, `$json`): a path relative to the catalog file, or a
    /// URL.
    Reference(String),
}

/// The OpenAPI API entities of a catalog file, every YAML document of it included. Entities of
/// other kinds, and APIs of other types (AsyncAPI, GraphQL, gRPC), are skipped.
pub fn parse_catalog(text: &str) -> Result<Vec<CatalogApi>, OpenApiError> {
    let mut apis = Vec::new();
    for document in serde_norway::Deserializer::from_str(text) {
        let value = Value::deserialize(document)
            .map_err(|error| OpenApiError::Syntax(error.to_string()))?;
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let spec = value.get("spec");
        let api_type = spec
            .and_then(|spec| spec.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !kind.eq_ignore_ascii_case("API") || !api_type.eq_ignore_ascii_case("openapi") {
            continue;
        }
        let definition = match spec.and_then(|spec| spec.get("definition")) {
            Some(Value::String(text)) => CatalogDefinition::Inline(text.clone()),
            Some(Value::Object(map)) => {
                let Some(reference) = ["$text", "$yaml", "$json"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str))
                else {
                    continue;
                };
                CatalogDefinition::Reference(reference.trim().to_string())
            }
            _ => continue,
        };
        apis.push(CatalogApi {
            name: value
                .get("metadata")
                .and_then(|metadata| metadata.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            definition,
        });
    }
    Ok(apis)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn spots_spec_roots() {
        assert!(looks_like_spec("openapi: 3.1.0\ninfo: {}"));
        assert!(looks_like_spec("# comment\nswagger: '2.0'\n"));
        assert!(looks_like_spec("{\n  \"openapi\": \"3.0.0\"\n}"));
        assert!(looks_like_spec("{\"swagger\":\"2.0\",\"info\":{}}"));
        // A schema fragment, or a nested key, is not a spec.
        assert!(!looks_like_spec("components:\n  openapi: x\n"));
        assert!(!looks_like_spec("type: object\nproperties: {}\n"));
        assert!(!looks_like_spec("{\"name\": \"openapi\"}"));
    }

    #[test]
    fn reads_the_openapi_entities_of_a_catalog() {
        let text = r##"apiVersion: backstage.io/v1alpha1
kind: Component
metadata: {name: payments}
spec: {type: service, providesApis: [payments-v1]}
---
apiVersion: backstage.io/v1alpha1
kind: API
metadata: {name: payments-v1}
spec:
  type: openapi
  definition:
    $text: ./specs/payments-v1.yaml
---
apiVersion: backstage.io/v1alpha1
kind: API
metadata: {name: payments-v2}
spec:
  type: openapi
  definition: |
    openapi: 3.1.0
---
apiVersion: backstage.io/v1alpha1
kind: API
metadata: {name: events}
spec: {type: asyncapi, definition: {$text: ./events.yaml}}
"##;
        assert!(looks_like_catalog(text));
        assert_eq!(
            parse_catalog(text).expect("valid"),
            vec![
                CatalogApi {
                    name: "payments-v1".into(),
                    definition: CatalogDefinition::Reference("./specs/payments-v1.yaml".into()),
                },
                CatalogApi {
                    name: "payments-v2".into(),
                    definition: CatalogDefinition::Inline("openapi: 3.1.0\n".into()),
                },
            ]
        );
        assert!(!looks_like_catalog("openapi: 3.1.0\n"));
    }
}
