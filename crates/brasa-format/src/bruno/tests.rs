//! End to end tests of [`parse_bruno_collection`]: whole collections in both formats.

use brasa_core::{Auth, Body, KeyValue, Method};
use pretty_assertions::assert_eq;

use super::*;
use crate::postman::{ImportFolder, RequestIssue};

fn file(path: &str, text: &str) -> BrunoFile {
    BrunoFile {
        path: path.to_string(),
        text: text.to_string(),
    }
}

fn request<'a>(folder: &'a ImportFolder, name: &str) -> &'a brasa_core::Request {
    &folder
        .requests
        .iter()
        .find(|(known, _)| known == name)
        .unwrap_or_else(|| panic!("no request {name}"))
        .1
}

fn issues<'a>(folder: &'a ImportFolder, name: &str) -> &'a [RequestIssue] {
    let index = folder
        .requests
        .iter()
        .position(|(known, _)| known == name)
        .expect("request");
    &folder.issues[index]
}

/// A `.bru` collection in a folder of the given archive, as Bruno writes it.
fn bru_collection(prefix: &str) -> Vec<BrunoFile> {
    vec![
        file(
            &format!("{prefix}bruno.json"),
            r#"{ "version": "1", "name": "Shop API", "type": "collection", "ignore": ["node_modules", ".git"] }"#,
        ),
        file(
            &format!("{prefix}collection.bru"),
            "headers {\n  X-Client: brasa-test\n}\n\nauth {\n  mode: bearer\n}\n\nauth:bearer {\n  token: {{token}}\n}\n\nvars:pre-request {\n  apiVersion: v2\n}\n\nscript:pre-request {\n  console.log('every request');\n}\n",
        ),
        file(
            &format!("{prefix}Login.bru"),
            "meta {\n  name: Login\n  type: http\n  seq: 1\n}\n\npost {\n  url: {{host}}/login\n  body: json\n  auth: none\n}\n\nbody:json {\n  {\n    \"user\": \"{{user}}\"\n  }\n}\n\nvars:post-response {\n  token: $res.body.token\n}\n\nassert {\n  $res.status: eq 200\n  $res.body.token: isString\n}\n",
        ),
        file(
            &format!("{prefix}users/folder.bru"),
            "meta {\n  name: Users\n  seq: 2\n}\n\nheaders {\n  Accept: application/json\n}\n",
        ),
        file(
            &format!("{prefix}users/Get user.bru"),
            "meta {\n  name: Get user\n  type: http\n  seq: 1\n}\n\nget {\n  url: {{host}}/users/:id?verbose=1\n  body: none\n  auth: inherit\n}\n\nparams:query {\n  verbose: 1\n  ~debug: true\n}\n\nparams:path {\n  id: 42\n}\n\nheaders {\n  \"X-Trace:Id\": abc\n}\n\ntests {\n  test('found', function () {\n    expect(res.status).to.equal(200);\n    expect(res.body.id).to.eql(42);\n  });\n}\n\ndocs {\n  Reads one user.\n}\n",
        ),
        file(
            &format!("{prefix}users/admin/Delete user.bru"),
            "meta {\n  name: Delete user\n  type: http\n  seq: 1\n}\n\ndelete {\n  url: {{host}}/users/1\n  body: none\n  auth: awsv4\n}\n\nscript:pre-request {\n  bru.sendRequest({ url: 'x' });\n  bru.setVar('deleted', true);\n}\n",
        ),
        file(
            &format!("{prefix}events/Stream.bru"),
            "meta {\n  name: Stream\n  type: ws\n  seq: 1\n}\n\nws {\n  url: wss://a.test\n}\n",
        ),
        file(
            &format!("{prefix}environments/Local.bru"),
            "vars {\n  host: http://localhost:8080\n  user: ann\n  ~old: 1\n}\nvars:secret [\n  token\n]\ncolor: #4CAF50\n",
        ),
        file(
            &format!("{prefix}environments/Local EU.bru"),
            "extends: Local\nvars {\n  host: http://eu.localhost:8080\n}\n",
        ),
        file(&format!("{prefix}.env"), "API_KEY=secret\n"),
        file(
            &format!("{prefix}node_modules/x/ignored.bru"),
            "meta {\n  name: Ignored\n}\n",
        ),
        file(&format!("{prefix}README.md"), "# Shop"),
    ]
}

#[test]
fn a_bru_collection() {
    let import = parse_bruno_collection(
        &bru_collection("shop/"),
        "fallback",
        &PostmanOptions {
            docs: true,
            ..PostmanOptions::default()
        },
    )
    .expect("collection");
    let plan = &import.plan;
    assert_eq!(plan.collection_name, "Shop API");
    assert_eq!(plan.variables, vec![KeyValue::new("apiVersion", "v2")]);
    assert!(
        plan.warnings.contains(
            &"collection: a pre-request script is not imported at that level, only per request"
                .to_string()
        ),
        "{:?}",
        plan.warnings
    );
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains(".env file"))
    );
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains("Stream: a ws request"))
    );

    let root = &plan.root;
    assert_eq!(root.requests.len(), 1);
    let login = request(root, "Login");
    assert_eq!(login.method, Method::Post);
    assert_eq!(login.url, "{{host}}/login");
    assert_eq!(login.auth, Auth::None);
    assert_eq!(
        login.headers,
        vec![KeyValue::new("X-Client", "brasa-test")]
    );
    assert_eq!(
        login.body,
        Body::Json("{\n  \"user\": \"{{user}}\"\n}".to_string())
    );
    assert_eq!(
        login.post_script,
        "env.set(\"token\", res.json().token);\n\ntest(\"$res.status: eq 200\", () => {\n  expect(res.status).toBe(200);\n});\ntest(\"$res.body.token: isString\", () => {\n  expect(typeof res.json().token).toBe(\"string\");\n});"
    );

    let names: Vec<&str> = root
        .folders
        .iter()
        .map(|folder| folder.name.as_str())
        .collect();
    assert_eq!(names, vec!["Users", "events"]);
    let users = &root.folders[0];
    let get_user = request(users, "Get user");
    assert_eq!(get_user.url, "{{host}}/users/42");
    assert_eq!(
        get_user.query,
        vec![
            KeyValue::new("verbose", "1"),
            KeyValue {
                key: "debug".to_string(),
                value: "true".to_string(),
                enabled: false
            }
        ]
    );
    assert_eq!(
        get_user.headers,
        vec![
            KeyValue::new("X-Client", "brasa-test"),
            KeyValue::new("Accept", "application/json"),
            KeyValue::new("X-Trace:Id", "abc"),
        ]
    );
    assert_eq!(
        get_user.auth,
        Auth::Bearer {
            token: "{{token}}".to_string()
        }
    );
    assert_eq!(
        get_user.post_script,
        "test('found', function () {\n  expect(res.status).toBe(200);\n  expect(res.json().id).toEqual(42);\n});"
    );
    assert_eq!(get_user.docs, "Reads one user.");
    assert_eq!(issues(users, "Get user"), &[RequestIssue::ScriptConverted]);

    let admin = &users.folders[0];
    assert_eq!(admin.name, "admin");
    let delete = request(admin, "Delete user");
    assert_eq!(delete.auth, Auth::None);
    assert!(
        delete
            .pre_script
            .contains("// bru.sendRequest({ url: 'x' });")
    );
    assert!(delete.pre_script.contains("env.set('deleted', true);"));
    assert_eq!(
        issues(admin, "Delete user"),
        &[
            RequestIssue::AuthUnsupported("awsv4".to_string()),
            RequestIssue::ScriptPartial {
                converted: 1,
                total: 2
            },
            RequestIssue::SendRequest,
        ]
    );

    assert_eq!(import.environments.len(), 2);
    let local = &import.environments[0];
    assert_eq!(local.name, "Local");
    assert_eq!(
        local.base,
        vec![
            KeyValue::new("host", "http://localhost:8080"),
            KeyValue::new("user", "ann")
        ]
    );
    assert_eq!(local.secret, vec![KeyValue::new("token", "")]);
    assert_eq!(local.warnings.len(), 1);
    let eu = &import.environments[1];
    assert_eq!(eu.name, "Local EU");
    assert_eq!(
        eu.base,
        vec![
            KeyValue::new("user", "ann"),
            KeyValue::new("host", "http://eu.localhost:8080")
        ]
    );
    assert_eq!(eu.secret, vec![KeyValue::new("token", "")]);
    assert_eq!(
        import.environment_colors,
        vec![("Local".to_string(), "#4CAF50".to_string())]
    );
}

#[test]
fn an_opencollection_yaml_collection() {
    let files = vec![
        file(
            "opencollection.yml",
            "opencollection: \"1.0.0\"\ninfo:\n  name: Billing\nrequest:\n  headers:\n    - name: X-Tenant\n      value: \"{{tenant}}\"\n  variables:\n    - name: tenant\n      value: acme\n",
        ),
        file(
            "invoices/folder.yml",
            "info:\n  name: Invoices\n  seq: 1\nrequest:\n  auth:\n    type: basic\n    username: billing\n    password: \"{{password}}\"\n",
        ),
        file(
            "invoices/create.yml",
            "info:\n  name: Create invoice\n  type: http\n  seq: 1\nhttp:\n  method: POST\n  url: \"{{host}}/invoices\"\n  body:\n    type: form-urlencoded\n    data:\n      - name: amount\n        value: \"10\"\n      - name: note\n        value: x\n        disabled: true\n  auth: inherit\nruntime:\n  scripts:\n    - type: tests\n      code: |-\n        test(\"created\", () => {\n          expect(res.getStatus()).to.equal(201);\n        });\n  assertions:\n    - expression: res.body.total\n      operator: gt\n      value: \"0\"\n",
        ),
        file(
            "Search.yml",
            "info:\n  name: Search\n  type: graphql\n  seq: 2\ngraphql:\n  method: POST\n  url: \"{{host}}/graphql\"\n  body:\n    query: \"{ invoices { id } }\"\n    variables: '{\"first\": 5}'\n",
        ),
        file(
            "environments/prod.yml",
            "name: Production\ncolor: \"#f44336\"\nvariables:\n  - name: host\n    value: https://billing.example\n  - name: password\n    secret: true\n",
        ),
    ];
    let import =
        parse_bruno_collection(&files, "billing", &PostmanOptions::default()).expect("collection");
    let plan = &import.plan;
    assert_eq!(plan.collection_name, "Billing");
    assert_eq!(plan.variables, vec![KeyValue::new("tenant", "acme")]);

    let search = request(&plan.root, "Search");
    assert_eq!(search.method, Method::Post);
    assert_eq!(
        search.body,
        Body::Json(
            "{\n  \"query\": \"{ invoices { id } }\",\n  \"variables\": {\"first\": 5}\n}"
                .to_string()
        )
    );
    assert_eq!(
        search.headers,
        vec![
            KeyValue::new("X-Tenant", "{{tenant}}"),
            KeyValue::new("Content-Type", "application/json")
        ]
    );

    let invoices = &plan.root.folders[0];
    let create = request(invoices, "Create invoice");
    assert_eq!(
        create.auth,
        Auth::Basic {
            username: "billing".to_string(),
            password: "{{password}}".to_string()
        }
    );
    assert_eq!(
        create.body,
        Body::Form(vec![
            KeyValue::new("amount", "10"),
            KeyValue {
                key: "note".to_string(),
                value: "x".to_string(),
                enabled: false
            }
        ])
    );
    assert_eq!(
        create.post_script,
        "test(\"res.body.total: gt 0\", () => {\n  expect(res.json().total).toBeGreaterThan(0);\n});\n\ntest(\"created\", () => {\n  expect(res.status).toBe(201);\n});"
    );
    assert_eq!(create.docs, "");

    let prod = &import.environments[0];
    assert_eq!(prod.name, "Production");
    assert_eq!(
        prod.base,
        vec![KeyValue::new("host", "https://billing.example")]
    );
    assert_eq!(prod.secret, vec![KeyValue::new("password", "")]);
    assert_eq!(
        import.environment_colors,
        vec![("Production".to_string(), "#f44336".to_string())]
    );
}

#[test]
fn options_flatten_folders_and_drop_variables_auth_and_secrets() {
    let options = PostmanOptions {
        folders: false,
        variables: false,
        secrets: false,
        auth: false,
        scripts: true,
        docs: false,
    };
    let import = parse_bruno_collection(&bru_collection(""), "x", &options).expect("collection");
    let root = &import.plan.root;
    assert!(root.folders.is_empty());
    let names: Vec<&str> = root
        .requests
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(names, vec!["Login", "Get user", "Delete user"]);
    assert_eq!(request(root, "Get user").auth, Auth::None);
    assert_eq!(request(root, "Get user").docs, "");
    assert!(import.plan.variables.is_empty());
    assert!(import.environments.iter().all(|env| env.secret.is_empty()));
}

#[test]
fn single_request_files_and_what_is_not_a_collection() {
    let import = parse_bruno_collection(
        &[file("ping.bru", "get {\n  url: https://a.test/ping\n}\n")],
        "ping",
        &PostmanOptions::default(),
    )
    .expect("one request");
    assert_eq!(import.plan.collection_name, "ping");
    assert_eq!(
        request(&import.plan.root, "ping").url,
        "https://a.test/ping"
    );

    assert_eq!(
        parse_bruno_collection(&[file("notes.txt", "hi")], "x", &PostmanOptions::default()),
        Err(BrunoError::NotACollection)
    );
}

#[test]
fn detection() {
    assert!(is_collection_root("a/b/bruno.json"));
    assert!(is_collection_root("opencollection.yml"));
    assert!(!is_collection_root("package.json"));
    assert!(is_bruno_file("x.bru", "meta {\n  name: X\n}\n"));
    assert!(is_bruno_file("dev.bru", "vars {\n  a: 1\n}\n"));
    assert!(is_bruno_file("ping.bru", "get {\n  url: /ping\n}\n"));
    assert!(!is_bruno_file("x.bru", "random text"));
    assert!(is_bruno_file(
        "r.yml",
        "info:\n  name: R\nhttp:\n  method: GET\n  url: /\n"
    ));
    assert!(!is_bruno_file(
        "openapi.yml",
        "openapi: 3.1.0\ninfo:\n  title: A\n"
    ));
}
