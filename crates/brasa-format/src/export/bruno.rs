//! Writes an [`ExportNode`] tree as a Bruno collection: a `bruno.json` manifest, one `.bru` file
//! per request and a `folder.bru` per subfolder, following the Bru language the `bruno` module
//! already reads (`crate::bruno`, see its `bru.rs` and `model.rs`), confirmed against
//! `usebruno/bruno`'s own `bruToJson.js` parser (block names `meta`, the method blocks, `headers`,
//! `params:query`, `auth:basic`/`auth:bearer`/`auth:apikey`, `body:json`/`body:text`/`body:xml`/
//! `body:form-urlencoded`/`body:multipart-form`, `script:pre-request`, `script:post-response`,
//! `docs`).
//!
//! Unlike [`super::postman`], this does not produce one text document: a Bruno collection is a
//! directory tree, so [`export_bruno_collection`] returns the files to write, paths relative to
//! the collection's own root folder, and leaves creating directories and writing them to the
//! caller (`brasa-app`, which already owns a native save dialog).

use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, MultipartValue, Request, multipart_value};

use super::ExportNode;

/// One file of an exported Bruno collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrunoFile {
    /// The file's path, relative to the collection's own root folder (`bruno.json`'s own
    /// folder), using `/` as the separator regardless of the host OS.
    pub path: String,
    /// The file's content.
    pub content: String,
}

/// Exports `nodes` (the direct children of the folder or workspace being exported) as a Bruno
/// collection named `name`: a `bruno.json` manifest at the root, then one file per [`ExportNode`].
pub fn export_bruno_collection(name: &str, nodes: &[ExportNode]) -> Vec<BrunoFile> {
    let mut files = vec![BrunoFile {
        path: "bruno.json".to_string(),
        content: bruno_json(name),
    }];
    collect(nodes, "", &mut files);
    files
}

/// The collection manifest Bruno requires at the root of every collection folder for it to be
/// recognized as one.
fn bruno_json(name: &str) -> String {
    format!(
        "{{\n  \"version\": \"1\",\n  \"name\": {},\n  \"type\": \"collection\"\n}}\n",
        json_string(name)
    )
}

/// Appends the files of `nodes` (found at `prefix`) to `out`, recursing into subfolders with
/// their own `folder.bru`.
fn collect(nodes: &[ExportNode], prefix: &str, out: &mut Vec<BrunoFile>) {
    for (index, node) in nodes.iter().enumerate() {
        match node {
            ExportNode::Folder { name, children } => {
                let dir = join(prefix, &sanitize(name));
                out.push(BrunoFile {
                    path: join(&dir, "folder.bru"),
                    content: folder_bru(name, index),
                });
                collect(children, &dir, out);
            }
            ExportNode::Request { name, request } => {
                let file_name = format!("{}.bru", sanitize(name));
                out.push(BrunoFile {
                    path: join(prefix, &file_name),
                    content: request_bru(name, index, request),
                });
            }
        }
    }
}

/// Joins a directory and a file/directory name with `/`, skipping an empty `prefix` (the
/// collection root itself).
fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// A name made safe as a single path segment: `/` and `\` (which would otherwise create or
/// escape a directory) become `-`. Bruno's own file names are not re-escaped further; the
/// resulting file is still readable even if the original name had other unusual characters.
fn sanitize(name: &str) -> String {
    name.replace(['/', '\\'], "-")
}

/// `folder.bru`'s content: just enough for Bruno to show the folder with its own name and order.
fn folder_bru(name: &str, index: usize) -> String {
    format!("meta {{\n  name: {name}\n  seq: {}\n}}\n", index + 1)
}

/// One request's `.bru` file.
fn request_bru(name: &str, index: usize, request: &Request) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "meta {{\n  name: {name}\n  type: http\n  seq: {}\n}}\n",
        index + 1
    ));

    let method = request.method.to_string().to_lowercase();
    out.push('\n');
    out.push_str(&method);
    out.push_str(" {\n");
    out.push_str(&dict_line("url", &request.url));
    out.push_str(&dict_line("body", body_mode(&request.body)));
    out.push_str(&dict_line("auth", auth_mode(&request.auth)));
    out.push_str("}\n");

    if !request.query.is_empty() {
        out.push('\n');
        push_dict_block(&mut out, "params:query", &request.query);
    }
    if !request.headers.is_empty() {
        out.push('\n');
        push_dict_block(&mut out, "headers", &request.headers);
    }

    push_auth_block(&mut out, &request.auth);
    push_body_block(&mut out, &request.body);

    if !request.pre_script.is_empty() {
        out.push('\n');
        push_text_block(&mut out, "script:pre-request", &script_text(&request.pre_script, request.pre_enabled));
    }
    if !request.post_script.is_empty() {
        out.push('\n');
        push_text_block(
            &mut out,
            "script:post-response",
            &script_text(&request.post_script, request.post_enabled),
        );
    }
    if !request.docs.is_empty() {
        out.push('\n');
        push_text_block(&mut out, "docs", &request.docs);
    }

    out
}

/// A `key: value` dictionary entry, one line.
fn dict_line(key: &str, value: &str) -> String {
    format!("  {key}: {value}\n")
}

/// A `name { ... }` dictionary block, entries indented two spaces, a disabled one prefixed `~`.
fn push_dict_block(out: &mut String, name: &str, entries: &[KeyValue]) {
    out.push_str(name);
    out.push_str(" {\n");
    for entry in entries {
        let prefix = if entry.enabled { "" } else { "~" };
        out.push_str(&format!("  {prefix}{}: {}\n", dict_key(&entry.key), entry.value));
    }
    out.push_str("}\n");
}

/// Quotes a dictionary key if it has a space or a colon, which would otherwise end it early.
fn dict_key(key: &str) -> String {
    if key.contains([' ', ':']) {
        json_string(key)
    } else {
        key.to_string()
    }
}

/// A `name { ... }` text block, every line of `text` indented two spaces, as Bruno itself writes
/// a body, a script or `docs`.
fn push_text_block(out: &mut String, name: &str, text: &str) {
    out.push_str(name);
    out.push_str(" {\n");
    for line in text.lines() {
        out.push_str("  ");
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("}\n");
}

/// The `body` setting of the method block, see `crate::bruno::model::bru_body`'s read side.
fn body_mode(body: &Body) -> &'static str {
    match body {
        Body::None => "none",
        Body::Json(_) => "json",
        Body::Text(_) => "text",
        Body::Xml(_) => "xml",
        Body::Form(_) => "formUrlEncoded",
        Body::Multipart(_) => "multipartForm",
    }
}

/// The `auth` setting of the method block, see `crate::bruno::model::bru_auth`'s read side.
fn auth_mode(auth: &Auth) -> &'static str {
    match auth {
        Auth::None => "none",
        Auth::Basic { .. } => "basic",
        Auth::Bearer { .. } => "bearer",
        Auth::ApiKey { .. } => "apikey",
    }
}

fn push_body_block(out: &mut String, body: &Body) {
    match body {
        Body::None => {}
        Body::Json(text) => {
            out.push('\n');
            push_text_block(out, "body:json", text);
        }
        Body::Text(text) => {
            out.push('\n');
            push_text_block(out, "body:text", text);
        }
        Body::Xml(text) => {
            out.push('\n');
            push_text_block(out, "body:xml", text);
        }
        Body::Form(fields) => {
            out.push('\n');
            push_dict_block(out, "body:form-urlencoded", fields);
        }
        Body::Multipart(fields) => {
            out.push('\n');
            out.push_str("body:multipart-form {\n");
            for field in fields {
                let prefix = if field.enabled { "" } else { "~" };
                let value = match multipart_value(&field.value) {
                    MultipartValue::File(path) => format!("@file({path})"),
                    MultipartValue::Text(text) => text.to_string(),
                };
                out.push_str(&format!("  {prefix}{}: {value}\n", dict_key(&field.key)));
            }
            out.push_str("}\n");
        }
    }
}

fn push_auth_block(out: &mut String, auth: &Auth) {
    match auth {
        Auth::None => {}
        Auth::Basic { username, password } => {
            out.push('\n');
            out.push_str("auth:basic {\n");
            out.push_str(&dict_line("username", username));
            out.push_str(&dict_line("password", password));
            out.push_str("}\n");
        }
        Auth::Bearer { token } => {
            out.push('\n');
            out.push_str("auth:bearer {\n");
            out.push_str(&dict_line("token", token));
            out.push_str("}\n");
        }
        Auth::ApiKey {
            name,
            value,
            location,
        } => {
            out.push('\n');
            out.push_str("auth:apikey {\n");
            out.push_str(&dict_line("key", name));
            out.push_str(&dict_line("value", value));
            let placement = match location {
                ApiKeyLocation::Header => "header",
                ApiKeyLocation::Query => "queryparams",
            };
            out.push_str(&dict_line("placement", placement));
            out.push_str("}\n");
        }
    }
}

/// Prepended to an exported script, since Brasa's script API is not Bruno's `bru.*`/`req.*`/
/// `res.*`, see `super::postman`'s identical banner.
const SCRIPT_BANNER: &str =
    "// Written for Brasa's script API (docs/scripting.md), not Bruno's bru.*/req.*/res.* API. Adapt before running here.";
/// Prepended, after [`SCRIPT_BANNER`], to a script that was turned off in Brasa.
const DISABLED_BANNER: &str = "// Turned off in Brasa: kept here, but it did not run there.";

fn script_text(script: &str, enabled: bool) -> String {
    let mut text = SCRIPT_BANNER.to_string();
    text.push('\n');
    if !enabled {
        text.push_str(DISABLED_BANNER);
        text.push('\n');
    }
    text.push_str(script);
    text
}

/// A JSON-quoted string, for `bruno.json` and a dictionary key that needs quoting: both follow
/// the same escaping rules as JSON strings.
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| format!("{text:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use brasa_core::Method;
    use pretty_assertions::assert_eq;

    fn get(url: &str) -> Request {
        Request {
            url: url.to_string(),
            ..Request::default()
        }
    }

    #[test]
    fn a_folder_gets_its_own_folder_bru() {
        let tree = vec![ExportNode::Folder {
            name: "US001".to_string(),
            children: vec![ExportNode::Request {
                name: "list users".to_string(),
                request: get("https://example.com/users"),
            }],
        }];
        let files = export_bruno_collection("US001", &tree);
        let paths: Vec<_> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["bruno.json", "US001/folder.bru", "US001/list users.bru"]
        );
        let folder = &files[1].content;
        assert!(folder.contains("name: US001"));
    }

    #[test]
    fn bruno_json_names_the_collection() {
        let files = export_bruno_collection("My Collection", &[]);
        assert_eq!(
            files[0].content,
            "{\n  \"version\": \"1\",\n  \"name\": \"My Collection\",\n  \"type\": \"collection\"\n}\n"
        );
    }

    #[test]
    fn a_get_request_writes_its_method_block() {
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request: get("https://example.com"),
        }];
        let files = export_bruno_collection("c", &tree);
        let content = &files[1].content;
        assert!(content.contains("meta {\n  name: r\n  type: http\n  seq: 1\n}"));
        assert!(content.contains("get {\n  url: https://example.com\n  body: none\n  auth: none\n}"));
    }

    #[test]
    fn a_disabled_header_is_prefixed_with_a_tilde() {
        let mut request = get("https://example.com");
        let mut header = KeyValue::new("X-Debug", "1");
        header.enabled = false;
        request.headers.push(header);
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let files = export_bruno_collection("c", &tree);
        assert!(files[1].content.contains("headers {\n  ~X-Debug: 1\n}"));
    }

    #[test]
    fn a_json_body_is_a_text_block() {
        let mut request = get("https://example.com");
        request.method = Method::Post;
        request.body = Body::Json("{\n  \"a\": 1\n}".to_string());
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let files = export_bruno_collection("c", &tree);
        let content = &files[1].content;
        assert!(content.contains("post {\n  url: https://example.com\n  body: json\n  auth: none\n}"));
        assert!(content.contains("body:json {\n  {\n    \"a\": 1\n  }\n}"));
    }

    #[test]
    fn a_bearer_auth_is_its_own_block() {
        let mut request = get("https://example.com");
        request.auth = Auth::Bearer {
            token: "{{token}}".to_string(),
        };
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let files = export_bruno_collection("c", &tree);
        assert!(
            files[1]
                .content
                .contains("auth:bearer {\n  token: {{token}}\n}")
        );
    }

    #[test]
    fn a_disabled_pre_script_carries_a_note() {
        let mut request = get("https://example.com");
        request.pre_script = "vars.set(\"a\", 1);".to_string();
        request.pre_enabled = false;
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let files = export_bruno_collection("c", &tree);
        let content = &files[1].content;
        assert!(content.contains("Turned off in Brasa"));
        assert!(content.contains("vars.set(\"a\", 1);"));
    }
}
