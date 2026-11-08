//! Writes an [`ExportNode`] tree as a Postman Collection v2.1 JSON document, the reverse of
//! `crate::postman::parse_collection`.

use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, MultipartValue, Request, multipart_value};
use serde_json::{Map, Value, json};

use super::ExportNode;

const SCHEMA: &str = "https://schema.getpostman.com/json/collection/v2.1.0/collection.json";

/// Exports `nodes` (the direct children of the folder or workspace being exported) as a Postman
/// Collection v2.1 JSON document named `name`, pretty printed.
pub fn export_postman_collection(name: &str, nodes: &[ExportNode]) -> String {
    let collection = json!({
        "info": {
            "name": name,
            "schema": SCHEMA,
        },
        "item": nodes.iter().map(export_node).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&collection).unwrap_or_default()
}

/// Exports one [`ExportNode`], a folder (its own `item` array) or a request (a `request` and,
/// when it has one, an `event` array of scripts).
fn export_node(node: &ExportNode) -> Value {
    match node {
        ExportNode::Folder { name, children } => json!({
            "name": name,
            "item": children.iter().map(export_node).collect::<Vec<_>>(),
        }),
        ExportNode::Request { name, request } => {
            let mut item = Map::new();
            item.insert("name".to_string(), json!(name));
            item.insert("request".to_string(), export_request(request));
            let events = export_events(request);
            if !events.is_empty() {
                item.insert("event".to_string(), json!(events));
            }
            Value::Object(item)
        }
    }
}

/// Exports the `request` object: method, headers, URL, body and auth.
fn export_request(request: &Request) -> Value {
    let mut map = Map::new();
    map.insert("method".to_string(), json!(request.method.to_string()));
    map.insert(
        "header".to_string(),
        json!(request.headers.iter().map(export_header).collect::<Vec<_>>()),
    );
    map.insert("url".to_string(), json!({ "raw": request.url }));
    if let Some(body) = export_body(&request.body) {
        map.insert("body".to_string(), body);
    }
    if let Some(auth) = export_auth(&request.auth) {
        map.insert("auth".to_string(), auth);
    }
    Value::Object(map)
}

/// Exports one header entry. `disabled` is the inverse of [`KeyValue::enabled`].
fn export_header(header: &KeyValue) -> Value {
    json!({
        "key": header.key,
        "value": header.value,
        "disabled": !header.enabled,
    })
}

/// Exports [`Body`] as Postman's `body` object, `None` for [`Body::None`] (Postman's own absence
/// of a body).
fn export_body(body: &Body) -> Option<Value> {
    match body {
        Body::None => None,
        Body::Json(text) => Some(raw_body(text, "json")),
        Body::Text(text) => Some(raw_body(text, "text")),
        Body::Xml(text) => Some(raw_body(text, "xml")),
        Body::Form(fields) => Some(json!({
            "mode": "urlencoded",
            "urlencoded": fields.iter().map(export_header).collect::<Vec<_>>(),
        })),
        Body::Multipart(fields) => Some(json!({
            "mode": "formdata",
            "formdata": fields.iter().map(export_multipart_field).collect::<Vec<_>>(),
        })),
    }
}

/// A `mode: raw` body with its `options.raw.language` hint, for the Json/Text/Xml variants.
fn raw_body(text: &str, language: &str) -> Value {
    json!({
        "mode": "raw",
        "raw": text,
        "options": { "raw": { "language": language } },
    })
}

/// Exports one `formdata` entry: a `file` field carries its path in `src`, a text field its
/// value in `value`, mirroring [`multipart_value`].
fn export_multipart_field(field: &KeyValue) -> Value {
    match multipart_value(&field.value) {
        MultipartValue::File(path) => json!({
            "key": field.key,
            "type": "file",
            "src": path,
            "disabled": !field.enabled,
        }),
        MultipartValue::Text(text) => json!({
            "key": field.key,
            "type": "text",
            "value": text,
            "disabled": !field.enabled,
        }),
    }
}

/// Exports [`Auth`] as Postman's `auth` object, `None` for [`Auth::None`] (no override at the
/// request level, Postman's own default).
fn export_auth(auth: &Auth) -> Option<Value> {
    match auth {
        Auth::None => None,
        Auth::Basic { username, password } => Some(json!({
            "type": "basic",
            "basic": [
                { "key": "username", "value": username, "type": "string" },
                { "key": "password", "value": password, "type": "string" },
            ],
        })),
        Auth::Bearer { token } => Some(json!({
            "type": "bearer",
            "bearer": [{ "key": "token", "value": token, "type": "string" }],
        })),
        Auth::ApiKey {
            name,
            value,
            location,
        } => {
            let in_value = match location {
                ApiKeyLocation::Header => "header",
                ApiKeyLocation::Query => "query",
            };
            Some(json!({
                "type": "apikey",
                "apikey": [
                    { "key": "key", "value": name, "type": "string" },
                    { "key": "value", "value": value, "type": "string" },
                    { "key": "in", "value": in_value, "type": "string" },
                ],
            }))
        }
    }
}

/// Exports `request`'s `pre`/`post` scripts as Postman's `event` array (`prerequest`, `test`).
/// Each script is carried over as is, not translated to `pm.*` (see the module docs), with a
/// comment banner saying so, and a second line when it was turned off in Brasa, so nothing about
/// it is lost or silently different once it lands in Postman.
fn export_events(request: &Request) -> Vec<Value> {
    let mut events = Vec::new();
    if !request.pre_script.is_empty() {
        events.push(script_event("prerequest", &request.pre_script, request.pre_enabled));
    }
    if !request.post_script.is_empty() {
        events.push(script_event("test", &request.post_script, request.post_enabled));
    }
    events
}

/// One `event` entry: `listen` is `"prerequest"` or `"test"`, `exec` is the script split into
/// lines, as Postman itself writes it.
fn script_event(listen: &str, script: &str, enabled: bool) -> Value {
    let mut lines = vec![SCRIPT_BANNER.to_string()];
    if !enabled {
        lines.push(DISABLED_BANNER.to_string());
    }
    lines.extend(script.lines().map(str::to_string));
    json!({
        "listen": listen,
        "script": { "type": "text/javascript", "exec": lines },
    })
}

/// Prepended to every exported script: it is Brasa's own API, not Postman's `pm.*`.
const SCRIPT_BANNER: &str =
    "// Written for Brasa's script API (docs/scripting.md), not Postman's pm.* API. Adapt before running here.";
/// Prepended, after [`SCRIPT_BANNER`], to a script that was turned off in Brasa.
const DISABLED_BANNER: &str = "// Turned off in Brasa: kept here, but it did not run there.";

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
    fn a_folder_nests_its_children() {
        let tree = vec![ExportNode::Folder {
            name: "US001".to_string(),
            children: vec![ExportNode::Request {
                name: "list users".to_string(),
                request: get("https://example.com/users"),
            }],
        }];
        let text = export_postman_collection("US001", &tree);
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(parsed["info"]["name"], "US001");
        assert_eq!(parsed["item"][0]["name"], "US001");
        assert_eq!(parsed["item"][0]["item"][0]["name"], "list users");
        assert_eq!(
            parsed["item"][0]["item"][0]["request"]["method"],
            "GET"
        );
    }

    #[test]
    fn a_disabled_header_is_marked_disabled() {
        let mut request = get("https://example.com");
        let mut header = KeyValue::new("X-Debug", "1");
        header.enabled = false;
        request.headers.push(header);
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let text = export_postman_collection("c", &tree);
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(parsed["item"][0]["request"]["header"][0]["disabled"], true);
    }

    #[test]
    fn a_json_body_is_a_raw_mode_with_a_language_hint() {
        let mut request = get("https://example.com");
        request.method = Method::Post;
        request.body = Body::Json("{}".to_string());
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let text = export_postman_collection("c", &tree);
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        let body = &parsed["item"][0]["request"]["body"];
        assert_eq!(body["mode"], "raw");
        assert_eq!(body["raw"], "{}");
        assert_eq!(body["options"]["raw"]["language"], "json");
    }

    #[test]
    fn a_bearer_auth_is_exported() {
        let mut request = get("https://example.com");
        request.auth = Auth::Bearer {
            token: "{{token}}".to_string(),
        };
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let text = export_postman_collection("c", &tree);
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        let auth = &parsed["item"][0]["request"]["auth"];
        assert_eq!(auth["type"], "bearer");
        assert_eq!(auth["bearer"][0]["value"], "{{token}}");
    }

    #[test]
    fn a_disabled_pre_script_carries_a_note_but_keeps_running_text() {
        let mut request = get("https://example.com");
        request.pre_script = "vars.set(\"a\", 1);".to_string();
        request.pre_enabled = false;
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let text = export_postman_collection("c", &tree);
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        let exec = parsed["item"][0]["event"][0]["script"]["exec"]
            .as_array()
            .expect("exec array");
        let joined = exec
            .iter()
            .map(|line| line.as_str().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Turned off in Brasa"));
        assert!(joined.contains("vars.set(\"a\", 1);"));
    }

    #[test]
    fn no_scripts_means_no_event_array() {
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request: get("https://example.com"),
        }];
        let text = export_postman_collection("c", &tree);
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        assert!(parsed["item"][0].get("event").is_none());
    }
}
