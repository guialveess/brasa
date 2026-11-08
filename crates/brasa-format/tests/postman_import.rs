//! Mapping tests for `brasa_format::postman` (see `docs/postman-import.md`).
//!
//! `tests/fixtures/postman_collection.json` is a realistic collection: nested folders, every
//! body mode, auth inheritance (collection, folder and per-request `inherit`/`noauth`),
//! disabled headers, pre/test scripts, collection variables and both description forms.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use brasa_core::{ApiKeyLocation, Auth, Body, KeyValue, Method};
use brasa_format::postman::{
    ImportFolder, IssueLevel, PostmanOptions, RequestIssue, parse_collection, parse_environment,
};
use pretty_assertions::assert_eq;

const COLLECTION: &str = include_str!("fixtures/postman_collection.json");
const ENVIRONMENT: &str = include_str!("fixtures/postman_environment.json");

/// The "Auth" folder's basic auth (Postman variables are kept as is).
fn auth_folder_basic() -> Auth {
    Auth::Basic {
        username: "admin".to_string(),
        password: "{{adminPass}}".to_string(),
    }
}

/// The collection's bearer auth.
fn collection_bearer() -> Auth {
    Auth::Bearer {
        token: "{{collectionToken}}".to_string(),
    }
}

fn find_folder<'a>(folders: &'a [ImportFolder], name: &str) -> &'a ImportFolder {
    folders
        .iter()
        .find(|folder| folder.name == name)
        .unwrap_or_else(|| panic!("folder {name:?} not found"))
}

/// The options with everything on, the docs included.
fn everything() -> PostmanOptions {
    PostmanOptions {
        docs: true,
        ..PostmanOptions::default()
    }
}

#[test]
fn collection_name_and_top_level_shape() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    assert_eq!(plan.collection_name, "Demo API");
    assert_eq!(plan.root.folders.len(), 2);
    assert_eq!(plan.root.requests.len(), 1);
    assert_eq!(plan.root.requests[0].0, "Ping");
}

#[test]
fn collection_variables_become_an_environment() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    assert_eq!(
        plan.variables,
        vec![
            KeyValue::new("baseUrl", "https://api.example.com"),
            KeyValue::new("collectionToken", "secret-collection-token"),
        ]
    );
}

#[test]
fn root_request_inherits_collection_bearer_auth() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let (name, ping) = &plan.root.requests[0];
    assert_eq!(name, "Ping");
    assert_eq!(ping.method, Method::Get);
    assert_eq!(ping.url, "{{baseUrl}}/ping");
    assert!(ping.query.is_empty());
    assert_eq!(ping.body, Body::None);
    assert_eq!(ping.docs, "");
    assert_eq!(ping.pre_script, "");
    assert_eq!(ping.post_script, "");
    assert!(ping.headers.is_empty());
    assert_eq!(ping.auth, collection_bearer());
}

#[test]
fn login_request_maps_headers_body_docs_and_folder_auth() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let auth_folder = find_folder(&plan.root.folders, "Auth");
    let (name, login) = auth_folder
        .requests
        .iter()
        .find(|(name, _)| name == "Login")
        .expect("Login request");
    assert_eq!(name, "Login");
    assert_eq!(login.method, Method::Post);
    assert_eq!(login.url, "{{baseUrl}}/login");
    assert_eq!(login.query, vec![KeyValue::new("debug", "1")]);
    assert_eq!(
        login.body,
        Body::Json("{ \"user\": \"{{username}}\" }".to_string())
    );
    assert_eq!(login.docs, "Login endpoint");

    let mut debug_header = KeyValue::new("X-Debug", "1");
    debug_header.enabled = false;
    assert_eq!(
        login.headers,
        vec![
            KeyValue::new("Content-Type", "application/json"),
            debug_header,
        ]
    );
    assert_eq!(login.auth, auth_folder_basic());
}

#[test]
fn login_scripts_are_converted() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let auth_folder = find_folder(&plan.root.folders, "Auth");
    let (_, login) = auth_folder
        .requests
        .iter()
        .find(|(name, _)| name == "Login")
        .expect("Login request");

    assert_eq!(login.pre_script, "env.set('ts', Date.now());");
    assert_eq!(
        login.post_script,
        "test('status is 200', function () {\n  expect(res.status).toBe(200);\n});"
    );
}

#[test]
fn scripts_are_commented_out_when_the_option_is_off() {
    let options = PostmanOptions {
        scripts: false,
        ..everything()
    };
    let plan = parse_collection(COLLECTION, &options).expect("valid fixture collection");
    let auth_folder = find_folder(&plan.root.folders, "Auth");
    let (index, (_, login)) = auth_folder
        .requests
        .iter()
        .enumerate()
        .find(|(_, (name, _))| name == "Login")
        .expect("Login request");
    assert_eq!(
        login.pre_script,
        "// Imported from Postman. Scripts were not converted, adapt the pm.* calls.\n\
         // pm.environment.set('ts', Date.now());"
    );
    assert_eq!(
        auth_folder.issues[index],
        vec![RequestIssue::ScriptNotConverted]
    );
}

#[test]
fn form_data_becomes_multipart_with_its_files() {
    let text = r#"{"info": {"name": "x"}, "item": [{"name": "Up", "request": {"method": "POST",
      "url": "http://x", "body": {"mode": "formdata", "formdata": [
        {"key": "note", "value": "hi", "type": "text"},
        {"key": "handle", "value": "@me", "type": "text"},
        {"key": "off", "value": "x", "type": "text", "disabled": true},
        {"key": "doc", "type": "file", "src": "/home/ana/doc.pdf"},
        {"key": "many", "type": "file", "src": ["/a.png", "/b.png"]}
      ]}}}]}"#;
    let plan = parse_collection(text, &everything()).expect("valid collection");
    let mut off = KeyValue::new("off", "x");
    off.enabled = false;
    assert_eq!(
        plan.root.requests[0].1.body,
        Body::Multipart(vec![
            KeyValue::new("note", "hi"),
            KeyValue::new("handle", "@@me"),
            off,
            KeyValue::new("doc", "@/home/ana/doc.pdf"),
            KeyValue::new("many", "@/a.png"),
        ])
    );
    assert_eq!(
        plan.root.issues[0],
        vec![RequestIssue::FormDataFile, RequestIssue::FormDataManyFiles]
    );
}

#[test]
fn oauth2_sends_its_access_token_from_a_local_variable() {
    let text = r#"{"info": {"name": "x"}, "auth": {"type": "oauth2", "oauth2": [
        {"key": "accessToken", "value": "tok"}, {"key": "tokenType", "value": "Bearer"}
      ]}, "item": [{"name": "A", "request": {"method": "GET", "url": "http://x"}}]}"#;
    let plan = parse_collection(text, &everything()).expect("valid collection");
    assert_eq!(
        plan.root.requests[0].1.auth,
        Auth::Bearer {
            token: "{{oauth2AccessToken}}".to_string()
        }
    );
    assert_eq!(
        plan.secret_variables,
        vec![KeyValue::new("oauth2AccessToken", "tok")]
    );
    assert_eq!(plan.root.issues[0], vec![RequestIssue::OAuth2Token]);
    assert_eq!(RequestIssue::OAuth2Token.level(), IssueLevel::Warning);
}

#[test]
fn an_auth_brasa_does_not_have_is_unsupported() {
    let text = r#"{"info": {"name": "x"}, "item": [{"name": "A", "request": {"method": "GET",
      "url": "http://x", "auth": {"type": "hawk"}}}]}"#;
    let plan = parse_collection(text, &everything()).expect("valid collection");
    assert_eq!(plan.root.requests[0].1.auth, Auth::None);
    let issue = &plan.root.issues[0][0];
    assert_eq!(issue, &RequestIssue::AuthUnsupported("hawk".to_string()));
    assert_eq!(issue.level(), IssueLevel::Unsupported);
}

#[test]
fn descriptions_and_examples_go_in_the_docs_only_with_the_option() {
    let text = r#"{"info": {"name": "x"}, "item": [{"name": "A",
      "request": {"method": "GET", "url": "http://x", "description": "What it does"},
      "response": [{"name": "Found", "code": 200, "body": "{}"}]}]}"#;
    let plan = parse_collection(text, &everything()).expect("valid collection");
    assert_eq!(
        plan.root.requests[0].1.docs,
        "What it does\n\n## Found (200)\n\n```\n{}\n```"
    );
    let plan = parse_collection(text, &PostmanOptions::default()).expect("valid collection");
    assert_eq!(plan.root.requests[0].1.docs, "");
}

#[test]
fn nested_users_folder_inherits_auth_through_its_parent_folder() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let auth_folder = find_folder(&plan.root.folders, "Auth");
    let users_folder = find_folder(&auth_folder.folders, "Users");

    let (_, list_users) = users_folder
        .requests
        .iter()
        .find(|(name, _)| name == "List users")
        .expect("List users request");
    assert_eq!(list_users.url, "{{baseUrl}}/users");
    assert_eq!(
        list_users.query,
        vec![KeyValue::new("page", "2"), KeyValue::new("limit", "10")]
    );
    assert_eq!(list_users.docs, "Returns paginated users");
    assert_eq!(
        list_users.auth,
        auth_folder_basic(),
        "an \"inherit\" auth must pick up the parent folder's auth"
    );
}

#[test]
fn explicit_noauth_request_gets_no_auth() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let auth_folder = find_folder(&plan.root.folders, "Auth");
    let users_folder = find_folder(&auth_folder.folders, "Users");

    let (_, create_user) = users_folder
        .requests
        .iter()
        .find(|(name, _)| name == "Create user")
        .expect("Create user request");
    assert!(create_user.headers.is_empty());
    assert_eq!(create_user.auth, Auth::None);
    assert_eq!(
        create_user.body,
        Body::Text("plain body {{name}}".to_string())
    );
}

#[test]
fn every_body_mode_maps_as_specified() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let uploads = find_folder(&plan.root.folders, "Uploads");
    let body_of = |name: &str| {
        uploads
            .requests
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} request"))
            .1
            .body
            .clone()
    };

    let mut disabled_b = KeyValue::new("b", "2");
    disabled_b.enabled = false;
    assert_eq!(
        body_of("Form encoded"),
        Body::Form(vec![KeyValue::new("a", "1"), disabled_b])
    );
    assert!(matches!(body_of("Multipart"), Body::Multipart(_)));
    assert_eq!(body_of("Raw file"), Body::None);
    assert_eq!(body_of("GraphQL query"), Body::None);
    assert_eq!(
        body_of("Xml payload"),
        Body::Xml("<ping>{{name}}</ping>".to_string())
    );
}

#[test]
fn unsupported_body_modes_produce_a_warning_each() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    for keyword in ["file", "graphql"] {
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains(keyword)),
            "expected a warning mentioning {keyword:?}, got {:?}",
            plan.warnings
        );
    }
}

#[test]
fn apikey_auth_in_header_location_is_kept() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let uploads = find_folder(&plan.root.folders, "Uploads");
    let (_, request) = uploads
        .requests
        .iter()
        .find(|(name, _)| name == "Api key header")
        .expect("Api key header request");
    assert!(request.headers.is_empty());
    assert_eq!(
        request.auth,
        Auth::ApiKey {
            name: "X-Api-Key".to_string(),
            value: "{{apiKey}}".to_string(),
            location: ApiKeyLocation::Header,
        }
    );
}

#[test]
fn apikey_auth_in_query_location_is_kept() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let uploads = find_folder(&plan.root.folders, "Uploads");
    let (_, request) = uploads
        .requests
        .iter()
        .find(|(name, _)| name == "Api key query")
        .expect("Api key query request");
    assert!(request.query.is_empty());
    assert_eq!(
        request.auth,
        Auth::ApiKey {
            name: "token".to_string(),
            value: "{{apiKey}}".to_string(),
            location: ApiKeyLocation::Query,
        }
    );
    assert!(
        !plan
            .warnings
            .iter()
            .any(|warning| warning.contains("apikey")),
        "{:?}",
        plan.warnings
    );
}

#[test]
fn folder_without_its_own_auth_inherits_the_collection_auth() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");
    let uploads = find_folder(&plan.root.folders, "Uploads");
    let (_, form_encoded) = uploads
        .requests
        .iter()
        .find(|(name, _)| name == "Form encoded")
        .expect("Form encoded request");
    assert!(form_encoded.headers.is_empty());
    assert_eq!(form_encoded.auth, collection_bearer());
}

#[test]
fn environment_export_splits_base_and_secret_values() {
    let environment = parse_environment(ENVIRONMENT).expect("valid fixture environment");
    assert_eq!(environment.name, "Demo");
    assert_eq!(
        environment.base,
        vec![KeyValue::new("baseUrl", "https://api.example.com")]
    );
    assert_eq!(
        environment.secret,
        vec![KeyValue::new("apiToken", "top-secret-token")]
    );
    assert!(environment.warnings.is_empty());
}

#[test]
fn environment_export_skips_disabled_values() {
    let environment = parse_environment(ENVIRONMENT).expect("valid fixture environment");
    let all_keys: Vec<&str> = environment
        .base
        .iter()
        .chain(environment.secret.iter())
        .map(|entry| entry.key.as_str())
        .collect();
    assert!(!all_keys.contains(&"disabledVar"));
}

#[test]
fn environment_without_a_name_falls_back_and_warns() {
    let environment = parse_environment(r#"{"values": []}"#).expect("valid minimal environment");
    assert_eq!(environment.name, "imported");
    assert_eq!(environment.warnings.len(), 1);
}

#[test]
fn invalid_json_is_a_parse_error() {
    assert!(parse_collection("not json", &everything()).is_err());
    assert!(parse_environment("not json").is_err());
}

#[test]
fn every_imported_request_parses_back_with_brasa_format() {
    let plan = parse_collection(COLLECTION, &everything()).expect("valid fixture collection");

    fn check_folder(folder: &ImportFolder) {
        for (name, request) in &folder.requests {
            let text = brasa_format::serialize(request);
            brasa_format::parse(&text)
                .unwrap_or_else(|error| panic!("request {name:?} must parse back: {error}"));
        }
        for child in &folder.folders {
            check_folder(child);
        }
    }
    check_folder(&plan.root);
}

#[test]
fn collection_variables_convert_non_string_scalars_and_warn_on_complex_values() {
    let json = r#"{
        "info": { "name": "Scalars" },
        "variable": [
            { "key": "count", "value": 42 },
            { "key": "enabled", "value": true },
            { "key": "empty", "value": null },
            { "key": "list", "value": [1, 2] }
        ],
        "item": []
    }"#;
    let plan = parse_collection(json, &everything()).expect("valid collection");
    assert_eq!(
        plan.variables,
        vec![
            KeyValue::new("count", "42"),
            KeyValue::new("enabled", "true"),
            KeyValue::new("empty", ""),
            KeyValue::new("list", "[1,2]"),
        ]
    );
    assert_eq!(plan.warnings.len(), 1, "only the array value should warn");
    assert!(plan.warnings[0].contains("list"));
}

#[test]
fn environment_values_convert_non_string_scalars_and_warn_on_complex_values() {
    let json = r#"{
        "name": "Scalars",
        "values": [
            { "key": "count", "value": 42, "enabled": true },
            { "key": "flag", "value": false, "enabled": true },
            { "key": "empty", "value": null, "enabled": true },
            { "key": "obj", "value": { "a": 1 }, "enabled": true }
        ]
    }"#;
    let environment = parse_environment(json).expect("valid environment");
    assert_eq!(
        environment.base,
        vec![
            KeyValue::new("count", "42"),
            KeyValue::new("flag", "false"),
            KeyValue::new("empty", ""),
            KeyValue::new("obj", "{\"a\":1}"),
        ]
    );
    assert_eq!(
        environment.warnings.len(),
        1,
        "only the object value should warn"
    );
    assert!(environment.warnings[0].contains("obj"));
}

#[test]
fn warns_about_dropped_collection_and_folder_level_event_scripts() {
    let json = r#"{
        "info": { "name": "Scripted" },
        "event": [
            { "listen": "prerequest", "script": { "exec": ["console.log('collection pre')"] } }
        ],
        "item": [
            {
                "name": "Group",
                "event": [
                    { "listen": "test", "script": { "exec": ["console.log('folder test')"] } }
                ],
                "item": []
            }
        ]
    }"#;
    let plan = parse_collection(json, &everything()).expect("valid collection");
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains("collection")
                && warning.contains("Scripted")
                && warning.contains("prerequest")),
        "expected a collection-level warning, got {:?}",
        plan.warnings
    );
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains("folder")
                && warning.contains("Group")
                && warning.contains("test")),
        "expected a folder-level warning, got {:?}",
        plan.warnings
    );
}
