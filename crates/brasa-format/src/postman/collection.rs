//! Mapping a Postman collection (v2.1, v2.0 accepted when it parses the same) into an
//! [`ImportPlan`]. The mapping is described in `docs/postman-import.md`.

use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, Method, Request};

use super::error::PostmanError;
use super::model::{
    RawAuth, RawAuthParam, RawBody, RawCollection, RawEvent, RawFormDataParam, RawItem, RawRequest,
    RawSavedResponse, RawUrl, RawUrlDetailed,
};
use super::script::convert_script;
use super::value::value_to_text;

/// The first line of a script imported without converting it (the option is off).
const IMPORT_SCRIPT_HEADER: &str =
    "// Imported from Postman. Scripts were not converted, adapt the pm.* calls.";

/// The variable that holds the access token of a request with OAuth 2.0 authorization.
const OAUTH2_TOKEN_VARIABLE: &str = "oauth2AccessToken";

/// What to convert from a collection, the wizard's "Convert" options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostmanOptions {
    /// Keep Postman's folders. Off, every request goes at the top of the collection.
    pub folders: bool,
    /// Collection variables and globals go in every environment.
    pub variables: bool,
    /// Variables of type secret go to `.local.env`. Off, they are left out.
    pub secrets: bool,
    /// Convert the authorization of requests, inherited or their own. Off, requests have none.
    pub auth: bool,
    /// Convert `pm.*` scripts. Off, they are copied as comments.
    pub scripts: bool,
    /// Keep the description of each request and its saved examples, in the request's docs.
    pub docs: bool,
}

impl Default for PostmanOptions {
    fn default() -> Self {
        PostmanOptions {
            folders: true,
            variables: true,
            secrets: true,
            auth: true,
            scripts: true,
            docs: false,
        }
    }
}

/// Something about one request that the wizard's preview and the import report tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestIssue {
    /// Its `pm.*` scripts were converted in full.
    ScriptConverted,
    /// Part of a script is commented out, `converted` of `total` statements stayed.
    ScriptPartial {
        /// The statements kept as code.
        converted: usize,
        /// The statements of the script.
        total: usize,
    },
    /// A script uses `pm.sendRequest`, which Brasa does not have.
    SendRequest,
    /// Its scripts were copied as comments, since the option is off.
    ScriptNotConverted,
    /// OAuth 2.0: the token is sent as a bearer token and is not renewed.
    OAuth2Token,
    /// An authorization Brasa does not have: the request is imported without it.
    AuthUnsupported(String),
    /// A form-data field points to a file, whose path may not exist on this machine.
    FormDataFile,
    /// A body Brasa does not have: the request is imported without a body.
    BodyUnsupported(String),
    /// A form-data field picks several files, only the first one is kept.
    FormDataManyFiles,
}

/// How serious a [`RequestIssue`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IssueLevel {
    /// Worth telling, nothing to do.
    Note,
    /// The request works, but it needs a look.
    Warning,
    /// A part of the request is lost.
    Unsupported,
}

impl RequestIssue {
    /// How serious it is.
    pub fn level(&self) -> IssueLevel {
        match self {
            RequestIssue::ScriptConverted => IssueLevel::Note,
            RequestIssue::ScriptPartial { .. }
            | RequestIssue::SendRequest
            | RequestIssue::ScriptNotConverted
            | RequestIssue::OAuth2Token
            | RequestIssue::FormDataFile
            | RequestIssue::FormDataManyFiles => IssueLevel::Warning,
            RequestIssue::AuthUnsupported(_) | RequestIssue::BodyUnsupported(_) => {
                IssueLevel::Unsupported
            }
        }
    }
}

/// The result of importing a Postman collection: a tree of folders and requests, plus the
/// collection's own variables and anything that could not be mapped faithfully. Pure in-memory
/// data, ready for `brasa-workspace` to write to disk.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportPlan {
    /// The collection name (`info.name`), used as the name of the new top-level folder and of
    /// the environment created from the collection's own variables.
    pub collection_name: String,
    /// `info._postman_id`, empty when the export has none.
    pub postman_id: String,
    /// The format version from `info.schema` (`v2.1.0`), empty when it names none.
    pub schema_version: String,
    /// The root folder of the imported tree. Its own name is not meaningful, only its `folders`
    /// and `requests` fields are used.
    pub root: ImportFolder,
    /// The collection's own `variable` array, to become the environment named after the
    /// collection (`environments/<collection name>.env`).
    pub variables: Vec<KeyValue>,
    /// The variables of type `secret`, and the access tokens of OAuth 2.0 authorizations, for
    /// `environments/<collection name>.local.env`.
    pub secret_variables: Vec<KeyValue>,
    /// Everything that could not be mapped faithfully, in the order it was found.
    pub warnings: Vec<String>,
}

/// One folder of an [`ImportPlan`], mirroring a Postman collection folder.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportFolder {
    /// The folder name, as it appeared in the collection.
    pub name: String,
    /// Nested folders, in collection order.
    pub folders: Vec<ImportFolder>,
    /// Requests directly inside this folder, as `(name, Request)` pairs, in collection order.
    pub requests: Vec<(String, Request)>,
    /// What to tell about each of `requests`, by position.
    pub issues: Vec<Vec<RequestIssue>>,
}

/// What mapping a collection collects on the way.
struct Mapping<'a> {
    options: &'a PostmanOptions,
    warnings: Vec<String>,
    /// The variables OAuth 2.0 authorizations need.
    secrets: Vec<KeyValue>,
}

/// Parses the text of a Postman collection export into an [`ImportPlan`].
pub fn parse_collection(text: &str, options: &PostmanOptions) -> Result<ImportPlan, PostmanError> {
    let raw: RawCollection =
        serde_json::from_str(text).map_err(PostmanError::InvalidCollectionJson)?;
    let mut mapping = Mapping {
        options,
        warnings: Vec::new(),
        secrets: Vec::new(),
    };
    warn_about_dropped_events(
        raw.event.as_deref(),
        "collection",
        &raw.info.name,
        &mut mapping.warnings,
    );
    let root = map_items(&raw.item, raw.auth.as_ref(), &raw.info.name, &mut mapping);
    let mut warnings = mapping.warnings;
    let mut secret_variables = mapping.secrets;
    let mut variables = Vec::new();
    for variable in raw.variable {
        let Some(key) = variable.key else { continue };
        if variable.disabled {
            continue;
        }
        let context = format!("variable \"{key}\"");
        let value = value_to_text(variable.value.as_ref(), &context, &mut warnings);
        if variable.kind.as_deref() == Some("secret") {
            secret_variables.push(KeyValue::new(key, value));
        } else {
            variables.push(KeyValue::new(key, value));
        }
    }
    let schema_version = raw
        .info
        .schema
        .split('/')
        .find(|part| part.starts_with('v') && part.contains('.'))
        .unwrap_or_default()
        .to_string();
    Ok(ImportPlan {
        collection_name: raw.info.name,
        postman_id: raw.info.postman_id,
        schema_version,
        root,
        variables,
        secret_variables,
        warnings,
    })
}

/// Maps one `item` array (the collection's own, or a folder's) into a folder, resolving auth
/// inheritance as it goes.
///
/// `inherited_auth` is the auth block that applies to a request in this list when the request
/// itself has no auth of its own or explicitly uses `"inherit"`. `path` is a human readable
/// location, used to prefix warnings.
fn map_items(
    items: &[RawItem],
    inherited_auth: Option<&RawAuth>,
    path: &str,
    mapping: &mut Mapping<'_>,
) -> ImportFolder {
    let mut folder = ImportFolder {
        name: String::new(),
        folders: Vec::new(),
        requests: Vec::new(),
        issues: Vec::new(),
    };

    for item in items {
        let name = item.name.clone().unwrap_or_default();
        let item_path = format!("{path}/{name}");

        if let Some(children) = &item.item {
            let folder_auth = item.auth.as_ref().or(inherited_auth);
            warn_about_dropped_events(
                item.event.as_deref(),
                "folder",
                &name,
                &mut mapping.warnings,
            );
            let mut child = map_items(children, folder_auth, &item_path, mapping);
            child.name = name;
            folder.folders.push(child);
        } else if let Some(request) = &item.request {
            let auth = effective_auth(request.auth.as_ref(), inherited_auth);
            let events = item.event.as_deref().unwrap_or(&[]);
            let (mapped, issues) =
                map_request(request, events, auth, &item.response, &item_path, mapping);
            folder.requests.push((name, mapped));
            folder.issues.push(issues);
        }
        // An item with neither `item` nor `request` is malformed and has nothing to map.
    }

    folder
}

/// Warns about a `prerequest`/`test` event found at the collection or folder level.
///
/// Only per-request scripts are imported: a collection- or
/// folder-level script has no single request to attach to, so it is never imported. This only
/// records that it was dropped, naming `level` (`"collection"` or `"folder"`) and `name`, so the
/// user knows some code was left behind.
fn warn_about_dropped_events(
    events: Option<&[RawEvent]>,
    level: &str,
    name: &str,
    warnings: &mut Vec<String>,
) {
    for event in events.into_iter().flatten() {
        let Some(kind) = event.listen.as_deref() else {
            continue;
        };
        if kind == "prerequest" || kind == "test" {
            warnings.push(format!(
                "{level} \"{name}\": a {kind} script is not imported at the {level} level, only per request"
            ));
        }
    }
}

/// Resolves which auth block applies to a request: its own, unless it is absent or explicitly
/// `"inherit"`, in which case the collection/folder level auth applies instead.
/// Collection and folder level auth is inherited by requests that use `inherit` or no auth.
fn effective_auth<'a>(
    own_auth: Option<&'a RawAuth>,
    inherited_auth: Option<&'a RawAuth>,
) -> Option<&'a RawAuth> {
    match own_auth {
        Some(auth) if auth.kind != "inherit" => Some(auth),
        Some(_) => inherited_auth,
        None => inherited_auth,
    }
}

/// Maps one Postman request into a Brasa [`Request`], and what to tell about it.
fn map_request(
    request: &RawRequest,
    events: &[RawEvent],
    auth: Option<&RawAuth>,
    examples: &[RawSavedResponse],
    path: &str,
    mapping: &mut Mapping<'_>,
) -> (Request, Vec<RequestIssue>) {
    let mut issues = Vec::new();
    let warnings = &mut mapping.warnings;
    let method = parse_method(request.method.as_deref().unwrap_or("GET"));
    let (url, query) = map_url(request.url.as_ref(), path, warnings);

    let mut headers = Vec::new();
    for header in request.header.iter().flatten() {
        let context = format!("{path}: header \"{}\"", header.key);
        headers.push(KeyValue {
            key: header.key.clone(),
            value: value_to_text(header.value.as_ref(), &context, warnings),
            enabled: !header.disabled,
        });
    }
    let auth = match auth {
        Some(auth) if mapping.options.auth => {
            map_auth(auth, path, warnings, &mut mapping.secrets, &mut issues)
        }
        _ => Auth::None,
    };

    let body = map_body(request.body.as_ref(), path, warnings, &mut issues);
    let docs = if mapping.options.docs {
        let description = request
            .description
            .as_ref()
            .map(super::model::RawDescription::as_text)
            .unwrap_or_default();
        with_examples(description, examples)
    } else {
        String::new()
    };

    let pre_script = map_script(events, "prerequest", mapping.options.scripts, &mut issues);
    let post_script = map_script(events, "test", mapping.options.scripts, &mut issues);
    (
        Request {
            method,
            url,
            headers,
            query,
            auth,
            body,
            pre_script,
            pre_enabled: true,
            post_script,
            post_enabled: true,
            docs,
        },
        issues,
    )
}

/// The docs of a request: its description, then its saved examples.
fn with_examples(description: String, examples: &[RawSavedResponse]) -> String {
    let mut docs = description;
    for example in examples {
        if !docs.is_empty() {
            docs.push_str("\n\n");
        }
        let name = example.name.as_deref().unwrap_or("Example");
        docs.push_str(&format!("## {name}"));
        if let Some(code) = example.code {
            docs.push_str(&format!(" ({code})"));
        }
        if let Some(body) = example.body.as_deref().filter(|body| !body.is_empty()) {
            docs.push_str(&format!("\n\n```\n{body}\n```"));
        }
    }
    docs
}

/// Parses a Postman method token, which is always one of the standard uppercase methods in
/// practice. [`Method::from_str`] never fails, an unrecognized token simply becomes
/// [`Method::Custom`].
fn parse_method(token: &str) -> Method {
    match token.parse::<Method>() {
        Ok(method) => method,
        // `Method::from_str`'s error type is `Infallible`: this branch can never run.
        Err(never) => match never {},
    }
}

/// Maps a `url`, splitting the query string out of `raw` and combining it with the structured
/// `query` array: `url.raw` without its query string, plus `url.query`.
fn map_url(
    url: Option<&RawUrl>,
    path: &str,
    warnings: &mut Vec<String>,
) -> (String, Vec<KeyValue>) {
    let Some(url) = url else {
        warnings.push(format!(
            "{path}: request has no URL, imported with an empty URL"
        ));
        return (String::new(), Vec::new());
    };
    match url {
        RawUrl::Raw(raw) => {
            let (base, query_string) = split_query_string(raw);
            (base, parse_query_string(&query_string))
        }
        RawUrl::Detailed(detailed) => map_detailed_url(detailed, path, warnings),
    }
}

/// Maps the detailed object form of a `url`.
fn map_detailed_url(
    detailed: &RawUrlDetailed,
    path: &str,
    warnings: &mut Vec<String>,
) -> (String, Vec<KeyValue>) {
    let raw = detailed.raw.clone().unwrap_or_default();
    let (base, query_string) = split_query_string(&raw);
    if detailed.query.is_empty() {
        (base, parse_query_string(&query_string))
    } else {
        let mut query = Vec::new();
        for param in &detailed.query {
            let key = param.key.clone().unwrap_or_default();
            let context = format!("{path}: query \"{key}\"");
            let value = value_to_text(param.value.as_ref(), &context, warnings);
            query.push(KeyValue {
                key,
                value,
                enabled: !param.disabled,
            });
        }
        (base, query)
    }
}

/// Splits a raw URL at its first `?`, returning `(url_without_query, query_string)`. When there
/// is no `?`, the whole string is the URL and the query string is empty.
fn split_query_string(raw: &str) -> (String, String) {
    match raw.split_once('?') {
        Some((base, query)) => (base.to_string(), query.to_string()),
        None => (raw.to_string(), String::new()),
    }
}

/// Parses a `key=value&key2=value2` query string into [`KeyValue`] entries. Used only for the
/// plain string form of a `url` (or when the detailed form omits its `query` array), since the
/// object form's `query` array already carries this information.
fn parse_query_string(query: &str) -> Vec<KeyValue> {
    if query.is_empty() {
        return Vec::new();
    }
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| match pair.split_once('=') {
            Some((key, value)) => KeyValue::new(key, value),
            None => KeyValue::new(pair, ""),
        })
        .collect()
}

/// Maps an `auth` block into the request's [`Auth`]: bearer, basic, apikey in the header or the
/// query, and the access token of OAuth 2.0, sent as a bearer token.
fn map_auth(
    auth: &RawAuth,
    path: &str,
    warnings: &mut Vec<String>,
    secrets: &mut Vec<KeyValue>,
    issues: &mut Vec<RequestIssue>,
) -> Auth {
    match auth.kind.as_str() {
        "bearer" => {
            let context = format!("{path}: auth bearer \"token\"");
            let token = find_param(&auth.bearer, "token", &context, warnings).unwrap_or_default();
            Auth::Bearer { token }
        }
        "basic" => {
            let username_context = format!("{path}: auth basic \"username\"");
            let password_context = format!("{path}: auth basic \"password\"");
            let username = find_param(&auth.basic, "username", &username_context, warnings)
                .unwrap_or_default();
            let password = find_param(&auth.basic, "password", &password_context, warnings)
                .unwrap_or_default();
            Auth::Basic { username, password }
        }
        "apikey" => {
            let key_context = format!("{path}: auth apikey \"key\"");
            let value_context = format!("{path}: auth apikey \"value\"");
            let in_context = format!("{path}: auth apikey \"in\"");
            let key = find_param(&auth.apikey, "key", &key_context, warnings).unwrap_or_default();
            let value =
                find_param(&auth.apikey, "value", &value_context, warnings).unwrap_or_default();
            let location = find_param(&auth.apikey, "in", &in_context, warnings)
                .unwrap_or_else(|| "header".to_string());
            let location = match location.as_str() {
                "header" => ApiKeyLocation::Header,
                "query" => ApiKeyLocation::Query,
                _ => {
                    warnings.push(format!(
                        "{path}: apikey auth in \"{location}\" is not supported, only header \
                         and query, skipped"
                    ));
                    issues.push(RequestIssue::AuthUnsupported(format!(
                        "apikey in {location}"
                    )));
                    return Auth::None;
                }
            };
            Auth::ApiKey {
                name: key,
                value,
                location,
            }
        }
        "oauth2" => {
            let context = format!("{path}: auth oauth2 \"accessToken\"");
            let token =
                find_param(&auth.oauth2, "accessToken", &context, warnings).unwrap_or_default();
            if !secrets
                .iter()
                .any(|entry| entry.key == OAUTH2_TOKEN_VARIABLE)
            {
                secrets.push(KeyValue::new(OAUTH2_TOKEN_VARIABLE, token));
            }
            issues.push(RequestIssue::OAuth2Token);
            Auth::Bearer {
                token: format!("{{{{{OAUTH2_TOKEN_VARIABLE}}}}}"),
            }
        }
        "noauth" | "" => Auth::None,
        other => {
            warnings.push(format!(
                "{path}: auth type \"{other}\" is not supported, skipped"
            ));
            issues.push(RequestIssue::AuthUnsupported(other.to_string()));
            Auth::None
        }
    }
}

/// Finds the auth param named `key`, returning its value converted to text, or `None` if there
/// is no such param at all (used to tell "absent" apart from "present but empty", which matters
/// for the apikey `"in"` location default).
fn find_param(
    params: &[RawAuthParam],
    key: &str,
    context: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    params
        .iter()
        .find(|param| param.key == key)
        .map(|param| value_to_text(param.value.as_ref(), context, warnings))
}

/// Maps a `body` object into a Brasa [`Body`].
fn map_body(
    body: Option<&RawBody>,
    path: &str,
    warnings: &mut Vec<String>,
    issues: &mut Vec<RequestIssue>,
) -> Body {
    let Some(body) = body else {
        return Body::None;
    };
    match body.mode.as_deref() {
        None => Body::None,
        Some("raw") => map_raw_body(body),
        Some("urlencoded") => {
            let mut fields = Vec::new();
            for param in &body.urlencoded {
                let context = format!("{path}: form field \"{}\"", param.key);
                fields.push(KeyValue {
                    key: param.key.clone(),
                    value: value_to_text(param.value.as_ref(), &context, warnings),
                    enabled: !param.disabled,
                });
            }
            Body::Form(fields)
        }
        Some("formdata") => map_form_data(&body.formdata, path, warnings, issues),
        Some(mode @ ("file" | "graphql")) => {
            warnings.push(format!(
                "{path}: {mode} body is not supported, imported as empty"
            ));
            issues.push(RequestIssue::BodyUnsupported(mode.to_string()));
            Body::None
        }
        Some(other) => {
            warnings.push(format!(
                "{path}: unknown body mode \"{other}\", imported as empty"
            ));
            issues.push(RequestIssue::BodyUnsupported(other.to_string()));
            Body::None
        }
    }
}

/// Maps a `formdata` body: a file field becomes `@path`, a text field that starts with `@` is
/// escaped.
fn map_form_data(
    params: &[RawFormDataParam],
    path: &str,
    warnings: &mut Vec<String>,
    issues: &mut Vec<RequestIssue>,
) -> Body {
    let mut fields = Vec::new();
    let mut has_file = false;
    let mut many_files = false;
    for param in params {
        let value = if param.kind.as_deref() == Some("file") {
            has_file = true;
            let files: Vec<&str> = match param.src.as_ref() {
                Some(serde_json::Value::String(file)) => vec![file.as_str()],
                Some(serde_json::Value::Array(files)) => {
                    files.iter().filter_map(|file| file.as_str()).collect()
                }
                _ => Vec::new(),
            };
            many_files |= files.len() > 1;
            format!("@{}", files.first().copied().unwrap_or_default())
        } else {
            let context = format!("{path}: form field \"{}\"", param.key);
            let text = value_to_text(param.value.as_ref(), &context, warnings);
            if text.starts_with('@') {
                format!("@{text}")
            } else {
                text
            }
        };
        fields.push(KeyValue {
            key: param.key.clone(),
            value,
            enabled: !param.disabled,
        });
    }
    if has_file {
        issues.push(RequestIssue::FormDataFile);
    }
    if many_files {
        warnings.push(format!(
            "{path}: a form-data field picks several files, only the first is kept"
        ));
        issues.push(RequestIssue::FormDataManyFiles);
    }
    Body::Multipart(fields)
}

/// Maps a `mode: "raw"` body, dispatching on `options.raw.language` (default `text`).
fn map_raw_body(body: &RawBody) -> Body {
    let text = body.raw.clone().unwrap_or_default();
    let language = body
        .options
        .as_ref()
        .and_then(|options| options.raw.as_ref())
        .and_then(|raw_options| raw_options.language.as_deref())
        .unwrap_or("text");
    match language {
        "json" => Body::Json(text),
        "xml" => Body::Xml(text),
        _ => Body::Text(text),
    }
}

/// Builds the `pre` or `post` script for the events whose `listen` matches `kind` (`"prerequest"`
/// or `"test"`): converted, or commented out line by line under a fixed header when `convert` is
/// off. Returns an empty string when there is no such event, or its script has no lines, so the
/// section is omitted when the request is serialized.
fn map_script(
    events: &[RawEvent],
    kind: &str,
    convert: bool,
    issues: &mut Vec<RequestIssue>,
) -> String {
    let lines: Vec<String> = events
        .iter()
        .filter(|event| event.listen.as_deref() == Some(kind))
        .filter_map(|event| event.script.as_ref())
        .filter_map(|script| script.exec.as_ref())
        .flat_map(super::model::RawExec::lines)
        .collect();
    if lines.iter().all(|line| line.trim().is_empty()) {
        return String::new();
    }
    if !convert {
        let mut commented = vec![IMPORT_SCRIPT_HEADER.to_string()];
        commented.extend(lines.iter().map(|line| format!("// {line}")));
        if !issues.contains(&RequestIssue::ScriptNotConverted) {
            issues.push(RequestIssue::ScriptNotConverted);
        }
        return commented.join("\n");
    }
    let converted = convert_script(&lines.join("\n"));
    if converted.send_request && !issues.contains(&RequestIssue::SendRequest) {
        issues.push(RequestIssue::SendRequest);
    }
    if converted.converted < converted.total {
        if !converted.send_request {
            issues.push(RequestIssue::ScriptPartial {
                converted: converted.converted,
                total: converted.total,
            });
        }
    } else if converted.rewrote && !issues.contains(&RequestIssue::ScriptConverted) {
        issues.push(RequestIssue::ScriptConverted);
    }
    converted.text
}
