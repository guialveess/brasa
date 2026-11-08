//! Turning parsed [`Spec`]s and the import options into an [`OpenApiPlan`]: the folders,
//! requests and environments to write. The mapping is described in `docs/openapi-import.md`.

use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, Method, Request};
use serde_json::Value;

use super::spec::{Operation, ParameterLocation, SchemeKind, Spec, Success, scalar_text};
use crate::postman::ImportFolder;

/// The longest server description used as an environment name, longer ones use the host.
const MAX_ENVIRONMENT_LABEL: usize = 32;

/// The value of `baseUrl` when the spec gives no absolute server URL.
const FALLBACK_BASE_URL: &str = "http://localhost";

/// The name of the environment variable that holds the server origin.
const BASE_URL_VARIABLE: &str = "baseUrl";

/// How requests are grouped into folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Grouping {
    /// One folder per path segment: `/customers/{id}` goes in `customers/{id}`.
    #[default]
    PathSegments,
    /// One folder per operation's first tag; untagged operations stay at the top.
    Tags,
}

/// What an import generates besides the requests themselves (the wizard's "Options" step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportOptions {
    /// Environments with `baseUrl` from the servers, one per server origin.
    pub environments: bool,
    /// Request auth and its variables from the security schemes, secrets in `.local.env`.
    pub auth: bool,
    /// Bodies from the spec's examples, or built from the schema.
    pub bodies: bool,
    /// A `test()` asserting the first documented 2xx status.
    pub tests: bool,
    /// The docs section from the summary, description and parameters.
    pub docs: bool,
    /// How requests are grouped into folders.
    pub grouping: Grouping,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            environments: true,
            auth: true,
            bodies: true,
            tests: true,
            docs: true,
            grouping: Grouping::default(),
        }
    }
}

/// One spec of an import, with the folder its requests go in when several versions are imported
/// together (`v1`, `v2`). An empty label puts the requests directly in the collection.
#[derive(Debug, Clone, Copy)]
pub struct SpecVersion<'a> {
    /// The version folder name, empty for none.
    pub label: &'a str,
    /// The spec.
    pub spec: &'a Spec,
}

/// Everything an OpenAPI import writes, before the user's selection is applied.
#[derive(Debug, Clone, PartialEq)]
pub struct OpenApiPlan {
    /// The collection's folders and requests. Its own name is the collection name.
    pub root: PlanFolder,
    /// The environments to write.
    pub environments: Vec<PlanEnvironment>,
    /// Everything that could not be mapped faithfully, the specs' own warnings included.
    pub warnings: Vec<String>,
}

/// A folder of an [`OpenApiPlan`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlanFolder {
    /// The folder name.
    pub name: String,
    /// Nested folders, in the order they were first needed.
    pub folders: Vec<PlanFolder>,
    /// Requests directly in this folder, in spec order.
    pub requests: Vec<PlanRequest>,
}

/// A request of an [`OpenApiPlan`].
#[derive(Debug, Clone, PartialEq)]
pub struct PlanRequest {
    /// Identifies the operation across rebuilds of the plan: `[<version label> ]<METHOD> <path>`.
    pub key: String,
    /// The request (file) name.
    pub name: String,
    /// The spec path (`/customers/{id}`), for display.
    pub path: String,
    /// The request.
    pub request: Request,
}

/// An environment of an [`OpenApiPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEnvironment {
    /// The environment name.
    pub name: String,
    /// The variables of `<name>.env`.
    pub base: Vec<KeyValue>,
    /// The variables of `<name>.local.env`, the secrets, always empty values.
    pub secret: Vec<KeyValue>,
}

impl PlanFolder {
    /// Every request key under this folder, depth first.
    pub fn keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_keys(&mut out);
        out
    }

    fn collect_keys(&self, out: &mut Vec<String>) {
        out.extend(self.requests.iter().map(|request| request.key.clone()));
        self.folders
            .iter()
            .for_each(|folder| folder.collect_keys(out));
    }

    /// How many folders there are under this one, at any depth.
    pub fn folder_count(&self) -> usize {
        self.folders
            .iter()
            .map(|folder| 1 + folder.folder_count())
            .sum()
    }

    /// The tree to write, with only the requests `keep` accepts. Folders left empty are dropped.
    pub fn to_import_folder(&self, keep: &dyn Fn(&str) -> bool) -> ImportFolder {
        ImportFolder {
            name: self.name.clone(),
            folders: self
                .folders
                .iter()
                .map(|folder| folder.to_import_folder(keep))
                .filter(|folder| !folder.folders.is_empty() || !folder.requests.is_empty())
                .collect(),
            requests: self
                .requests
                .iter()
                .filter(|request| keep(&request.key))
                .map(|request| (request.name.clone(), request.request.clone()))
                .collect(),
            issues: Vec::new(),
        }
    }

    fn child(&mut self, name: &str) -> &mut PlanFolder {
        let index = match self.folders.iter().position(|folder| folder.name == name) {
            Some(index) => index,
            None => {
                self.folders.push(PlanFolder {
                    name: name.to_string(),
                    ..PlanFolder::default()
                });
                self.folders.len() - 1
            }
        };
        &mut self.folders[index]
    }
}

/// The key of an operation, as in [`PlanRequest::key`].
fn operation_key(label: &str, operation: &Operation) -> String {
    let key = format!("{} {}", operation.method, operation.path);
    if label.is_empty() {
        key
    } else {
        format!("{label} {key}")
    }
}

/// Builds the plan of an import of `specs` into a collection named `collection_name`.
pub fn build_plan(
    collection_name: &str,
    specs: &[SpecVersion<'_>],
    options: &ImportOptions,
) -> OpenApiPlan {
    let mut warnings = Vec::new();
    for version in specs {
        warnings.extend(version.spec.warnings.iter().cloned());
    }

    // The origins of every server, in order: one environment each, `baseUrl` is the origin and
    // the server's path stays in the request URLs.
    let mut origins: Vec<(String, String)> = Vec::new();
    for version in specs {
        for server in &version.spec.servers {
            let (origin, _) = split_server(&server.url);
            if origin.is_empty() {
                continue;
            }
            if !origins.iter().any(|(known, _)| *known == origin) {
                let mut label = environment_label(origin, &server.description);
                if origins.iter().any(|(_, known)| *known == label) {
                    label = origin_host(origin).to_string();
                }
                origins.push((origin.to_string(), label));
            }
        }
    }
    if options.environments && origins.is_empty() {
        warnings.push(format!(
            "no absolute server URL, {BASE_URL_VARIABLE} is set to {FALLBACK_BASE_URL}"
        ));
    }

    let mut root = PlanFolder {
        name: collection_name.to_string(),
        ..PlanFolder::default()
    };
    let mut secrets: Vec<String> = Vec::new();
    for version in specs {
        let spec = version.spec;
        let first = spec
            .servers
            .first()
            .map(|server| server.url.as_str())
            .unwrap_or_default();
        let (first_origin, base_path) = split_server(first);
        if spec
            .servers
            .iter()
            .any(|server| split_server(&server.url).1 != base_path)
        {
            warnings.push(format!(
                "{}: the servers have different paths, {base_path:?} is used",
                spec.title
            ));
        }
        let prefix = if options.environments {
            format!("{{{{{BASE_URL_VARIABLE}}}}}{base_path}")
        } else if first_origin.is_empty() {
            base_path.to_string()
        } else {
            format!("{first_origin}{base_path}")
        };
        for operation in &spec.operations {
            let request = build_request(
                &prefix,
                spec,
                operation,
                options,
                &mut secrets,
                &mut warnings,
            );
            let mut folder = &mut root;
            if !version.label.is_empty() {
                folder = folder.child(version.label);
            }
            match options.grouping {
                Grouping::PathSegments => {
                    for segment in operation
                        .path
                        .split('/')
                        .filter(|segment| !segment.is_empty())
                    {
                        folder = folder.child(segment);
                    }
                }
                Grouping::Tags => {
                    if let Some(tag) = operation.tags.first().filter(|tag| !tag.trim().is_empty()) {
                        folder = folder.child(tag.trim());
                    }
                }
            }
            folder.requests.push(PlanRequest {
                key: operation_key(version.label, operation),
                name: operation.display_name(),
                path: operation.path.clone(),
                request,
            });
        }
    }

    let secret: Vec<KeyValue> = secrets.iter().map(|name| KeyValue::new(name, "")).collect();
    let mut environments = Vec::new();
    if options.environments {
        if origins.is_empty() {
            origins.push((FALLBACK_BASE_URL.to_string(), String::new()));
        }
        let several = origins.len() > 1;
        for (origin, label) in origins {
            environments.push(PlanEnvironment {
                name: if several {
                    label
                } else {
                    collection_name.to_string()
                },
                base: vec![KeyValue::new(BASE_URL_VARIABLE, origin)],
                secret: secret.clone(),
            });
        }
    } else if !secret.is_empty() {
        environments.push(PlanEnvironment {
            name: collection_name.to_string(),
            base: Vec::new(),
            secret,
        });
    }

    OpenApiPlan {
        root,
        environments,
        warnings,
    }
}

/// Splits an absolute server URL into its origin (`https://api.acme.dev`) and its path (`/v2`).
/// A relative URL has an empty origin.
fn split_server(url: &str) -> (&str, &str) {
    let url = url.trim_end_matches('/');
    let Some((_, rest)) = url.split_once("://") else {
        return ("", url);
    };
    match rest.find('/') {
        Some(index) => url.split_at(url.len() - rest.len() + index),
        None => (url, ""),
    }
}

/// The part of an environment name that tells servers apart: the server description when it is
/// short, the host otherwise.
fn environment_label(origin: &str, description: &str) -> String {
    let description = description.trim();
    if !description.is_empty() && description.chars().count() <= MAX_ENVIRONMENT_LABEL {
        return description.to_string();
    }
    origin_host(origin).to_string()
}

/// The host of a server origin: `https://api.acme.dev` gives `api.acme.dev`.
fn origin_host(origin: &str) -> &str {
    origin.split_once("://").map_or(origin, |(_, host)| host)
}

/// A security scheme name made into a variable name: letters, digits and `_` only.
fn variable_name(scheme: &str) -> String {
    let name: String = scheme
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if name.is_empty() {
        "token".to_string()
    } else {
        name
    }
}

fn build_request(
    prefix: &str,
    spec: &Spec,
    operation: &Operation,
    options: &ImportOptions,
    secrets: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Request {
    let mut request = Request {
        method: operation.method.clone(),
        url: format!("{prefix}{}", template_path(&operation.path)),
        ..Request::default()
    };
    let mut cookies = Vec::new();
    for parameter in &operation.parameters {
        let value = parameter.example.clone().unwrap_or_default();
        let entry = KeyValue {
            key: parameter.name.clone(),
            value,
            enabled: parameter.required,
        };
        match parameter.location {
            ParameterLocation::Path => {}
            ParameterLocation::Query => request.query.push(entry),
            ParameterLocation::Header => request.headers.push(entry),
            ParameterLocation::Cookie => cookies.push(entry),
        }
    }
    if !cookies.is_empty() {
        request.headers.push(KeyValue {
            key: "Cookie".to_string(),
            value: cookies
                .iter()
                .map(|cookie| format!("{}={}", cookie.key, cookie.value))
                .collect::<Vec<_>>()
                .join("; "),
            enabled: cookies.iter().any(|cookie| cookie.enabled),
        });
    }

    if options.auth {
        apply_security(spec, operation, &mut request, secrets, warnings);
    }
    if let Some(body) = &operation.body {
        let (value, content_type) =
            build_body(&body.media_type, body.example.as_ref(), options.bodies);
        // Some specs declare an empty body on GET operations; a GET without one is the request.
        let empty = match &value {
            Body::Json(text) | Body::Text(text) | Body::Xml(text) => text.is_empty(),
            Body::Form(fields) | Body::Multipart(fields) => fields.is_empty(),
            Body::None => true,
        };
        if !(empty && matches!(operation.method, Method::Get | Method::Head)) {
            if let Some(content_type) = content_type {
                request
                    .headers
                    .push(KeyValue::new("Content-Type", content_type));
            }
            request.body = value;
        }
    }
    if options.tests {
        request.post_script = match operation.success {
            Some(Success::Status(status)) => format!(
                "test(\"status is {status}\", () => {{\n  expect(res.status).toBe({status});\n}});"
            ),
            Some(Success::AnySuccess) => "test(\"status is 2xx\", () => {\n  expect(res.status >= 200 && res.status < 300).toBe(true);\n});".to_string(),
            None => String::new(),
        };
    }
    if options.docs {
        request.docs = build_docs(operation);
    }
    request
}

/// `/customers/{id}` as `/customers/{{id}}`: a path parameter becomes a variable.
fn template_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len() + 8);
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        let Some(length) = rest[start..].find('}') else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str("{{");
        out.push_str(rest[start + 1..start + length].trim());
        out.push_str("}}");
        rest = &rest[start + length + 1..];
    }
    out.push_str(rest);
    out
}

fn apply_security(
    spec: &Spec,
    operation: &Operation,
    request: &mut Request,
    secrets: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    for name in &operation.security {
        let Some(scheme) = spec
            .security_schemes
            .iter()
            .find(|scheme| &scheme.name == name)
        else {
            push_unique(
                warnings,
                format!("{}: unknown security scheme {name}", spec.title),
            );
            continue;
        };
        let variable = variable_name(&scheme.name);
        let auth_free = request.auth == Auth::None;
        match &scheme.kind {
            SchemeKind::Basic if auth_free => {
                let username = format!("{variable}Username");
                let password = format!("{variable}Password");
                request.auth = Auth::Basic {
                    username: format!("{{{{{username}}}}}"),
                    password: format!("{{{{{password}}}}}"),
                };
                push_unique(secrets, username);
                push_unique(secrets, password);
            }
            SchemeKind::Bearer if auth_free => {
                request.auth = Auth::Bearer {
                    token: format!("{{{{{variable}}}}}"),
                };
                push_unique(secrets, variable);
            }
            SchemeKind::ApiKey { name, location } => {
                let value = format!("{{{{{variable}}}}}");
                match location {
                    ParameterLocation::Cookie => {
                        request
                            .headers
                            .push(KeyValue::new("Cookie", format!("{name}={value}")));
                    }
                    _ if auth_free => {
                        request.auth = Auth::ApiKey {
                            name: name.clone(),
                            value,
                            location: if *location == ParameterLocation::Query {
                                ApiKeyLocation::Query
                            } else {
                                ApiKeyLocation::Header
                            },
                        };
                    }
                    // The request already has its auth: a second key goes in as a plain entry.
                    ParameterLocation::Query => request.query.push(KeyValue::new(name, value)),
                    _ => request.headers.push(KeyValue::new(name, value)),
                }
                push_unique(secrets, variable);
            }
            SchemeKind::Other(kind) => push_unique(
                warnings,
                format!(
                    "{}: security scheme {name} ({kind}) is not imported",
                    spec.title
                ),
            ),
            SchemeKind::Basic | SchemeKind::Bearer => push_unique(
                warnings,
                format!(
                    "{}: operations needing several credentials only get the first, {name} is not set",
                    spec.title
                ),
            ),
        }
    }
}

/// The body for a media type, and the `Content-Type` header to add when the body type's default
/// is not the right one. `fill` puts the example in; without it the body is empty.
fn build_body(media_type: &str, example: Option<&Value>, fill: bool) -> (Body, Option<String>) {
    let example = if fill { example } else { None };
    let header = |default: &str| (media_type != default).then(|| media_type.to_string());
    if media_type.ends_with("/json") || media_type.ends_with("+json") {
        let text = match example {
            Some(Value::String(text)) => text.clone(),
            Some(value) => serde_json::to_string_pretty(value).unwrap_or_default(),
            None => String::new(),
        };
        (Body::Json(text), header("application/json"))
    } else if media_type == "application/x-www-form-urlencoded"
        || media_type == "multipart/form-data"
    {
        let fields = match example {
            Some(Value::Object(map)) => map
                .iter()
                .map(|(key, value)| KeyValue::new(key, scalar_text(value)))
                .collect(),
            _ => Vec::new(),
        };
        if media_type == "multipart/form-data" {
            (Body::Multipart(fields), None)
        } else {
            (Body::Form(fields), None)
        }
    } else if media_type.ends_with("/xml") || media_type.ends_with("+xml") {
        let text = example
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        (Body::Xml(text), header("application/xml"))
    } else {
        let text = match example {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => String::new(),
            Some(value) => serde_json::to_string_pretty(value).unwrap_or_default(),
        };
        (Body::Text(text), Some(media_type.to_string()))
    }
}

fn build_docs(operation: &Operation) -> String {
    let mut parts = Vec::new();
    if !operation.summary.trim().is_empty() {
        parts.push(format!("# {}", operation.summary.trim()));
    }
    if operation.deprecated {
        parts.push("**Deprecated.**".to_string());
    }
    if !operation.description.is_empty() {
        parts.push(operation.description.clone());
    }
    if !operation.parameters.is_empty() {
        let lines: Vec<String> = operation
            .parameters
            .iter()
            .map(|parameter| {
                let location = match parameter.location {
                    ParameterLocation::Path => "path",
                    ParameterLocation::Query => "query",
                    ParameterLocation::Header => "header",
                    ParameterLocation::Cookie => "cookie",
                };
                let required = if parameter.required { ", required" } else { "" };
                let description = if parameter.description.is_empty() {
                    String::new()
                } else {
                    format!(": {}", parameter.description.replace('\n', " "))
                };
                format!("- `{}` ({location}{required}){description}", parameter.name)
            })
            .collect();
        parts.push(format!("## Parameters\n\n{}", lines.join("\n")));
    }
    parts.join("\n\n")
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::super::document::NoExternalRefs;
    use super::super::spec::parse_spec;
    use super::*;
    use pretty_assertions::assert_eq;

    const V1: &str = r##"
openapi: 3.0.3
info: {title: Acme Payments API, version: 1.0.0}
servers:
  - {url: https://api.acme.dev/v1, description: production}
  - {url: https://staging.acme.dev/v1, description: staging}
security: [{apiKey: []}]
components:
  securitySchemes:
    apiKey: {type: http, scheme: bearer}
paths:
  /customers:
    get:
      summary: List customers
      tags: [Customers]
      parameters:
        - {name: limit, in: query, description: "Page\nsize", schema: {type: integer, default: 10}}
        - {name: session, in: cookie, required: true}
      responses: {'200': {description: ok}}
    post:
      summary: Create customer
      tags: [Customers]
      requestBody:
        content:
          application/json:
            example: {name: Ada}
      responses: {'201': {description: created}}
  /customers/{id}:
    delete:
      operationId: deleteCustomer
      deprecated: true
      parameters: [{name: id, in: path, required: true}]
      responses: {'204': {description: gone}}
      security: []
"##;

    const V2: &str = r##"
openapi: 3.1.0
info: {title: Acme Payments API, version: 2.0.0}
servers: [{url: https://api.acme.dev/v2}]
components:
  securitySchemes:
    key: {type: apiKey, in: header, name: X-Api-Key}
    basic: {type: http, scheme: basic}
paths:
  /charges:
    post:
      summary: Create charge
      security: [{basic: [], key: []}]
      requestBody:
        content:
          application/x-www-form-urlencoded:
            schema: {properties: {amount: {type: integer, example: 5}, currency: {type: string}}}
      responses: {'2XX': {description: ok}}
"##;

    fn spec(text: &str) -> Spec {
        parse_spec(text, "", &NoExternalRefs).expect("valid")
    }

    #[test]
    fn builds_versions_folders_environments_and_requests() {
        let (v1, v2) = (spec(V1), spec(V2));
        let plan = build_plan(
            "Acme Payments API",
            &[
                SpecVersion {
                    label: "v1",
                    spec: &v1,
                },
                SpecVersion {
                    label: "v2",
                    spec: &v2,
                },
            ],
            &ImportOptions::default(),
        );
        assert_eq!(plan.root.name, "Acme Payments API");
        let names: Vec<_> = plan
            .root
            .folders
            .iter()
            .map(|folder| folder.name.as_str())
            .collect();
        assert_eq!(names, vec!["v1", "v2"]);
        let customers = &plan.root.folders[0].folders[0];
        assert_eq!(customers.name, "customers");
        assert_eq!(customers.requests.len(), 2);
        assert_eq!(customers.folders[0].name, "{id}");
        assert_eq!(plan.root.folder_count(), 5);
        assert_eq!(
            plan.root.keys(),
            vec![
                "v1 GET /customers",
                "v1 POST /customers",
                "v1 DELETE /customers/{id}",
                "v2 POST /charges"
            ]
        );

        let list = &customers.requests[0];
        assert_eq!(list.name, "List customers");
        let request = &list.request;
        assert_eq!(request.url, "{{baseUrl}}/v1/customers");
        assert_eq!(
            request.query,
            vec![KeyValue {
                key: "limit".into(),
                value: "10".into(),
                enabled: false
            }]
        );
        assert_eq!(request.headers, vec![KeyValue::new("Cookie", "session=")]);
        assert_eq!(
            request.auth,
            Auth::Bearer {
                token: "{{apiKey}}".into()
            }
        );
        assert_eq!(
            request.post_script,
            "test(\"status is 200\", () => {\n  expect(res.status).toBe(200);\n});"
        );
        assert_eq!(
            request.docs,
            "# List customers\n\n## Parameters\n\n- `limit` (query): Page size\n- `session` (cookie, required)"
        );

        let create = &customers.requests[1].request;
        assert_eq!(create.body, Body::Json("{\n  \"name\": \"Ada\"\n}".into()));

        let delete = &customers.folders[0].requests[0];
        assert_eq!(delete.name, "deleteCustomer");
        assert_eq!(delete.request.url, "{{baseUrl}}/v1/customers/{{id}}");
        assert_eq!(delete.request.auth, Auth::None);
        assert!(delete.request.docs.starts_with("**Deprecated.**"));

        let charge = &plan.root.folders[1].folders[0].requests[0].request;
        assert_eq!(
            charge.auth,
            Auth::Basic {
                username: "{{basicUsername}}".into(),
                password: "{{basicPassword}}".into()
            }
        );
        assert_eq!(charge.headers, vec![KeyValue::new("X-Api-Key", "{{key}}")]);
        assert_eq!(
            charge.body,
            Body::Form(vec![
                KeyValue::new("amount", "5"),
                KeyValue::new("currency", "string")
            ])
        );
        assert!(charge.post_script.contains("res.status >= 200"));

        let secrets: Vec<_> = ["apiKey", "basicUsername", "basicPassword", "key"]
            .iter()
            .map(|name| KeyValue::new(*name, ""))
            .collect();
        assert_eq!(
            plan.environments,
            vec![
                PlanEnvironment {
                    name: "production".into(),
                    base: vec![KeyValue::new("baseUrl", "https://api.acme.dev")],
                    secret: secrets.clone(),
                },
                PlanEnvironment {
                    name: "staging".into(),
                    base: vec![KeyValue::new("baseUrl", "https://staging.acme.dev")],
                    secret: secrets,
                },
            ]
        );
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    #[test]
    fn options_turn_parts_off() {
        let v1 = spec(V1);
        let options = ImportOptions {
            environments: false,
            auth: false,
            bodies: false,
            tests: false,
            docs: false,
            grouping: Grouping::Tags,
        };
        let plan = build_plan(
            "Acme",
            &[SpecVersion {
                label: "",
                spec: &v1,
            }],
            &options,
        );
        assert!(plan.environments.is_empty());
        let names: Vec<_> = plan
            .root
            .folders
            .iter()
            .map(|folder| folder.name.as_str())
            .collect();
        assert_eq!(names, vec!["Customers"]);
        // The untagged operation stays at the top of the collection.
        assert_eq!(plan.root.requests[0].name, "deleteCustomer");
        let create = &plan.root.folders[0].requests[1].request;
        assert_eq!(create.url, "https://api.acme.dev/v1/customers");
        assert_eq!(create.body, Body::Json(String::new()));
        assert_eq!(create.auth, Auth::None);
        assert!(create.post_script.is_empty() && create.docs.is_empty());
    }

    #[test]
    fn a_single_server_gives_one_environment_named_after_the_collection() {
        let v2 = spec(V2);
        let plan = build_plan(
            "Acme",
            &[SpecVersion {
                label: "",
                spec: &v2,
            }],
            &ImportOptions::default(),
        );
        let names: Vec<_> = plan
            .environments
            .iter()
            .map(|env| env.name.as_str())
            .collect();
        assert_eq!(names, vec!["Acme"]);
    }

    #[test]
    fn specs_without_servers_fall_back_to_localhost() {
        let bare = spec(
            "openapi: 3.0.0\ninfo: {title: T, version: '1'}\npaths:\n  /x:\n    get: {responses: {}}\n",
        );
        let plan = build_plan(
            "T",
            &[SpecVersion {
                label: "",
                spec: &bare,
            }],
            &ImportOptions::default(),
        );
        assert_eq!(
            plan.environments[0].base,
            vec![KeyValue::new("baseUrl", "http://localhost")]
        );
        assert_eq!(
            plan.root.folders[0].requests[0].request.url,
            "{{baseUrl}}/x"
        );
        assert_eq!(plan.warnings.len(), 1);
    }

    #[test]
    fn selection_drops_requests_and_empty_folders() {
        let v1 = spec(V1);
        let plan = build_plan(
            "Acme",
            &[SpecVersion {
                label: "",
                spec: &v1,
            }],
            &ImportOptions::default(),
        );
        let folder = plan.root.to_import_folder(&|key| key == "GET /customers");
        assert_eq!(folder.folders.len(), 1);
        assert_eq!(folder.folders[0].requests.len(), 1);
        assert!(
            folder.folders[0].folders.is_empty(),
            "the {{id}} folder had nothing selected"
        );
    }

    #[test]
    fn media_types_map_to_body_types() {
        let example = serde_json::json!({"a": 1});
        assert_eq!(
            build_body("application/problem+json", Some(&example), true)
                .1
                .as_deref(),
            Some("application/problem+json")
        );
        assert_eq!(
            build_body("application/xml", Some(&Value::from("<a/>")), true),
            (Body::Xml("<a/>".into()), None)
        );
        assert_eq!(
            build_body("application/octet-stream", None, true),
            (
                Body::Text(String::new()),
                Some("application/octet-stream".into())
            )
        );
        assert_eq!(template_path("/a/{ id }/b/{x}"), "/a/{{id}}/b/{{x}}");
    }

    #[test]
    fn empty_get_bodies_are_dropped() {
        let text = "openapi: 3.0.0\ninfo: {title: T, version: '1'}\npaths:\n  /x:\n    get:\n      requestBody: {content: {application/x-www-form-urlencoded: {schema: {type: object}}}}\n      responses: {}\n";
        let parsed = spec(text);
        let plan = build_plan(
            "T",
            &[SpecVersion {
                label: "",
                spec: &parsed,
            }],
            &ImportOptions::default(),
        );
        let request = &plan.root.folders[0].requests[0].request;
        assert_eq!(request.body, Body::None);
        assert!(request.headers.is_empty());
    }
}
