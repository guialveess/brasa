//! From what Postman exports to the plan the wizard shows and `brasa-workspace` writes: the
//! same [`OpenApiPlan`] an OpenAPI import uses, so the preview, the selection and the update of a
//! collection are one code path.

use std::collections::{HashMap, HashSet};

use brasa_core::KeyValue;
use serde_json::Value;

use super::collection::{ImportFolder, ImportPlan, PostmanOptions, RequestIssue};
use super::environment::ImportedEnvironment;
use crate::openapi::{OpenApiPlan, PlanEnvironment, PlanFolder, PlanRequest};

/// The `kind` of a manifest source written by a Postman import. Its `location` is the collection's
/// `info._postman_id`, which identifies it across exports wherever the file is.
pub const MANIFEST_KIND: &str = "postman";

/// What a Postman export is, from its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostmanFormat {
    /// A collection (`info.schema` names Postman's collection schema).
    Collection,
    /// An environment (`_postman_variable_scope` is `environment`).
    Environment,
    /// The globals (`_postman_variable_scope` is `globals`).
    Globals,
}

/// Tells what Postman export `text` is. `None` when it is not one.
pub fn detect_format(text: &str) -> Option<PostmanFormat> {
    let value: Value = serde_json::from_str(text).ok()?;
    let is_collection = value
        .pointer("/info/schema")
        .and_then(Value::as_str)
        .is_some_and(|schema| {
            schema.contains("getpostman.com/json/collection")
                || schema.contains("postman.com/json/collection")
        });
    if is_collection {
        return Some(PostmanFormat::Collection);
    }
    match value.get("_postman_variable_scope").and_then(Value::as_str) {
        Some("environment") => Some(PostmanFormat::Environment),
        Some("globals") => Some(PostmanFormat::Globals),
        _ => None,
    }
}

/// The plan of a Postman import, and what to tell about its requests.
#[derive(Debug, Clone, PartialEq)]
pub struct PostmanPlan {
    /// The folders, requests and environments to write.
    pub plan: OpenApiPlan,
    /// What to tell about each request, by request key. Requests with nothing to tell are absent.
    pub issues: HashMap<String, Vec<RequestIssue>>,
}

/// Builds the plan of `collections` under a collection folder named `name`.
///
/// With one collection its folders go straight in; with several each gets a folder of its own.
/// Brasa activates one environment at a time and has no inheritance between them, so collection
/// variables and `globals` are copied into every environment, a value of the environment winning.
/// Without environments they make one named after the collection.
pub fn build_postman_plan(
    name: &str,
    collections: &[ImportPlan],
    environments: &[ImportedEnvironment],
    globals: &[ImportedEnvironment],
    options: &PostmanOptions,
) -> PostmanPlan {
    let mut root = PlanFolder {
        name: name.to_string(),
        ..PlanFolder::default()
    };
    let mut issues = HashMap::new();
    let mut warnings = Vec::new();
    for collection in collections {
        warnings.extend(collection.warnings.iter().cloned());
        if let [_] = collections {
            add_folder(&mut root, &collection.root, "", options, &mut issues);
        } else {
            let mut folder = PlanFolder {
                name: collection.collection_name.clone(),
                ..PlanFolder::default()
            };
            let prefix = format!("{}/", collection.collection_name);
            add_folder(&mut folder, &collection.root, &prefix, options, &mut issues);
            root.folders.push(folder);
        }
    }
    for environment in environments.iter().chain(globals) {
        warnings.extend(environment.warnings.iter().cloned());
    }

    let shared_base = shared_variables(
        collections,
        globals,
        options,
        |collection| &collection.variables,
        |global| &global.base,
    );
    let shared_secret = shared_variables(
        collections,
        globals,
        options,
        |collection| &collection.secret_variables,
        |global| &global.secret,
    );
    // OAuth 2.0 tokens are in `secret_variables` whatever the variables option says: the requests
    // use them.
    let tokens: Vec<KeyValue> = collections
        .iter()
        .flat_map(|collection| &collection.secret_variables)
        .filter(|entry| entry.key.starts_with("oauth2"))
        .cloned()
        .collect();
    let secret_of = |own: &[KeyValue]| {
        if !options.secrets {
            return Vec::new();
        }
        let mut secret = merged(&shared_secret, own);
        secret = merged(&tokens, &secret);
        secret
    };

    let mut plan_environments: Vec<PlanEnvironment> = environments
        .iter()
        .map(|environment| PlanEnvironment {
            name: environment.name.clone(),
            base: merged(&shared_base, &environment.base),
            secret: secret_of(&environment.secret),
        })
        .collect();
    if plan_environments.is_empty() {
        let secret = secret_of(&[]);
        if !shared_base.is_empty() || !secret.is_empty() {
            plan_environments.push(PlanEnvironment {
                name: name.to_string(),
                base: shared_base.clone(),
                secret,
            });
        }
    }

    PostmanPlan {
        plan: OpenApiPlan {
            root,
            environments: plan_environments,
            warnings,
        },
        issues,
    }
}

/// The variables every environment gets, later sources winning: globals, then collections.
fn shared_variables<'a>(
    collections: &'a [ImportPlan],
    globals: &'a [ImportedEnvironment],
    options: &PostmanOptions,
    of_collection: impl Fn(&'a ImportPlan) -> &'a Vec<KeyValue>,
    of_globals: impl Fn(&'a ImportedEnvironment) -> &'a Vec<KeyValue>,
) -> Vec<KeyValue> {
    if !options.variables {
        return Vec::new();
    }
    let mut shared = Vec::new();
    for global in globals {
        shared = merged(&shared, of_globals(global));
    }
    for collection in collections {
        shared = merged(&shared, of_collection(collection));
    }
    shared
}

/// `base` with `over` on top: a key in both takes the value of `over`, in the position of `base`.
fn merged(base: &[KeyValue], over: &[KeyValue]) -> Vec<KeyValue> {
    let mut out: Vec<KeyValue> = base.to_vec();
    for entry in over {
        match out.iter_mut().find(|known| known.key == entry.key) {
            Some(known) => known.value.clone_from(&entry.value),
            None => out.push(entry.clone()),
        }
    }
    out
}

/// Copies `source` into `target`, flattening its folders when the option is off.
fn add_folder(
    target: &mut PlanFolder,
    source: &ImportFolder,
    path: &str,
    options: &PostmanOptions,
    issues: &mut HashMap<String, Vec<RequestIssue>>,
) {
    let mut keys: HashSet<String> = target
        .requests
        .iter()
        .map(|request| request.key.clone())
        .collect();
    for (index, (name, request)) in source.requests.iter().enumerate() {
        let base = format!("{path}{} {name}", request.method);
        let key = unique_key(&base, &mut keys);
        if let Some(found) = source.issues.get(index).filter(|found| !found.is_empty()) {
            issues.insert(key.clone(), found.clone());
        }
        target.requests.push(PlanRequest {
            key,
            name: name.clone(),
            path: display_path(&request.url),
            request: request.clone(),
        });
    }
    for child in &source.folders {
        let child_path = format!("{path}{}/", child.name);
        if options.folders {
            let mut folder = PlanFolder {
                name: child.name.clone(),
                ..PlanFolder::default()
            };
            add_folder(&mut folder, child, &child_path, options, issues);
            target.folders.push(folder);
        } else {
            add_folder(target, child, &child_path, options, issues);
        }
    }
}

/// `base`, or `base #2`, `base #3`, ... when two requests of a folder share method and name.
fn unique_key(base: &str, keys: &mut HashSet<String>) -> String {
    let mut key = base.to_string();
    let mut counter = 2;
    while !keys.insert(key.clone()) {
        key = format!("{base} #{counter}");
        counter += 1;
    }
    key
}

/// What the preview shows for a URL: its path, without the scheme, host and `{{baseUrl}}`.
fn display_path(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    match rest
        .strip_prefix("{{")
        .and_then(|rest| rest.split_once("}}"))
    {
        Some((_, path)) => path.to_string(),
        None => rest
            .find('/')
            .map_or_else(|| url.to_string(), |index| rest[index..].to_string()),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::super::collection::parse_collection;
    use super::super::environment::parse_environment;
    use super::*;
    use pretty_assertions::assert_eq;

    const COLLECTION: &str = r#"{
      "info": {
        "_postman_id": "4f1c",
        "name": "Acme",
        "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
      },
      "variable": [
        {"key": "baseUrl", "value": "https://acme.dev"},
        {"key": "region", "value": "eu"},
        {"key": "apiSecret", "value": "s3", "type": "secret"}
      ],
      "item": [
        {"name": "Ping", "request": {"method": "GET", "url": "{{baseUrl}}/ping"}},
        {"name": "Ping", "request": {"method": "GET", "url": "{{baseUrl}}/ping2"}},
        {"name": "Auth", "item": [
          {"name": "Login", "request": {"method": "POST", "url": "https://acme.dev/login"},
           "event": [{"listen": "test", "script": {"exec": ["pm.response.to.have.status(200);"]}}]}
        ]}
      ]
    }"#;

    const PRODUCTION: &str = r#"{
      "name": "Production",
      "_postman_variable_scope": "environment",
      "values": [
        {"key": "baseUrl", "value": "https://prod.acme.dev", "enabled": true},
        {"key": "token", "value": "abc", "type": "secret", "enabled": true}
      ]
    }"#;

    const GLOBALS: &str = r#"{
      "name": "My Workspace Globals",
      "_postman_variable_scope": "globals",
      "values": [
        {"key": "region", "value": "us", "enabled": true},
        {"key": "tenant", "value": "t1", "enabled": true}
      ]
    }"#;

    fn collection(options: &PostmanOptions) -> ImportPlan {
        parse_collection(COLLECTION, options).expect("a collection")
    }

    fn environment(text: &str) -> ImportedEnvironment {
        parse_environment(text).expect("an environment")
    }

    #[test]
    fn formats_are_detected_from_the_content() {
        assert_eq!(detect_format(COLLECTION), Some(PostmanFormat::Collection));
        assert_eq!(detect_format(PRODUCTION), Some(PostmanFormat::Environment));
        assert_eq!(detect_format(GLOBALS), Some(PostmanFormat::Globals));
        assert_eq!(detect_format("{\"openapi\": \"3.1.0\"}"), None);
        assert_eq!(detect_format("openapi: 3.1.0"), None);
    }

    #[test]
    fn the_id_and_version_come_from_info() {
        let plan = collection(&PostmanOptions::default());
        assert_eq!(plan.postman_id, "4f1c");
        assert_eq!(plan.schema_version, "v2.1.0");
    }

    #[test]
    fn requests_get_stable_unique_keys_and_their_issues() {
        let options = PostmanOptions::default();
        let plan = build_postman_plan("Acme", &[collection(&options)], &[], &[], &options);
        assert_eq!(
            plan.plan.root.keys(),
            vec!["GET Ping", "GET Ping #2", "Auth/POST Login"]
        );
        assert_eq!(
            plan.issues.get("Auth/POST Login"),
            Some(&vec![RequestIssue::ScriptConverted])
        );
        assert_eq!(plan.plan.root.requests[0].path, "/ping");
        assert_eq!(plan.plan.root.folders[0].requests[0].path, "/login");
    }

    #[test]
    fn collection_variables_and_globals_are_copied_into_every_environment() {
        let options = PostmanOptions::default();
        let plan = build_postman_plan(
            "Acme",
            &[collection(&options)],
            &[environment(PRODUCTION)],
            &[environment(GLOBALS)],
            &options,
        );
        let [production] = plan.plan.environments.as_slice() else {
            panic!("one environment");
        };
        assert_eq!(production.name, "Production");
        assert_eq!(
            production.base,
            vec![
                KeyValue::new("region", "eu"),
                KeyValue::new("tenant", "t1"),
                KeyValue::new("baseUrl", "https://prod.acme.dev"),
            ]
        );
        assert_eq!(
            production.secret,
            vec![
                KeyValue::new("apiSecret", "s3"),
                KeyValue::new("token", "abc")
            ]
        );
    }

    #[test]
    fn without_environments_the_variables_make_one_named_after_the_collection() {
        let options = PostmanOptions::default();
        let plan = build_postman_plan("Acme", &[collection(&options)], &[], &[], &options);
        let [only] = plan.plan.environments.as_slice() else {
            panic!("one environment");
        };
        assert_eq!(only.name, "Acme");
        assert_eq!(only.secret, vec![KeyValue::new("apiSecret", "s3")]);
    }

    #[test]
    fn options_turn_parts_off() {
        let options = PostmanOptions {
            folders: false,
            variables: false,
            secrets: false,
            ..PostmanOptions::default()
        };
        let plan = build_postman_plan(
            "Acme",
            &[collection(&options)],
            &[environment(PRODUCTION)],
            &[],
            &options,
        );
        assert!(plan.plan.root.folders.is_empty());
        assert_eq!(plan.plan.root.requests.len(), 3);
        assert_eq!(
            plan.plan.environments[0].base,
            vec![KeyValue::new("baseUrl", "https://prod.acme.dev")]
        );
        assert!(plan.plan.environments[0].secret.is_empty());
    }

    #[test]
    fn several_collections_get_a_folder_each() {
        let options = PostmanOptions::default();
        let plan = build_postman_plan(
            "Export",
            &[collection(&options), collection(&options)],
            &[],
            &[],
            &options,
        );
        assert_eq!(plan.plan.root.folders.len(), 2);
        assert_eq!(plan.plan.root.folders[0].name, "Acme");
    }

    #[test]
    fn only_an_environment_writes_no_requests() {
        let options = PostmanOptions::default();
        let plan = build_postman_plan("Production", &[], &[environment(PRODUCTION)], &[], &options);
        assert!(plan.plan.root.keys().is_empty());
        assert_eq!(plan.plan.environments.len(), 1);
    }
}
