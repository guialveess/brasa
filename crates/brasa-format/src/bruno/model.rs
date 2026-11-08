//! What a Bruno collection holds, read from either of its formats (`.bru` or OpenCollection
//! YAML) into one shape, so the mapping to Brasa ([`super::convert`]) is written once.

use brasa_core::KeyValue;

use super::bru::{BruContent, BruFile, BruPair};

/// The authorization of a request, a folder or the collection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum RawAuth {
    /// None set.
    #[default]
    None,
    /// The folder's or the collection's.
    Inherit,
    /// HTTP Basic.
    Basic { username: String, password: String },
    /// A bearer token.
    Bearer { token: String },
    /// An API key in a header, or in the query with `query`.
    ApiKey {
        key: String,
        value: String,
        query: bool,
    },
    /// OAuth 2.0: Bruno fetches the token, which is not in the files.
    OAuth2,
    /// A kind Brasa does not have (`digest`, `ntlm`, `awsv4`, `wsse`, `oauth1`, ...).
    Other(String),
}

/// One multipart field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MultipartEntry {
    /// The field.
    pub(super) field: KeyValue,
    /// The files it uploads, `None` for a text field.
    pub(super) files: Option<Vec<String>>,
}

/// The body of a request.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum RawBody {
    /// No body.
    #[default]
    None,
    /// JSON text.
    Json(String),
    /// Plain text.
    Text(String),
    /// XML text.
    Xml(String),
    /// URL encoded form fields.
    Form(Vec<KeyValue>),
    /// Multipart fields.
    Multipart(Vec<MultipartEntry>),
    /// A GraphQL query and its variables (JSON text, possibly empty).
    GraphQl { query: String, variables: String },
    /// A kind Brasa does not have (`sparql`, `file`, ...).
    Other(String),
}

/// What a folder, or the collection, gives every request inside it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Defaults {
    /// Headers sent with every request.
    pub(super) headers: Vec<KeyValue>,
    /// The authorization requests inherit, `None` when it says nothing (inherit further up).
    pub(super) auth: Option<RawAuth>,
    /// Variables.
    pub(super) vars: Vec<KeyValue>,
    /// Whether it has a pre-request script, a post-response script or tests, which Brasa does
    /// not run at that level.
    pub(super) scripts: Vec<&'static str>,
    /// Its docs.
    pub(super) docs: String,
}

/// One request.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct RawRequest {
    /// Its name.
    pub(super) name: String,
    /// Its place in its folder.
    pub(super) seq: f64,
    /// `http` or `graphql`; anything else (`grpc`, `ws`) is skipped.
    pub(super) kind: String,
    /// The method, upper case.
    pub(super) method: String,
    /// The URL, query string included.
    pub(super) url: String,
    /// The query parameters.
    pub(super) query: Vec<KeyValue>,
    /// The path parameters (`:id` in the URL).
    pub(super) path: Vec<KeyValue>,
    /// The headers.
    pub(super) headers: Vec<KeyValue>,
    /// The body.
    pub(super) body: RawBody,
    /// The authorization.
    pub(super) auth: RawAuth,
    /// `vars:pre-request`.
    pub(super) vars_pre: Vec<KeyValue>,
    /// `vars:post-response`, a key starting with `@` is local to the request.
    pub(super) vars_post: Vec<KeyValue>,
    /// `assert`: the expression as key, `<operator> <value>` as value.
    pub(super) asserts: Vec<KeyValue>,
    /// `script:pre-request`.
    pub(super) pre_script: String,
    /// `script:post-response`.
    pub(super) post_script: String,
    /// `tests`.
    pub(super) tests: String,
    /// `docs`.
    pub(super) docs: String,
}

/// One folder.
#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct RawFolder {
    /// Its name.
    pub(super) name: String,
    /// Its place in its parent.
    pub(super) seq: f64,
    /// What it gives its requests.
    pub(super) defaults: Defaults,
    /// Its subfolders.
    pub(super) folders: Vec<RawFolder>,
    /// Its requests.
    pub(super) requests: Vec<RawRequest>,
}

/// One environment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct RawEnvironment {
    /// Its name.
    pub(super) name: String,
    /// Its variables, disabled ones included.
    pub(super) vars: Vec<KeyValue>,
    /// The names of its secret variables, whose values are not in the files.
    pub(super) secrets: Vec<String>,
    /// Its color as written (`#4caf50`).
    pub(super) color: Option<String>,
    /// The environment it extends.
    pub(super) extends: Option<String>,
}

impl From<&BruPair> for KeyValue {
    fn from(pair: &BruPair) -> Self {
        KeyValue {
            key: pair.key.clone(),
            value: pair.value.clone(),
            enabled: pair.enabled,
        }
    }
}

fn entries(pairs: &[BruPair]) -> Vec<KeyValue> {
    pairs.iter().map(KeyValue::from).collect()
}

/// The HTTP method blocks of a `.bru` request.
const METHOD_BLOCKS: [&str; 10] = [
    "get", "post", "put", "delete", "patch", "options", "head", "connect", "trace", "http",
];

/// Reads the authorization block selected by `mode`.
fn bru_auth(file: &BruFile, mode: &str) -> RawAuth {
    let block = format!("auth:{mode}");
    let get = |key: &str| file.value(&block, key).unwrap_or_default().to_string();
    match mode {
        "" | "none" => RawAuth::None,
        "inherit" => RawAuth::Inherit,
        "basic" => RawAuth::Basic {
            username: get("username"),
            password: get("password"),
        },
        "bearer" => RawAuth::Bearer {
            token: get("token"),
        },
        "apikey" => RawAuth::ApiKey {
            key: get("key"),
            value: get("value"),
            query: get("placement") == "queryparams",
        },
        "oauth2" => RawAuth::OAuth2,
        other => RawAuth::Other(other.to_string()),
    }
}

/// Reads the body selected by `mode`.
fn bru_body(file: &BruFile, mode: &str) -> RawBody {
    let text = |block: &str| file.text(block).unwrap_or_default().to_string();
    match mode {
        "" | "none" => RawBody::None,
        "json" => RawBody::Json(text("body:json")),
        "text" => RawBody::Text(text("body:text")),
        "xml" => RawBody::Xml(text("body:xml")),
        "formUrlEncoded" | "form-urlencoded" => {
            RawBody::Form(entries(file.dict("body:form-urlencoded")))
        }
        "multipartForm" | "multipart-form" => RawBody::Multipart(
            file.dict("body:multipart-form")
                .iter()
                .map(multipart_entry)
                .collect(),
        ),
        "graphql" => RawBody::GraphQl {
            query: text("body:graphql"),
            variables: text("body:graphql:vars"),
        },
        other => RawBody::Other(other.to_string()),
    }
}

/// A multipart field: `@file(a|b)` uploads files, an `@contentType(...)` after it is dropped.
fn multipart_entry(pair: &BruPair) -> MultipartEntry {
    let value = pair.value.trim();
    let value = match value.find(" @contentType(") {
        Some(at) => value[..at].trim_end(),
        None => value,
    };
    let files = value
        .strip_prefix("@file(")
        .and_then(|rest| rest.strip_suffix(')'))
        .map(|paths| {
            paths
                .split('|')
                .filter(|path| !path.is_empty())
                .map(str::to_string)
                .collect()
        });
    MultipartEntry {
        field: KeyValue {
            key: pair.key.clone(),
            value: value.to_string(),
            enabled: pair.enabled,
        },
        files,
    }
}

fn seq_of(text: Option<&str>) -> f64 {
    text.and_then(|seq| seq.trim().parse().ok())
        .unwrap_or(f64::MAX)
}

/// The scripts of a `.bru` collection or folder file.
fn bru_scripts(file: &BruFile) -> Vec<&'static str> {
    [
        ("script:pre-request", "pre-request script"),
        ("script:post-response", "post-response script"),
        ("tests", "test script"),
    ]
    .into_iter()
    .filter(|(block, _)| file.text(block).is_some_and(|text| !text.trim().is_empty()))
    .map(|(_, what)| what)
    .collect()
}

/// Reads `collection.bru` or `folder.bru`.
pub(super) fn bru_defaults(file: &BruFile) -> Defaults {
    let auth = file
        .value("auth", "mode")
        .map(|mode| bru_auth(file, mode))
        .filter(|auth| *auth != RawAuth::Inherit);
    Defaults {
        headers: entries(file.dict("headers")),
        auth,
        vars: entries(file.dict("vars:pre-request")),
        scripts: bru_scripts(file),
        docs: file.text("docs").unwrap_or_default().to_string(),
    }
}

/// The name and place `folder.bru` gives its folder.
pub(super) fn bru_folder_meta(file: &BruFile) -> (Option<String>, f64) {
    (
        file.value("meta", "name").map(str::to_string),
        seq_of(file.value("meta", "seq")),
    )
}

/// Reads a `.bru` request. `fallback` names it when it has no `meta` name.
pub(super) fn bru_request(file: &BruFile, fallback: &str) -> RawRequest {
    let method_block = file
        .blocks
        .iter()
        .find(|block| METHOD_BLOCKS.contains(&block.name.as_str()));
    let (method, settings) = match method_block {
        Some(block) => {
            let pairs = match &block.content {
                BruContent::Dict(pairs) => pairs.as_slice(),
                _ => &[],
            };
            let method = if block.name == "http" {
                pairs
                    .iter()
                    .find(|pair| pair.key == "method")
                    .map(|pair| pair.value.clone())
                    .unwrap_or_else(|| "GET".to_string())
            } else {
                block.name.clone()
            };
            (method.to_uppercase(), pairs)
        }
        None => ("GET".to_string(), &[][..]),
    };
    let setting = |key: &str| {
        settings
            .iter()
            .find(|pair| pair.key == key)
            .map(|pair| pair.value.as_str())
            .unwrap_or_default()
    };
    let kind = file.value("meta", "type").unwrap_or("http").to_string();
    RawRequest {
        name: file
            .value("meta", "name")
            .map_or_else(|| fallback.to_string(), str::to_string),
        seq: seq_of(file.value("meta", "seq")),
        kind,
        method,
        url: setting("url").to_string(),
        query: entries(file.dict("params:query")),
        path: entries(file.dict("params:path")),
        headers: entries(file.dict("headers")),
        body: bru_body(file, setting("body")),
        auth: bru_auth(file, setting("auth")),
        vars_pre: entries(file.dict("vars:pre-request")),
        vars_post: entries(file.dict("vars:post-response")),
        asserts: entries(file.dict("assert")),
        pre_script: file
            .text("script:pre-request")
            .unwrap_or_default()
            .to_string(),
        post_script: file
            .text("script:post-response")
            .unwrap_or_default()
            .to_string(),
        tests: file.text("tests").unwrap_or_default().to_string(),
        docs: file.text("docs").unwrap_or_default().to_string(),
    }
}

/// Reads a `.bru` environment named `name`.
pub(super) fn bru_environment(file: &BruFile, name: &str) -> RawEnvironment {
    RawEnvironment {
        name: name.to_string(),
        vars: entries(file.dict("vars")),
        secrets: file.list("vars:secret").to_vec(),
        color: file.line("color").map(str::to_string),
        extends: file.line("extends").map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::super::bru::parse_bru;
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_request_reads_its_selected_body_and_auth() {
        let file = parse_bru(
            "meta {\n  name: Login\n  type: http\n  seq: 3\n}\n\npost {\n  url: {{host}}/login\n  body: json\n  auth: bearer\n}\n\nauth:basic {\n  username: a\n  password: b\n}\n\nauth:bearer {\n  token: {{token}}\n}\n\nbody:json {\n  {\"a\": 1}\n}\n\nbody:text {\n  ignored\n}\n",
        );
        let request = bru_request(&file, "login");
        assert_eq!(request.name, "Login");
        assert_eq!(request.seq, 3.0);
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "{{host}}/login");
        assert_eq!(request.body, RawBody::Json("{\"a\": 1}".to_string()));
        assert_eq!(
            request.auth,
            RawAuth::Bearer {
                token: "{{token}}".to_string()
            }
        );
    }

    #[test]
    fn custom_methods_multipart_files_and_api_keys() {
        let file = parse_bru(
            "http {\n  method: purge\n  url: /x\n  body: multipartForm\n  auth: apikey\n}\n\nauth:apikey {\n  key: X-Key\n  value: k\n  placement: queryparams\n}\n\nbody:multipart-form {\n  name: n\n  doc: @file(a.pdf|b.pdf) @contentType(application/pdf)\n}\n",
        );
        let request = bru_request(&file, "fallback");
        assert_eq!(request.name, "fallback");
        assert_eq!(request.method, "PURGE");
        assert_eq!(
            request.auth,
            RawAuth::ApiKey {
                key: "X-Key".to_string(),
                value: "k".to_string(),
                query: true
            }
        );
        let RawBody::Multipart(fields) = request.body else {
            panic!("multipart")
        };
        assert_eq!(fields[0].files, None);
        assert_eq!(
            fields[1].files,
            Some(vec!["a.pdf".to_string(), "b.pdf".to_string()])
        );
    }

    #[test]
    fn folder_defaults_skip_inherit_and_list_scripts() {
        let file = parse_bru(
            "meta {\n  name: Users\n  seq: 2\n}\n\nheaders {\n  X-A: 1\n}\n\nauth {\n  mode: inherit\n}\n\nscript:pre-request {\n  bru.setVar('a', 1);\n}\n",
        );
        let defaults = bru_defaults(&file);
        assert_eq!(defaults.auth, None);
        assert_eq!(defaults.headers, vec![KeyValue::new("X-A", "1")]);
        assert_eq!(defaults.scripts, vec!["pre-request script"]);
        assert_eq!(bru_folder_meta(&file), (Some("Users".to_string()), 2.0));
    }
}
