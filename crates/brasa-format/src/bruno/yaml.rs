//! Reading the OpenCollection YAML files of a Bruno collection (Bruno 3.0 and later) into the
//! shapes of [`super::model`]. The structure follows what Bruno writes
//! (`packages/bruno-converters/src/opencollection` in usebruno/bruno): `info`, then `http` (or
//! `graphql`) with `method`, `url`, `params`, `headers`, `body` and `auth`, then `runtime` with
//! `variables`, `scripts`, `assertions` and `actions`, and `docs`.

use brasa_core::KeyValue;
use serde_json::Value;

use super::model::{Defaults, MultipartEntry, RawAuth, RawBody, RawEnvironment, RawRequest};

/// Parses YAML text into a JSON value, `None` when it is not valid YAML.
pub(super) fn parse_yaml(text: &str) -> Option<Value> {
    serde_norway::from_str(text).ok()
}

/// A scalar as text: strings as they are, numbers and booleans written out, a typed variable
/// value (`{ type, data }`) by its data.
fn text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Object(object)) => text(object.get("data").or_else(|| object.get("value"))),
        _ => String::new(),
    }
}

fn str_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn disabled(entry: &Value) -> bool {
    entry.get("disabled").and_then(Value::as_bool) == Some(true)
        || entry.get("enabled").and_then(Value::as_bool) == Some(false)
}

/// A list of `{ name, value, disabled }` entries, keeping only those `keep` accepts.
fn entries(list: Option<&Value>, keep: impl Fn(&Value) -> bool) -> Vec<KeyValue> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| keep(entry))
        .map(|entry| KeyValue {
            key: text(entry.get("name")),
            value: text(entry.get("value")),
            enabled: !disabled(entry),
        })
        .collect()
}

fn auth(value: Option<&Value>) -> Option<RawAuth> {
    let value = value?;
    let kind = match value {
        Value::String(kind) => kind.as_str(),
        _ => str_at(value, "type"),
    };
    Some(match kind {
        "" | "none" => RawAuth::None,
        "inherit" => RawAuth::Inherit,
        "basic" => RawAuth::Basic {
            username: text(value.get("username")),
            password: text(value.get("password")),
        },
        "bearer" => RawAuth::Bearer {
            token: text(value.get("token")),
        },
        "apikey" => RawAuth::ApiKey {
            key: text(value.get("key")),
            value: text(value.get("value")),
            query: matches!(str_at(value, "placement"), "query" | "queryparams"),
        },
        "oauth2" => RawAuth::OAuth2,
        other => RawAuth::Other(other.to_string()),
    })
}

fn body(value: Option<&Value>) -> RawBody {
    let Some(value) = value else {
        return RawBody::None;
    };
    // What Bruno writes is `{ type, data }`; older files say `{ mode, <mode>: data }`.
    let kind = match str_at(value, "type") {
        "" => str_at(value, "mode"),
        kind => kind,
    };
    let data = value.get("data").or_else(|| value.get(kind));
    let raw = || text(data);
    let form = || entries(data, |_| true);
    match kind {
        "" | "none" => RawBody::None,
        "json" => RawBody::Json(raw()),
        "text" => RawBody::Text(raw()),
        "xml" => RawBody::Xml(raw()),
        "form-urlencoded" | "formUrlEncoded" => RawBody::Form(form()),
        "multipart-form" | "multipartForm" => RawBody::Multipart(
            data.and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|entry| {
                    let is_file = str_at(entry, "type") == "file";
                    let files = is_file.then(|| match entry.get("value") {
                        Some(Value::Array(paths)) => paths
                            .iter()
                            .map(|path| text(Some(path)))
                            .filter(|path| !path.is_empty())
                            .collect(),
                        other => vec![text(other)],
                    });
                    MultipartEntry {
                        field: KeyValue {
                            key: text(entry.get("name")),
                            value: if is_file {
                                String::new()
                            } else {
                                text(entry.get("value"))
                            },
                            enabled: !disabled(entry),
                        },
                        files,
                    }
                })
                .collect(),
        ),
        other => RawBody::Other(other.to_string()),
    }
}

/// The scripts a folder or the collection has, which Brasa does not run at that level.
fn script_kinds(scripts: Option<&Value>) -> Vec<&'static str> {
    let mut kinds = Vec::new();
    for script in scripts.and_then(Value::as_array).into_iter().flatten() {
        if text(script.get("code")).trim().is_empty() {
            continue;
        }
        let kind = match str_at(script, "type") {
            "before-request" => "pre-request script",
            "after-response" => "post-response script",
            "tests" => "test script",
            _ => continue,
        };
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

/// Reads the `request` defaults of `opencollection.yml` or `folder.yml`, and its `docs`.
pub(super) fn yaml_defaults(value: &Value) -> Defaults {
    let request = value.get("request").unwrap_or(&Value::Null);
    Defaults {
        headers: entries(request.get("headers"), |_| true),
        auth: auth(request.get("auth")).filter(|auth| *auth != RawAuth::Inherit),
        vars: entries(request.get("variables"), |_| true),
        scripts: script_kinds(request.get("scripts")),
        docs: docs(value.get("docs")),
    }
}

/// The docs, a string or `{ content }`.
fn docs(value: Option<&Value>) -> String {
    match value {
        Some(Value::Object(object)) => text(object.get("content")),
        other => text(other),
    }
}

/// The name and place of a folder (`info` of `folder.yml`) or a collection.
pub(super) fn yaml_info(value: &Value) -> (Option<String>, f64) {
    let info = value.get("info").unwrap_or(&Value::Null);
    let name = info.get("name").and_then(Value::as_str).map(str::to_string);
    let seq = info.get("seq").and_then(Value::as_f64).unwrap_or(f64::MAX);
    (name, seq)
}

/// Reads a request file. `fallback` names it when it has no `info.name`.
pub(super) fn yaml_request(value: &Value, fallback: &str) -> RawRequest {
    let (name, seq) = yaml_info(value);
    let info = value.get("info").unwrap_or(&Value::Null);
    let kind = match str_at(info, "type") {
        "" => "http",
        kind => kind,
    };
    let details = value
        .get("http")
        .or_else(|| value.get("graphql"))
        .unwrap_or(&Value::Null);
    let runtime = value.get("runtime").unwrap_or(&Value::Null);
    let mut scripts = (String::new(), String::new(), String::new());
    for script in runtime
        .get("scripts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let code = text(script.get("code"));
        match str_at(script, "type") {
            "before-request" => scripts.0 = code,
            "after-response" => scripts.1 = code,
            "tests" => scripts.2 = code,
            _ => {}
        }
    }
    let body = if kind == "graphql" {
        let graphql = details.get("body").unwrap_or(&Value::Null);
        RawBody::GraphQl {
            query: text(graphql.get("query")),
            variables: text(graphql.get("variables")),
        }
    } else {
        body(details.get("body"))
    };
    RawRequest {
        name: name.unwrap_or_else(|| fallback.to_string()),
        seq,
        kind: kind.to_string(),
        method: match str_at(details, "method") {
            "" => "GET".to_string(),
            method => method.to_uppercase(),
        },
        url: text(details.get("url")),
        query: entries(details.get("params"), |param| {
            matches!(str_at(param, "type"), "" | "query")
        }),
        path: entries(details.get("params"), |param| {
            str_at(param, "type") == "path"
        }),
        headers: entries(details.get("headers"), |_| true),
        body,
        auth: auth(details.get("auth")).unwrap_or_default(),
        vars_pre: entries(runtime.get("variables"), |_| true),
        vars_post: runtime
            .get("actions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|action| {
                str_at(action, "type") == "set-variable"
                    && matches!(str_at(action, "phase"), "" | "after-response")
            })
            .map(|action| {
                let variable = action.get("variable").unwrap_or(&Value::Null);
                let local = str_at(variable, "scope") == "request";
                let name = text(variable.get("name"));
                let selector = action.get("selector").unwrap_or(&Value::Null);
                KeyValue {
                    key: if local { format!("@{name}") } else { name },
                    value: text(selector.get("expression")),
                    enabled: !disabled(action),
                }
            })
            .collect(),
        asserts: runtime
            .get("assertions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|assertion| {
                let operator = match str_at(assertion, "operator") {
                    "" => "eq",
                    operator => operator,
                };
                let value = text(assertion.get("value"));
                KeyValue {
                    key: text(assertion.get("expression")),
                    value: if value.is_empty() {
                        operator.to_string()
                    } else {
                        format!("{operator} {value}")
                    },
                    enabled: !disabled(assertion),
                }
            })
            .collect(),
        pre_script: scripts.0,
        post_script: scripts.1,
        tests: scripts.2,
        docs: docs(value.get("docs")),
    }
}

/// Reads an environment file. `fallback` names it when it has no `name`.
pub(super) fn yaml_environment(value: &Value, fallback: &str) -> RawEnvironment {
    let list = value.get("variables").and_then(Value::as_array);
    let is_secret = |entry: &Value| entry.get("secret").and_then(Value::as_bool) == Some(true);
    RawEnvironment {
        name: value
            .get("name")
            .and_then(Value::as_str)
            .map_or_else(|| fallback.to_string(), str::to_string),
        vars: entries(value.get("variables"), |entry| !is_secret(entry)),
        secrets: list
            .into_iter()
            .flatten()
            .filter(|entry| is_secret(entry) && !disabled(entry))
            .map(|entry| text(entry.get("name")))
            .collect(),
        color: value
            .get("color")
            .and_then(Value::as_str)
            .map(str::to_string),
        extends: value
            .get("extends")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn a_request_as_bruno_writes_it() {
        let value = parse_yaml(
            r#"
info:
  name: Create User
  type: http
  seq: 2
http:
  method: post
  url: "{{host}}/users/:id?verbose=1"
  params:
    - name: verbose
      value: "1"
      type: query
    - name: id
      value: "7"
      type: path
  headers:
    - name: X-Trace
      value: abc
      disabled: true
  body:
    type: json
    data: |-
      {"name": "a"}
  auth:
    type: apikey
    key: X-Key
    value: "{{key}}"
    placement: query
runtime:
  variables:
    - name: page
      value: 2
  scripts:
    - type: before-request
      code: bru.setVar("a", 1);
    - type: tests
      code: test("ok", () => {});
  assertions:
    - expression: res.status
      operator: eq
      value: "201"
  actions:
    - type: set-variable
      phase: after-response
      selector:
        expression: res.body.id
        method: jsonq
      variable:
        name: userId
        scope: request
docs: Creates a user.
"#,
        )
        .expect("yaml");
        let request = yaml_request(&value, "file");
        assert_eq!(request.name, "Create User");
        assert_eq!(request.method, "POST");
        assert_eq!(request.query, vec![KeyValue::new("verbose", "1")]);
        assert_eq!(request.path, vec![KeyValue::new("id", "7")]);
        assert!(!request.headers[0].enabled);
        assert_eq!(request.body, RawBody::Json("{\"name\": \"a\"}".to_string()));
        assert_eq!(
            request.auth,
            RawAuth::ApiKey {
                key: "X-Key".to_string(),
                value: "{{key}}".to_string(),
                query: true
            }
        );
        assert_eq!(request.vars_pre, vec![KeyValue::new("page", "2")]);
        assert_eq!(request.pre_script, "bru.setVar(\"a\", 1);");
        assert_eq!(request.tests, "test(\"ok\", () => {});");
        assert_eq!(request.asserts, vec![KeyValue::new("res.status", "eq 201")]);
        assert_eq!(
            request.vars_post,
            vec![KeyValue::new("@userId", "res.body.id")]
        );
        assert_eq!(request.docs, "Creates a user.");
    }

    #[test]
    fn the_older_body_mode_and_an_environment() {
        let value = parse_yaml(
            "http:\n  method: POST\n  url: /u\n  body:\n    mode: json\n    json: |\n      {}\n",
        )
        .expect("yaml");
        let request = yaml_request(&value, "create-user");
        assert_eq!(request.name, "create-user");
        assert_eq!(request.body, RawBody::Json("{}\n".to_string()));

        let env = parse_yaml(
            "name: Development\ncolor: '#ff0000'\nvariables:\n  - name: baseUrl\n    value: https://a.test\n  - name: token\n    secret: true\n",
        )
        .expect("yaml");
        let env = yaml_environment(&env, "dev");
        assert_eq!(env.name, "Development");
        assert_eq!(env.vars, vec![KeyValue::new("baseUrl", "https://a.test")]);
        assert_eq!(env.secrets, vec!["token"]);
        assert_eq!(env.color.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn collection_defaults() {
        let value = parse_yaml(
            "opencollection: \"1.0.0\"\ninfo:\n  name: Shop\nrequest:\n  headers:\n    - name: X-C\n      value: c\n  auth: inherit\n  scripts:\n    - type: after-response\n      code: console.log(1)\n",
        )
        .expect("yaml");
        let defaults = yaml_defaults(&value);
        assert_eq!(defaults.headers, vec![KeyValue::new("X-C", "c")]);
        assert_eq!(defaults.auth, None);
        assert_eq!(defaults.scripts, vec!["post-response script"]);
        assert_eq!(yaml_info(&value).0.as_deref(), Some("Shop"));
    }
}
