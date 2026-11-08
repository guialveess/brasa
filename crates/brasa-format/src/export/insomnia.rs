//! Writes an [`ExportNode`] tree as an Insomnia v4 export document: `{"_type": "export", ...,
//! "resources": [...]}`, a flat array of `workspace`/`request_group`/`request` objects linked by
//! `parentId`. Field names confirmed against Kong/insomnia's own model source
//! (`packages/insomnia-data/src/models/request.ts` and `request-group.ts`): a header is `{name,
//! value, disabled}` (not `key`), a body is `{mimeType, text, params}`, scripts are the request's
//! own `preRequestScript`/`afterResponseScript` strings, and every resource needs a numeric
//! `metaSortKey`.

use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, MultipartValue, Request, multipart_value};
use serde_json::{Map, Value, json};

use super::ExportNode;

const EXPORT_FORMAT: u32 = 4;
const EXPORT_SOURCE: &str = "brasa.export:v1.0";

/// Exports `nodes` (the direct children of the folder or workspace being exported) as an
/// Insomnia v4 export document named `name`, pretty printed. `exported_at` is an RFC 3339
/// timestamp (`__export_date`); the caller supplies it so this stays free of a clock.
pub fn export_insomnia_collection(name: &str, nodes: &[ExportNode], exported_at: &str) -> String {
    let workspace_id = "wrk_1".to_string();
    let mut resources = vec![json!({
        "_id": workspace_id,
        "_type": "workspace",
        "parentId": Value::Null,
        "name": name,
        "description": "",
        "scope": "collection",
    })];
    let mut next_id = 2u32;
    collect(nodes, &workspace_id, &mut next_id, &mut resources);

    let document = json!({
        "_type": "export",
        "__export_format": EXPORT_FORMAT,
        "__export_date": exported_at,
        "__export_source": EXPORT_SOURCE,
        "resources": resources,
    });
    serde_json::to_string_pretty(&document).unwrap_or_default()
}

/// Appends the resources of `nodes` (children of `parent_id`) to `out`, assigning each one the
/// next id from `next_id` and recursing into subfolders.
fn collect(nodes: &[ExportNode], parent_id: &str, next_id: &mut u32, out: &mut Vec<Value>) {
    for (index, node) in nodes.iter().enumerate() {
        let sort_key = index as i64;
        match node {
            ExportNode::Folder { name, children } => {
                let id = format!("fld_{next_id}");
                *next_id += 1;
                out.push(json!({
                    "_id": id,
                    "_type": "request_group",
                    "parentId": parent_id,
                    "name": name,
                    "environment": {},
                    "metaSortKey": sort_key,
                }));
                collect(children, &id, next_id, out);
            }
            ExportNode::Request { name, request } => {
                let id = format!("req_{next_id}");
                *next_id += 1;
                out.push(export_request(&id, parent_id, name, sort_key, request));
            }
        }
    }
}

/// Exports one request resource.
fn export_request(id: &str, parent_id: &str, name: &str, sort_key: i64, request: &Request) -> Value {
    let mut resource = Map::new();
    resource.insert("_id".to_string(), json!(id));
    resource.insert("_type".to_string(), json!("request"));
    resource.insert("parentId".to_string(), json!(parent_id));
    resource.insert("name".to_string(), json!(name));
    resource.insert("method".to_string(), json!(request.method.to_string()));
    resource.insert("url".to_string(), json!(request.url));
    resource.insert("metaSortKey".to_string(), json!(sort_key));
    resource.insert(
        "headers".to_string(),
        json!(request.headers.iter().map(export_header).collect::<Vec<_>>()),
    );
    resource.insert(
        "parameters".to_string(),
        json!(request.query.iter().map(export_header).collect::<Vec<_>>()),
    );
    resource.insert("body".to_string(), export_body(&request.body));
    resource.insert("authentication".to_string(), export_auth(&request.auth));
    if !request.pre_script.is_empty() {
        resource.insert(
            "preRequestScript".to_string(),
            json!(script_text(&request.pre_script, request.pre_enabled)),
        );
    }
    if !request.post_script.is_empty() {
        resource.insert(
            "afterResponseScript".to_string(),
            json!(script_text(&request.post_script, request.post_enabled)),
        );
    }
    if !request.docs.is_empty() {
        resource.insert("description".to_string(), json!(request.docs));
    }
    Value::Object(resource)
}

/// Exports one header or query parameter entry: `{name, value, disabled}`, the shape shared by
/// `RequestHeader` and `RequestParameter`.
fn export_header(entry: &KeyValue) -> Value {
    json!({
        "name": entry.key,
        "value": entry.value,
        "disabled": !entry.enabled,
    })
}

/// Exports [`Body`] as Insomnia's `body` object, `{}` for [`Body::None`] (its own default, see
/// `init()` in `request.ts`).
fn export_body(body: &Body) -> Value {
    match body {
        Body::None => json!({}),
        Body::Json(text) => json!({ "mimeType": "application/json", "text": text }),
        Body::Text(text) => json!({ "mimeType": "text/plain", "text": text }),
        Body::Xml(text) => json!({ "mimeType": "application/xml", "text": text }),
        Body::Form(fields) => json!({
            "mimeType": "application/x-www-form-urlencoded",
            "params": fields.iter().map(export_body_param).collect::<Vec<_>>(),
        }),
        Body::Multipart(fields) => json!({
            "mimeType": "multipart/form-data",
            "params": fields.iter().map(export_body_param).collect::<Vec<_>>(),
        }),
    }
}

/// Exports one `body.params` entry: a file field carries its path in `fileName` and an empty
/// `value` with `type: "file"`, a text field its value with `type: "text"`, mirroring
/// [`multipart_value`].
fn export_body_param(field: &KeyValue) -> Value {
    match multipart_value(&field.value) {
        MultipartValue::File(path) => json!({
            "name": field.key,
            "type": "file",
            "fileName": path,
            "disabled": !field.enabled,
        }),
        MultipartValue::Text(text) => json!({
            "name": field.key,
            "type": "text",
            "value": text,
            "disabled": !field.enabled,
        }),
    }
}

/// Exports [`Auth`] as Insomnia's `authentication` object, `{}` for [`Auth::None`] (its own
/// default).
fn export_auth(auth: &Auth) -> Value {
    match auth {
        Auth::None => json!({}),
        Auth::Basic { username, password } => json!({
            "type": "basic",
            "username": username,
            "password": password,
        }),
        Auth::Bearer { token } => json!({
            "type": "bearer",
            "token": token,
        }),
        Auth::ApiKey {
            name,
            value,
            location,
        } => {
            let add_to = match location {
                ApiKeyLocation::Header => "header",
                ApiKeyLocation::Query => "queryParams",
            };
            json!({
                "type": "apikey",
                "key": name,
                "value": value,
                "addTo": add_to,
            })
        }
    }
}

/// Prepended to an exported script, since Brasa's script API is not Insomnia's own
/// pre-request/after-response script context, see `super::postman`'s identical banner.
const SCRIPT_BANNER: &str =
    "// Written for Brasa's script API (docs/scripting.md), not Insomnia's own script context. Adapt before running here.";
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
    fn the_root_object_has_the_export_envelope() {
        let text = export_insomnia_collection("c", &[], "2026-01-01T00:00:00Z");
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(parsed["_type"], "export");
        assert_eq!(parsed["__export_format"], 4);
        assert_eq!(parsed["__export_date"], "2026-01-01T00:00:00Z");
        assert_eq!(parsed["resources"][0]["_type"], "workspace");
        assert_eq!(parsed["resources"][0]["name"], "c");
        assert_eq!(parsed["resources"][0]["scope"], "collection");
    }

    #[test]
    fn a_folder_becomes_a_request_group_linked_to_its_parent() {
        let tree = vec![ExportNode::Folder {
            name: "US001".to_string(),
            children: vec![ExportNode::Request {
                name: "list users".to_string(),
                request: get("https://example.com/users"),
            }],
        }];
        let text = export_insomnia_collection("c", &tree, "2026-01-01T00:00:00Z");
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        let resources = parsed["resources"].as_array().expect("array");
        assert_eq!(resources.len(), 3);
        let workspace_id = resources[0]["_id"].as_str().expect("id").to_string();
        assert_eq!(resources[1]["_type"], "request_group");
        assert_eq!(resources[1]["name"], "US001");
        assert_eq!(resources[1]["parentId"], workspace_id);
        let folder_id = resources[1]["_id"].as_str().expect("id").to_string();
        assert_eq!(resources[2]["_type"], "request");
        assert_eq!(resources[2]["name"], "list users");
        assert_eq!(resources[2]["parentId"], folder_id);
    }

    #[test]
    fn headers_and_parameters_use_name_not_key() {
        let mut request = get("https://example.com");
        request.headers.push(KeyValue::new("X-Debug", "1"));
        request.query.push(KeyValue::new("page", "1"));
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let text = export_insomnia_collection("c", &tree, "2026-01-01T00:00:00Z");
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        let request = &parsed["resources"][1];
        assert_eq!(request["headers"][0]["name"], "X-Debug");
        assert_eq!(request["parameters"][0]["name"], "page");
    }

    #[test]
    fn a_json_body_sets_its_mime_type_and_text() {
        let mut request = get("https://example.com");
        request.method = Method::Post;
        request.body = Body::Json("{}".to_string());
        let tree = vec![ExportNode::Request {
            name: "r".to_string(),
            request,
        }];
        let text = export_insomnia_collection("c", &tree, "2026-01-01T00:00:00Z");
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        let body = &parsed["resources"][1]["body"];
        assert_eq!(body["mimeType"], "application/json");
        assert_eq!(body["text"], "{}");
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
        let text = export_insomnia_collection("c", &tree, "2026-01-01T00:00:00Z");
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        let auth = &parsed["resources"][1]["authentication"];
        assert_eq!(auth["type"], "bearer");
        assert_eq!(auth["token"], "{{token}}");
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
        let text = export_insomnia_collection("c", &tree, "2026-01-01T00:00:00Z");
        let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
        let script = parsed["resources"][1]["preRequestScript"]
            .as_str()
            .expect("string");
        assert!(script.contains("Turned off in Brasa"));
        assert!(script.contains("vars.set(\"a\", 1);"));
    }
}
