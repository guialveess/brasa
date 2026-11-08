//! Integration tests for [`brasa_workspace::Workspace`]'s Postman import. `tests/fixtures/postman_collection.json` has a nested folder, a duplicate
//! request name (two "Login" requests in the same folder), a `formdata` body and a collection variable. `tests/fixtures/postman_environment.json` has one plain
//! value and one secret value.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;

use brasa_format::openapi::{ChangeKind, ImportOptions, ManifestSource, diff_plan};
use brasa_format::postman::{
    MANIFEST_KIND, PostmanOptions, build_postman_plan, parse_collection, parse_environment,
};
use brasa_workspace::{ImportOrigin, ImportReport, Workspace, WorkspaceError};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

const COLLECTION: &str = include_str!("fixtures/postman_collection.json");
const ENVIRONMENT: &str = include_str!("fixtures/postman_environment.json");

/// The origin of a test import.
fn origin(location: &str) -> ImportOrigin {
    ImportOrigin {
        source: ManifestSource {
            kind: MANIFEST_KIND.to_string(),
            location: location.to_string(),
            branch: String::new(),
        },
        date: "2026-11-07".to_string(),
        options: ImportOptions::default(),
    }
}

/// Imports a collection the way the wizard does.
fn import_collection(
    workspace: &mut Workspace,
    text: &str,
) -> Result<ImportReport, WorkspaceError> {
    let options = PostmanOptions::default();
    let collection = parse_collection(text, &options).expect("a collection");
    let name = collection.collection_name.clone();
    let built = build_postman_plan(&name, &[collection], &[], &[], &options);
    workspace.import_openapi(&built.plan, &|_| true, &origin("id"))
}

/// Imports an environment on its own the way the wizard does.
fn import_environment(
    workspace: &mut Workspace,
    text: &str,
) -> Result<ImportReport, WorkspaceError> {
    let options = PostmanOptions::default();
    let environment = parse_environment(text).expect("an environment");
    let name = environment.name.clone();
    let built = build_postman_plan(&name, &[], &[environment], &[], &options);
    workspace.import_openapi(&built.plan, &|_| true, &origin(""))
}

#[test]
fn import_collection_creates_a_folder_named_after_it_with_its_requests() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let report = import_collection(&mut workspace, COLLECTION).expect("import collection");

    assert!(temp.path().join("Sample API").is_dir());
    assert!(temp.path().join("Sample API/Auth/Login.brasa").is_file());
    assert!(
        temp.path()
            .join("Sample API/Auth/Login (2).brasa")
            .is_file(),
        "a second request named \"Login\" must get a \" (2)\" suffix"
    );
    assert!(temp.path().join("Sample API/Upload.brasa").is_file());
    assert_eq!(
        report.created_files,
        vec![
            "Sample API/Auth/Login.brasa".to_string(),
            "Sample API/Auth/Login (2).brasa".to_string(),
            "Sample API/Upload.brasa".to_string(),
            "environments/Sample API.env".to_string(),
        ]
    );
}

#[test]
fn import_collection_writes_form_data_as_multipart() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let report = import_collection(&mut workspace, COLLECTION).expect("import collection");
    let multipart = report.created_files.iter().any(|id| {
        std::fs::read_to_string(temp.path().join(id))
            .is_ok_and(|text| text.contains("::: body multipart"))
    });
    assert!(multipart, "{:?}", report.created_files);
}

#[test]
fn import_collection_writes_its_variables_as_an_environment() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    import_collection(&mut workspace, COLLECTION).expect("import collection");

    let content = fs::read_to_string(temp.path().join("environments/Sample API.env"))
        .expect("read environment file");
    assert_eq!(content, "token=abc123\n");
}

#[test]
fn importing_the_same_collection_twice_suffixes_the_second_folder_and_environment() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    import_collection(&mut workspace, COLLECTION).expect("first import");
    let second = import_collection(&mut workspace, COLLECTION).expect("second import");

    assert!(temp.path().join("Sample API (2)").is_dir());
    assert!(
        temp.path()
            .join("Sample API (2)/Auth/Login.brasa")
            .is_file()
    );
    assert!(
        second
            .created_files
            .contains(&"environments/Sample API (2).env".to_string())
    );
    assert!(temp.path().join("environments/Sample API.env").is_file());
    assert!(
        temp.path()
            .join("environments/Sample API (2).env")
            .is_file()
    );
}

#[test]
fn import_collection_never_overwrites_an_existing_file_or_folder() {
    let temp = TempDir::new().expect("temp dir");
    fs::write(temp.path().join("Sample API"), "not a folder, a stray file")
        .expect("write stray file");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    import_collection(&mut workspace, COLLECTION)
        .expect("import collection despite the name clash");

    assert!(temp.path().join("Sample API").is_file(), "left untouched");
    assert!(temp.path().join("Sample API (2)").is_dir());
}

#[test]
fn import_environment_writes_base_and_secret_files() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    let report = import_environment(&mut workspace, ENVIRONMENT).expect("import environment");

    let base = fs::read_to_string(temp.path().join("environments/Sample Env.env"))
        .expect("read base env file");
    assert_eq!(base, "host=example.com\n");
    let secret = fs::read_to_string(temp.path().join("environments/Sample Env.local.env"))
        .expect("read secret env file");
    assert_eq!(secret, "apiSecret=shh\n");
    assert_eq!(
        report.created_files,
        vec![
            "environments/Sample Env.env".to_string(),
            "environments/Sample Env.local.env".to_string(),
        ]
    );

    let environments = workspace.list_environments().expect("list environments");
    assert!(environments.contains(&"Sample Env".to_string()));
}

#[test]
fn import_environment_does_not_overwrite_an_existing_env_file() {
    let temp = TempDir::new().expect("temp dir");
    fs::create_dir_all(temp.path().join("environments")).expect("create environments dir");
    fs::write(
        temp.path().join("environments/Sample Env.env"),
        "PRE_EXISTING=1\n",
    )
    .expect("write pre-existing env file");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");

    import_environment(&mut workspace, ENVIRONMENT)
        .expect("import environment despite the name clash");

    let untouched = fs::read_to_string(temp.path().join("environments/Sample Env.env"))
        .expect("read pre-existing env file");
    assert_eq!(untouched, "PRE_EXISTING=1\n");
    let suffixed = fs::read_to_string(temp.path().join("environments/Sample Env (2).env"))
        .expect("read suffixed env file");
    assert_eq!(suffixed, "host=example.com\n");
}

#[test]
fn import_environment_without_secret_values_only_writes_the_base_file() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    import_environment(&mut workspace, r#"{"name": "Plain", "values": []}"#)
        .expect("import minimal environment");

    // No values at all: nothing should be written, not even the "environments" folder.
    assert!(!temp.path().join("environments").exists());
}

#[test]
fn every_file_written_by_a_collection_import_parses_back_without_errors() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let report = import_collection(&mut workspace, COLLECTION).expect("import collection");

    for id in &report.created_files {
        if let Some(name) = id.strip_prefix("environments/") {
            let path = temp.path().join("environments").join(name);
            let text = fs::read_to_string(&path).expect("read environment file");
            brasa_format::env::parse(&text)
                .unwrap_or_else(|error| panic!("{id} must parse as an env file: {error}"));
        } else {
            workspace
                .load_request(id)
                .unwrap_or_else(|error| panic!("{id} must parse back: {error}"));
        }
    }
}

#[test]
fn every_file_written_by_an_environment_import_parses_back_without_errors() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    let report = import_environment(&mut workspace, ENVIRONMENT).expect("import environment");

    for id in &report.created_files {
        let name = id
            .strip_prefix("environments/")
            .expect("environment import only writes into environments/");
        let path = temp.path().join("environments").join(name);
        let text = fs::read_to_string(&path).expect("read environment file");
        brasa_format::env::parse(&text)
            .unwrap_or_else(|error| panic!("{id} must parse as an env file: {error}"));
    }
}

/// Sanity check that the workspace rescans after a collection import: the new folder and
/// requests show up in [`Workspace::tree`] without a manual `rescan`.
#[test]
fn import_collection_is_visible_in_the_tree_afterwards() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    import_collection(&mut workspace, COLLECTION).expect("import collection");

    let names: Vec<&str> = workspace
        .tree()
        .iter()
        .map(brasa_workspace::Node::name)
        .collect();
    assert!(names.contains(&"Sample API"));
}

const V1: &str = r#"{
  "info": {"_postman_id": "4f1c", "name": "Acme",
    "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
  "item": [
    {"name": "List", "request": {"method": "GET", "url": "{{baseUrl}}/list"},
     "event": [{"listen": "test", "script": {"exec": ["pm.response.to.have.status(200);"]}}]},
    {"name": "Old", "request": {"method": "GET", "url": "{{baseUrl}}/old"}}
  ]
}"#;

const V2: &str = r#"{
  "info": {"_postman_id": "4f1c", "name": "Acme",
    "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"},
  "item": [
    {"name": "List", "request": {"method": "GET", "url": "{{baseUrl}}/v2/list"},
     "event": [{"listen": "test", "script": {"exec": ["pm.response.to.have.status(201);"]}}]},
    {"name": "New", "request": {"method": "POST", "url": "{{baseUrl}}/new"}}
  ]
}"#;

fn plan_of(text: &str) -> brasa_format::openapi::OpenApiPlan {
    let options = PostmanOptions::default();
    let collection = parse_collection(text, &options).expect("a collection");
    let name = collection.collection_name.clone();
    build_postman_plan(&name, &[collection], &[], &[], &options).plan
}

#[test]
fn a_new_export_of_a_collection_updates_it_and_keeps_edited_scripts() {
    let temp = TempDir::new().expect("temp dir");
    let mut workspace = Workspace::open(temp.path()).expect("open workspace");
    workspace
        .import_openapi(&plan_of(V1), &|_| true, &origin("4f1c"))
        .expect("first import");

    // The collection is found again by its id, wherever the file is.
    let collections = workspace.openapi_collections();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].manifest.source.kind, MANIFEST_KIND);
    assert_eq!(collections[0].manifest.source.location, "4f1c");

    // The user's own script stays.
    let list = temp.path().join("Acme/List.brasa");
    let edited = fs::read_to_string(&list)
        .expect("read")
        .replace("toBe(200)", "toBe(299)");
    fs::write(&list, edited).expect("write");

    let plan = plan_of(V2);
    let changes = diff_plan(&collections[0].manifest, &plan);
    let kinds: Vec<(&str, ChangeKind)> = changes
        .iter()
        .map(|change| (change.key.as_str(), change.kind))
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("GET List", ChangeKind::Changed),
            ("POST New", ChangeKind::Added),
            ("GET Old", ChangeKind::Removed),
        ]
    );

    let report = workspace
        .update_openapi("Acme", &plan, &|_| true, &origin("4f1c"))
        .expect("update");
    assert_eq!((report.added, report.changed, report.removed), (1, 1, 1));
    let list = fs::read_to_string(&list).expect("read");
    assert!(list.contains("{{baseUrl}}/v2/list"), "{list}");
    assert!(list.contains("toBe(299)"), "{list}");
    assert!(temp.path().join("Acme/New.brasa").is_file());
    assert!(!temp.path().join("Acme/Old.brasa").exists());
}
