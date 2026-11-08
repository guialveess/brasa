//! Reading an OpenAPI 3.0, 3.1 or Swagger 2.0 document into one normalized [`Spec`].

use std::str::FromStr;

use brasa_core::Method;
use serde_json::{Map, Value};

use super::document::{Documents, Node, RefLoader, parse_text};
use super::error::OpenApiError;
use super::example::{explicit_example, media_example};

/// The HTTP methods a path item can hold, in the order they are imported.
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// One API spec, normalized: OpenAPI 3.0, 3.1 and Swagger 2.0 all read into this.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    /// `info.title`.
    pub title: String,
    /// `info.version`, the version of the API itself (`2.0.0`).
    pub api_version: String,
    /// The version of the spec format (`3.1.0`, `3.0.3`, `2.0`).
    pub spec_version: String,
    /// The servers, with their URL variables replaced by their defaults.
    pub servers: Vec<Server>,
    /// The security schemes, in document order.
    pub security_schemes: Vec<SecurityScheme>,
    /// Every operation, in document order.
    pub operations: Vec<Operation>,
    /// Everything that could not be read faithfully, in the order found.
    pub warnings: Vec<String>,
}

/// A server the API is served from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// The URL, absolute when the spec's location allowed resolving a relative one.
    pub url: String,
    /// The server description, empty when there is none.
    pub description: String,
}

/// A security scheme (`components.securitySchemes` or Swagger's `securityDefinitions`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityScheme {
    /// The name the scheme is declared under, the one security requirements use.
    pub name: String,
    /// What kind of credential it takes.
    pub kind: SchemeKind,
}

/// The kinds of security scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemeKind {
    /// HTTP basic auth.
    Basic,
    /// A bearer token: HTTP bearer, and also OAuth 2 and OpenID Connect, whose flows end in one.
    Bearer,
    /// An API key sent in a header, the query string or a cookie.
    ApiKey {
        /// The header, query parameter or cookie name.
        name: String,
        /// Where the key goes.
        location: ParameterLocation,
    },
    /// Anything else (mutual TLS, an unknown HTTP scheme), which is not mapped.
    Other(String),
}

/// Where a parameter goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterLocation {
    /// A `{name}` segment of the path.
    Path,
    /// The query string.
    Query,
    /// A request header.
    Header,
    /// A cookie.
    Cookie,
}

/// One operation of a spec.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    /// The HTTP method.
    pub method: Method,
    /// The path as written in the spec, with `{name}` parameters (`/customers/{id}`).
    pub path: String,
    /// `operationId`, empty when there is none.
    pub operation_id: String,
    /// `summary`, empty when there is none.
    pub summary: String,
    /// `description`, empty when there is none.
    pub description: String,
    /// `tags`, in order.
    pub tags: Vec<String>,
    /// Whether the operation is marked deprecated.
    pub deprecated: bool,
    /// The path, query, header and cookie parameters, path item parameters included.
    pub parameters: Vec<Parameter>,
    /// The request body, if the operation takes one.
    pub body: Option<RequestBody>,
    /// The first documented success response, if any.
    pub success: Option<Success>,
    /// The security schemes the operation needs, by name: the first requirement of its own
    /// `security`, or of the spec's. Empty when it needs none.
    pub security: Vec<String>,
}

impl Operation {
    /// The name the request gets: the summary, else the operation id, else `METHOD /path`.
    pub fn display_name(&self) -> String {
        if !self.summary.trim().is_empty() {
            self.summary.trim().to_string()
        } else if !self.operation_id.trim().is_empty() {
            self.operation_id.trim().to_string()
        } else {
            format!("{} {}", self.method, self.path)
        }
    }
}

/// One parameter of an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    /// The parameter name.
    pub name: String,
    /// Where it goes.
    pub location: ParameterLocation,
    /// Whether it is required (path parameters always are).
    pub required: bool,
    /// The description, empty when there is none.
    pub description: String,
    /// An example value as text, when the spec gives one.
    pub example: Option<String>,
}

/// The request body of an operation.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestBody {
    /// The media type chosen among the ones the operation accepts (JSON first).
    pub media_type: String,
    /// The example value: the spec's own, or one built from the schema.
    pub example: Option<Value>,
}

/// The first success response an operation documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Success {
    /// One status code (`201`).
    Status(u16),
    /// The `2XX` range.
    AnySuccess,
}

/// Reads spec text (JSON or YAML) found at `location` into a [`Spec`]. `location` is the file
/// path or URL of the spec, empty for pasted text: external `$ref`s are loaded relative to it
/// through `loader`, and relative server URLs are resolved against it when it is a URL.
pub fn parse_spec(
    text: &str,
    location: &str,
    loader: &dyn RefLoader,
) -> Result<Spec, OpenApiError> {
    let value = parse_text(text)?;
    let version = detect_version(&value)?;
    let documents = Documents::load(value, location, loader);
    let root = documents.root();
    let swagger = version == "2.0";
    let info = root.get("info");
    let mut spec = Spec {
        title: info
            .and_then(|info| info.str("title"))
            .unwrap_or_default()
            .trim()
            .to_string(),
        api_version: info
            .and_then(|info| info.get("version"))
            .map(|node| scalar_text(node.value()))
            .unwrap_or_default(),
        spec_version: version,
        servers: if swagger {
            swagger_servers(root, location)
        } else {
            servers(root, location)
        },
        security_schemes: security_schemes(root, swagger),
        operations: Vec::new(),
        warnings: Vec::new(),
    };
    let global_security = root.get("security");
    let global_consumes = string_list(root.get("consumes"));
    if let Some(paths) = root.get("paths") {
        for (path, item) in paths.entries() {
            if !path.starts_with('/') {
                continue;
            }
            for method in METHODS {
                let Some(operation) = item.get(method) else {
                    continue;
                };
                spec.operations.push(read_operation(
                    method,
                    path,
                    item,
                    operation,
                    global_security,
                    &global_consumes,
                    swagger,
                ));
            }
        }
    }
    if spec.operations.is_empty() {
        documents.warn("the spec has no operations".to_string());
    }
    spec.warnings = documents.take_warnings();
    Ok(spec)
}

/// The spec format version (`openapi` or `swagger` field), checked to be one this reads.
fn detect_version(value: &Value) -> Result<String, OpenApiError> {
    if let Some(version) = value.get("openapi") {
        let version = scalar_text(version);
        return if version.starts_with("3.0") || version.starts_with("3.1") {
            Ok(version)
        } else {
            Err(OpenApiError::UnsupportedVersion(version))
        };
    }
    if let Some(version) = value.get("swagger") {
        let version = scalar_text(version);
        return if version == "2.0" || version == "2" {
            Ok("2.0".to_string())
        } else {
            Err(OpenApiError::UnsupportedVersion(version))
        };
    }
    Err(OpenApiError::NotOpenApi)
}

fn servers(root: Node<'_>, location: &str) -> Vec<Server> {
    let mut out = Vec::new();
    for server in root
        .get("servers")
        .map(|list| list.items())
        .unwrap_or_default()
    {
        let Some(url) = server.str("url") else {
            continue;
        };
        let mut url = url.trim().to_string();
        if let Some(variables) = server.get("variables") {
            for (name, variable) in variables.entries() {
                let default = variable
                    .get("default")
                    .map(|node| scalar_text(node.value()))
                    .unwrap_or_default();
                url = url.replace(&format!("{{{name}}}"), &default);
            }
        }
        out.push(Server {
            url: resolve_url(location, &url),
            description: server
                .str("description")
                .unwrap_or_default()
                .trim()
                .to_string(),
        });
    }
    if out.is_empty() && is_http(location) {
        // OpenAPI 3: no `servers` means one server at `/`, relative to the spec's location.
        out.push(Server {
            url: resolve_url(location, "/"),
            description: String::new(),
        });
    }
    out
}

fn swagger_servers(root: Node<'_>, location: &str) -> Vec<Server> {
    let base_path = root.str("basePath").unwrap_or("/");
    let host = match root.str("host") {
        Some(host) => host.to_string(),
        None if is_http(location) => origin(location)
            .split_once("://")
            .map(|(_, host)| host.to_string())
            .unwrap_or_default(),
        None => return Vec::new(),
    };
    let mut schemes = string_list(root.get("schemes"));
    if schemes.is_empty() {
        schemes.push(
            location
                .split_once("://")
                .map(|(scheme, _)| scheme.to_string())
                .filter(|scheme| scheme == "http" || scheme == "https")
                .unwrap_or_else(|| "https".to_string()),
        );
    }
    // One server: `https` when it is listed, the same host over plain `http` adds nothing.
    let scheme = if schemes.iter().any(|scheme| scheme == "https") {
        "https"
    } else if schemes.iter().any(|scheme| scheme == "http") {
        "http"
    } else {
        return Vec::new();
    };
    vec![Server {
        url: format!("{scheme}://{host}{}", base_path.trim_end_matches('/')),
        description: String::new(),
    }]
}

fn security_schemes(root: Node<'_>, swagger: bool) -> Vec<SecurityScheme> {
    let schemes = if swagger {
        root.get("securityDefinitions")
    } else {
        root.get("components")
            .and_then(|components| components.get("securitySchemes"))
    };
    let mut out = Vec::new();
    for (name, scheme) in schemes.map(|node| node.entries()).unwrap_or_default() {
        let kind_name = scheme.str("type").unwrap_or_default();
        let kind = match kind_name {
            "basic" => SchemeKind::Basic,
            "http" => match scheme
                .str("scheme")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .as_str()
            {
                "basic" => SchemeKind::Basic,
                "bearer" => SchemeKind::Bearer,
                other => SchemeKind::Other(format!("http {other}")),
            },
            "oauth2" | "openIdConnect" => SchemeKind::Bearer,
            "apiKey" => SchemeKind::ApiKey {
                name: scheme.str("name").unwrap_or_default().to_string(),
                location: match scheme.str("in") {
                    Some("query") => ParameterLocation::Query,
                    Some("cookie") => ParameterLocation::Cookie,
                    _ => ParameterLocation::Header,
                },
            },
            other => SchemeKind::Other(other.to_string()),
        };
        if kind_name == "oauth2" || kind_name == "openIdConnect" {
            scheme.warn(format!(
                "security scheme {name} ({kind_name}) imported as a bearer token, get the token yourself"
            ));
        }
        out.push(SecurityScheme {
            name: name.to_string(),
            kind,
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn read_operation(
    method: &str,
    path: &str,
    item: Node<'_>,
    operation: Node<'_>,
    global_security: Option<Node<'_>>,
    global_consumes: &[String],
    swagger: bool,
) -> Operation {
    let mut parameters = Vec::new();
    let mut body = None;
    let mut form = Map::new();
    let mut body_parameter = None;
    let path_level = item
        .get("parameters")
        .map(|list| list.items())
        .unwrap_or_default();
    let own = operation
        .get("parameters")
        .map(|list| list.items())
        .unwrap_or_default();
    // Operation parameters override path item parameters with the same name and location.
    let mut merged: Vec<Node<'_>> = Vec::new();
    for parameter in path_level.into_iter().chain(own) {
        let key = (parameter.str("name"), parameter.str("in"));
        merged.retain(|known| (known.str("name"), known.str("in")) != key);
        merged.push(parameter);
    }
    for parameter in merged {
        let name = parameter.str("name").unwrap_or_default().to_string();
        let location = match parameter.str("in") {
            Some("path") => ParameterLocation::Path,
            Some("query") => ParameterLocation::Query,
            Some("header") => ParameterLocation::Header,
            Some("cookie") => ParameterLocation::Cookie,
            Some("body") if swagger => {
                body_parameter = Some(parameter);
                continue;
            }
            Some("formData") if swagger => {
                let example = parameter_example(parameter).unwrap_or_default();
                form.insert(name, Value::String(example));
                continue;
            }
            _ => continue,
        };
        // OpenAPI ignores these as parameters: the body type and the auth set them.
        if location == ParameterLocation::Header
            && ["accept", "content-type", "authorization"]
                .contains(&name.to_ascii_lowercase().as_str())
        {
            continue;
        }
        parameters.push(Parameter {
            required: location == ParameterLocation::Path || parameter.flag("required"),
            description: parameter
                .str("description")
                .unwrap_or_default()
                .trim()
                .to_string(),
            example: parameter_example(parameter),
            name,
            location,
        });
    }

    if swagger {
        let consumes = {
            let own = string_list(operation.get("consumes"));
            if own.is_empty() {
                global_consumes.to_vec()
            } else {
                own
            }
        };
        if let Some(parameter) = body_parameter {
            body = Some(RequestBody {
                media_type: preferred_media_type(&consumes)
                    .unwrap_or_else(|| "application/json".to_string()),
                example: media_example(parameter),
            });
        } else if !form.is_empty() {
            let multipart = consumes.iter().any(|media| media.starts_with("multipart/"));
            body = Some(RequestBody {
                media_type: if multipart {
                    "multipart/form-data"
                } else {
                    "application/x-www-form-urlencoded"
                }
                .to_string(),
                example: Some(Value::Object(form)),
            });
        }
    } else if let Some(content) = operation
        .get("requestBody")
        .and_then(|body| body.get("content"))
    {
        let entries = content.entries();
        let names: Vec<String> = entries.iter().map(|(name, _)| name.to_string()).collect();
        if let Some(media_type) = preferred_media_type(&names)
            && let Some((_, media)) = entries.iter().find(|(name, _)| *name == media_type)
        {
            body = Some(RequestBody {
                example: media_example(*media),
                media_type,
            });
        }
    }

    let security = operation
        .get("security")
        .or(global_security)
        .and_then(|requirements| requirements.items().into_iter().next())
        .map(|first| {
            first
                .entries()
                .into_iter()
                .map(|(name, _)| name.to_string())
                .collect()
        })
        .unwrap_or_default();

    Operation {
        method: Method::from_str(&method.to_ascii_uppercase()).unwrap_or_default(),
        path: path.to_string(),
        operation_id: operation.str("operationId").unwrap_or_default().to_string(),
        summary: operation.str("summary").unwrap_or_default().to_string(),
        description: operation
            .str("description")
            .unwrap_or_default()
            .trim()
            .to_string(),
        tags: string_list(operation.get("tags")),
        deprecated: operation.flag("deprecated"),
        parameters,
        body,
        success: success(operation),
        security,
    }
}

/// The example of a parameter: its own `example`/`examples`, or its schema's (Swagger 2 keeps the
/// schema keywords on the parameter itself).
fn parameter_example(parameter: Node<'_>) -> Option<String> {
    if let Some(example) = parameter.get("example") {
        return Some(scalar_text(example.value()));
    }
    if let Some(value) = parameter.get("examples").and_then(|examples| {
        examples
            .entries()
            .into_iter()
            .find_map(|(_, example)| example.get("value"))
    }) {
        return Some(scalar_text(value.value()));
    }
    let schema = parameter.raw("schema").unwrap_or(parameter);
    explicit_example(schema).map(|value| scalar_text(&value))
}

/// The first 2xx response documented, in document order.
fn success(operation: Node<'_>) -> Option<Success> {
    let responses = operation.get("responses")?;
    responses.raw_entries().into_iter().find_map(|(code, _)| {
        let code = code.trim();
        if code.eq_ignore_ascii_case("2XX") {
            Some(Success::AnySuccess)
        } else {
            code.parse::<u16>()
                .ok()
                .filter(|status| (200..300).contains(status))
                .map(Success::Status)
        }
    })
}

/// The media type to use among `types`: JSON first, then forms, XML, text, else the first.
fn preferred_media_type(types: &[String]) -> Option<String> {
    let base = |media: &String| {
        media
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
    };
    let checks: [&dyn Fn(&str) -> bool; 5] = [
        &|media| media == "application/json",
        &|media| media.ends_with("+json") || media.ends_with("/json"),
        &|media| media == "application/x-www-form-urlencoded" || media == "multipart/form-data",
        &|media| media.ends_with("/xml") || media.ends_with("+xml"),
        &|media| media.starts_with("text/"),
    ];
    for check in checks {
        if let Some(found) = types.iter().find(|media| check(&base(media))) {
            return Some(base(found));
        }
    }
    types.first().map(base)
}

/// A list of strings (`tags`, `consumes`, `schemes`), empty when absent.
fn string_list(node: Option<Node<'_>>) -> Vec<String> {
    node.map(|list| {
        list.items()
            .into_iter()
            .filter_map(|item| item.value().as_str().map(str::to_string))
            .collect()
    })
    .unwrap_or_default()
}

/// A scalar as text: strings as they are, numbers and booleans written out, a list of scalars
/// joined with commas, anything else as compact JSON.
pub(crate) fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Array(items)
            if items
                .iter()
                .all(|item| !item.is_object() && !item.is_array()) =>
        {
            items.iter().map(scalar_text).collect::<Vec<_>>().join(",")
        }
        other => other.to_string(),
    }
}

fn is_http(location: &str) -> bool {
    location.starts_with("http://") || location.starts_with("https://")
}

/// `scheme://host[:port]` of an absolute URL.
fn origin(url: &str) -> &str {
    let Some((_, rest)) = url.split_once("://") else {
        return url;
    };
    let end = rest
        .find(['/', '?', '#'])
        .map_or(url.len(), |index| url.len() - rest.len() + index);
    &url[..end]
}

/// Resolves a possibly relative server URL against the spec's location, when that is a URL.
fn resolve_url(location: &str, url: &str) -> String {
    if url.contains("://") || !is_http(location) {
        return url.trim_end_matches('/').to_string();
    }
    let resolved = if let Some(rest) = url.strip_prefix("//") {
        let scheme = location.split("://").next().unwrap_or("https");
        format!("{scheme}://{rest}")
    } else if url.starts_with('/') {
        format!("{}{url}", origin(location))
    } else {
        let directory = location
            .rsplit_once('/')
            .map_or(location, |(directory, _)| directory);
        format!("{directory}/{url}")
    };
    resolved.trim_end_matches('/').to_string()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::super::document::NoExternalRefs;
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn parse(text: &str) -> Spec {
        parse_spec(text, "", &NoExternalRefs).expect("a valid spec")
    }

    #[test]
    fn rejects_documents_that_are_not_specs() {
        assert_eq!(
            parse_spec("a: 1", "", &NoExternalRefs),
            Err(OpenApiError::NotOpenApi)
        );
        assert_eq!(
            parse_spec("openapi: 4.0.0", "", &NoExternalRefs),
            Err(OpenApiError::UnsupportedVersion("4.0.0".to_string()))
        );
        assert_eq!(
            parse_spec("swagger: '1.2'", "", &NoExternalRefs),
            Err(OpenApiError::UnsupportedVersion("1.2".to_string()))
        );
    }

    #[test]
    fn reads_an_openapi_3_operation() {
        let spec = parse(
            r##"
openapi: 3.1.0
info: {title: ' Acme ', version: 2.0.0}
servers:
  - url: https://{region}.acme.dev/v2/
    description: Production
    variables: {region: {default: eu}}
security: [{bearerAuth: []}]
components:
  securitySchemes:
    bearerAuth: {type: http, scheme: bearer}
    key: {type: apiKey, in: query, name: api_key}
  parameters:
    Id: {name: id, in: path, required: true, schema: {type: string, example: cus_1}}
paths:
  /customers/{id}:
    parameters: [{$ref: '#/components/parameters/Id'}]
    patch:
      operationId: updateCustomer
      tags: [Customers]
      parameters:
        - {name: expand, in: query, schema: {type: array, items: {type: string}}, example: [a, b]}
        - {name: X-Trace, in: header, required: true}
        - {name: Accept, in: header}
      requestBody:
        content:
          text/plain: {}
          application/merge-patch+json:
            schema: {properties: {name: {type: string}}}
      responses:
        '404': {description: no}
        '200': {description: ok}
      security: []
"##,
        );
        assert_eq!(spec.title, "Acme");
        assert_eq!(spec.api_version, "2.0.0");
        assert_eq!(spec.spec_version, "3.1.0");
        assert_eq!(
            spec.servers,
            vec![Server {
                url: "https://eu.acme.dev/v2".to_string(),
                description: "Production".to_string()
            }]
        );
        assert_eq!(spec.security_schemes.len(), 2);
        assert_eq!(
            spec.security_schemes[1].kind,
            SchemeKind::ApiKey {
                name: "api_key".to_string(),
                location: ParameterLocation::Query
            }
        );
        let operation = &spec.operations[0];
        assert_eq!(operation.method, Method::Patch);
        assert_eq!(operation.display_name(), "updateCustomer");
        assert_eq!(operation.tags, vec!["Customers".to_string()]);
        let names: Vec<_> = operation
            .parameters
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, vec!["id", "expand", "X-Trace"]);
        assert_eq!(operation.parameters[0].example.as_deref(), Some("cus_1"));
        assert_eq!(operation.parameters[1].example.as_deref(), Some("a,b"));
        assert!(!operation.parameters[1].required);
        let body = operation.body.as_ref().expect("a body");
        assert_eq!(body.media_type, "application/merge-patch+json");
        assert_eq!(body.example, Some(json!({"name": "string"})));
        assert_eq!(operation.success, Some(Success::Status(200)));
        // `security: []` on the operation overrides the global requirement.
        assert!(operation.security.is_empty());
        assert!(spec.warnings.is_empty(), "{:?}", spec.warnings);
    }

    #[test]
    fn global_security_applies_and_2xx_ranges_count() {
        let spec = parse(
            "openapi: 3.0.3\ninfo: {title: T, version: '1'}\nsecurity: [{a: [], b: []}]\npaths:\n  /x:\n    get:\n      responses: {2XX: {description: ok}}\n",
        );
        assert_eq!(
            spec.operations[0].security,
            vec!["a".to_string(), "b".to_string()]
        );
        assert_eq!(spec.operations[0].success, Some(Success::AnySuccess));
        assert_eq!(spec.operations[0].display_name(), "GET /x");
    }

    #[test]
    fn relative_servers_resolve_against_the_spec_url() {
        let text =
            "openapi: 3.0.0\ninfo: {title: T, version: '1'}\nservers: [{url: /v2}]\npaths: {}\n";
        let spec = parse_spec(
            text,
            "https://api.acme.dev/openapi/v2.yaml",
            &NoExternalRefs,
        )
        .expect("valid");
        assert_eq!(spec.servers[0].url, "https://api.acme.dev/v2");
        let text = "openapi: 3.0.0\ninfo: {title: T, version: '1'}\npaths: {}\n";
        let spec =
            parse_spec(text, "http://localhost:8080/spec.json", &NoExternalRefs).expect("valid");
        assert_eq!(spec.servers[0].url, "http://localhost:8080");
        assert_eq!(
            spec.warnings,
            vec!["the spec has no operations".to_string()]
        );
    }

    #[test]
    fn reads_a_swagger_2_spec() {
        let spec = parse(
            r##"{
  "swagger": "2.0",
  "info": {"title": "Pets", "version": "1.0"},
  "host": "petstore.test",
  "basePath": "/v1/",
  "schemes": ["http", "https"],
  "consumes": ["application/json"],
  "securityDefinitions": {"basic": {"type": "basic"}, "oauth": {"type": "oauth2"}},
  "definitions": {"Pet": {"type": "object", "properties": {"name": {"type": "string", "example": "Rex"}}}},
  "paths": {
    "/pets": {
      "post": {
        "parameters": [
          {"in": "body", "name": "pet", "schema": {"$ref": "#/definitions/Pet"}},
          {"in": "query", "name": "dry", "type": "boolean", "default": false}
        ],
        "responses": {"201": {"description": "created"}}
      },
      "put": {
        "consumes": ["multipart/form-data"],
        "parameters": [{"in": "formData", "name": "photo", "type": "file"}, {"in": "formData", "name": "tag", "type": "string", "x-example": "a"}],
        "responses": {"default": {"description": "x"}}
      }
    }
  }
}"##,
        );
        assert_eq!(spec.spec_version, "2.0");
        let urls: Vec<_> = spec
            .servers
            .iter()
            .map(|server| server.url.as_str())
            .collect();
        assert_eq!(urls, vec!["https://petstore.test/v1"]);
        assert_eq!(spec.security_schemes[0].kind, SchemeKind::Basic);
        assert_eq!(spec.security_schemes[1].kind, SchemeKind::Bearer);
        assert_eq!(spec.warnings.len(), 1, "the oauth2 warning");
        let post = spec
            .operations
            .iter()
            .find(|op| op.method == Method::Post)
            .expect("post");
        assert_eq!(
            post.body.as_ref().map(|body| body.example.clone()),
            Some(Some(json!({"name": "Rex"})))
        );
        assert_eq!(post.parameters[0].example.as_deref(), Some("false"));
        assert_eq!(post.success, Some(Success::Status(201)));
        let put = spec
            .operations
            .iter()
            .find(|op| op.method == Method::Put)
            .expect("put");
        let body = put.body.as_ref().expect("a form");
        assert_eq!(body.media_type, "multipart/form-data");
        assert_eq!(body.example, Some(json!({"photo": "", "tag": "a"})));
        assert_eq!(put.success, None);
    }
}
