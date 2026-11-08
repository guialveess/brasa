//! `Workspace::import_openapi` and `Workspace::update_openapi`: the selected requests, the
//! environments and the manifest end up on disk, and an update applies only the chosen changes
//! while keeping the user's edits.

#![allow(clippy::expect_used)]

use brasa_format::openapi::{
    ImportOptions, MANIFEST_FILE, Manifest, ManifestSource, NoExternalRefs, OpenApiPlan,
    SpecVersion, build_plan, parse_spec,
};
use brasa_workspace::{ImportOrigin, Workspace};
use pretty_assertions::assert_eq;

const SPEC: &str = r##"
openapi: 3.1.0
info: {title: Acme Payments API, version: 2.0.0}
servers: [{url: https://api.acme.dev/v2}]
security: [{bearerAuth: []}]
components:
  securitySchemes:
    bearerAuth: {type: http, scheme: bearer}
paths:
  /customers:
    get:
      summary: List customers
      parameters: [{name: limit, in: query}]
      responses: {'200': {description: ok}}
    post:
      summary: Create customer
      requestBody: {content: {application/json: {example: {name: Ada}}}}
      responses: {'201': {description: created}}
  /customers/{id}:
    get: {summary: Get customer, responses: {'200': {description: ok}}}
    delete: {summary: Delete customer, responses: {'204': {description: ok}}}
"##;

/// The spec a while later: a query parameter more, one operation gone, one new.
const SPEC_V2: &str = r##"
openapi: 3.1.0
info: {title: Acme Payments API, version: 2.1.0}
servers: [{url: https://api.acme.dev/v2}]
security: [{bearerAuth: []}]
components:
  securitySchemes:
    bearerAuth: {type: http, scheme: bearer}
paths:
  /customers:
    get:
      summary: List customers
      parameters: [{name: limit, in: query}, {name: created_after, in: query}]
      responses: {'200': {description: ok}}
    post:
      summary: Create customer
      requestBody: {content: {application/json: {example: {name: Ada}}}}
      responses: {'201': {description: created}}
  /customers/{id}:
    get: {summary: Get customer, responses: {'200': {description: ok}}}
  /payouts:
    get: {summary: List payouts, responses: {'200': {description: ok}}}
"##;

fn plan(text: &str) -> OpenApiPlan {
    let spec = parse_spec(text, "", &NoExternalRefs).expect("valid");
    build_plan(
        &spec.title,
        &[SpecVersion {
            label: "",
            spec: &spec,
        }],
        &ImportOptions::default(),
    )
}

fn origin(date: &str) -> ImportOrigin {
    ImportOrigin {
        source: ManifestSource {
            kind: "url".into(),
            location: "https://api.acme.dev/openapi.yaml".into(),
            branch: String::new(),
        },
        date: date.into(),
        options: ImportOptions::default(),
    }
}

#[test]
fn writes_the_selected_requests_the_environments_and_the_manifest() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("Acme Payments API")).expect("a clashing folder");
    let mut workspace = Workspace::open(dir.path()).expect("an empty folder opens");

    let report = workspace
        .import_openapi(
            &plan(SPEC),
            &|key| key != "GET /customers/{id}",
            &origin("2026-10-28"),
        )
        .expect("the import succeeds");

    assert_eq!(
        report.created_files,
        vec![
            "Acme Payments API (2)/customers/{id}/Delete customer.brasa",
            "Acme Payments API (2)/customers/List customers.brasa",
            "Acme Payments API (2)/customers/Create customer.brasa",
            "environments/Acme Payments API.env",
            "environments/Acme Payments API.local.env",
        ]
    );
    for id in report
        .created_files
        .iter()
        .filter(|id| id.ends_with(".brasa"))
    {
        workspace
            .load_request(id)
            .unwrap_or_else(|error| panic!("{id} failed to parse back: {error}"));
    }
    let env = std::fs::read_to_string(dir.path().join("environments/Acme Payments API.env"))
        .expect("written");
    assert_eq!(env, "baseUrl=https://api.acme.dev\n");

    // The manifest lists the written requests only, and the folder is found as an import.
    let collections = workspace.openapi_collections();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0].folder, "Acme Payments API (2)");
    let manifest = &collections[0].manifest;
    assert_eq!(manifest.imported, "2026-10-28");
    assert_eq!(
        manifest.requests.keys().cloned().collect::<Vec<_>>(),
        vec![
            "DELETE /customers/{id}",
            "GET /customers",
            "POST /customers"
        ]
    );
    assert_eq!(
        manifest.requests["GET /customers"].file,
        "customers/List customers.brasa"
    );
    // Hidden from the sidebar.
    assert!(!format!("{:?}", workspace.tree()).contains(MANIFEST_FILE));
}

#[test]
fn updates_apply_the_chosen_changes_and_keep_user_edits() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut workspace = Workspace::open(dir.path()).expect("opens");
    workspace
        .import_openapi(&plan(SPEC), &|_| true, &origin("2026-10-28"))
        .expect("imports");
    let collection = dir.path().join("Acme Payments API");

    // The user sets a value and writes a test in the listed request.
    let list = collection.join("customers/List customers.brasa");
    let mut request =
        brasa_format::parse(&std::fs::read_to_string(&list).expect("read")).expect("parses");
    request.query[0].value = "50".into();
    request.post_script = "test('mine', () => {});".into();
    std::fs::write(&list, brasa_format::serialize(&request)).expect("write");

    // Apply everything but the new operation.
    let report = workspace
        .update_openapi(
            "Acme Payments API",
            &plan(SPEC_V2),
            &|key| key != "GET /payouts",
            &origin("2026-11-04"),
        )
        .expect("updates");
    assert_eq!((report.added, report.changed, report.removed), (0, 1, 1));

    let updated =
        brasa_format::parse(&std::fs::read_to_string(&list).expect("read")).expect("parses");
    let query: Vec<(&str, &str)> = updated
        .query
        .iter()
        .map(|entry| (entry.key.as_str(), entry.value.as_str()))
        .collect();
    assert_eq!(query, vec![("limit", "50"), ("created_after", "")]);
    assert_eq!(updated.post_script, "test('mine', () => {});");
    assert!(
        !collection
            .join("customers/{id}/Delete customer.brasa")
            .exists()
    );
    assert!(!collection.join("payouts").exists());

    // The skipped addition is offered again; now take it.
    let manifest = Manifest::from_toml(
        &std::fs::read_to_string(collection.join(MANIFEST_FILE)).expect("read"),
    )
    .expect("parses");
    assert_eq!(manifest.imported, "2026-11-04");
    assert!(!manifest.requests.contains_key("GET /payouts"));
    let report = workspace
        .update_openapi(
            "Acme Payments API",
            &plan(SPEC_V2),
            &|_| true,
            &origin("2026-11-05"),
        )
        .expect("updates");
    assert_eq!((report.added, report.changed, report.removed), (1, 0, 0));
    assert!(collection.join("payouts/List payouts.brasa").exists());
}

#[test]
fn a_kept_removed_request_is_no_longer_tracked() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut workspace = Workspace::open(dir.path()).expect("opens");
    workspace
        .import_openapi(&plan(SPEC), &|_| true, &origin("2026-10-28"))
        .expect("imports");
    workspace
        .update_openapi(
            "Acme Payments API",
            &plan(SPEC_V2),
            &|key| key != "DELETE /customers/{id}",
            &origin("2026-11-04"),
        )
        .expect("updates");
    let collection = dir.path().join("Acme Payments API");
    assert!(
        collection
            .join("customers/{id}/Delete customer.brasa")
            .exists()
    );
    let manifest = Manifest::from_toml(
        &std::fs::read_to_string(collection.join(MANIFEST_FILE)).expect("read"),
    )
    .expect("parses");
    assert!(!manifest.requests.contains_key("DELETE /customers/{id}"));
}

#[test]
fn a_folder_left_empty_by_a_removal_goes_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut workspace = Workspace::open(dir.path()).expect("opens");
    workspace
        .import_openapi(&plan(SPEC_V2), &|_| true, &origin("2026-10-28"))
        .expect("imports");
    let collection = dir.path().join("Acme Payments API");
    assert!(collection.join("payouts/List payouts.brasa").exists());
    let report = workspace
        .update_openapi(
            "Acme Payments API",
            &plan(SPEC),
            &|_| true,
            &origin("2026-11-04"),
        )
        .expect("updates");
    assert_eq!(report.removed, 1);
    assert!(!collection.join("payouts").exists());
    assert!(collection.exists());
}

#[test]
fn a_failed_import_leaves_nothing_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A file where the environments folder goes, so writing the environments fails last.
    std::fs::write(dir.path().join("environments"), "in the way").expect("write");
    let mut workspace = Workspace::open(dir.path()).expect("open");

    let result = workspace.import_openapi(&plan(SPEC), &|_| true, &origin("2026-11-06"));

    assert!(result.is_err());
    assert!(!dir.path().join("Acme Payments API").exists());
}
