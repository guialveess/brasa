//! The manifest an OpenAPI import leaves in its collection folder (`.openapi.toml`, GitHub #203),
//! and what it is for: telling what changed when the spec is imported again, and updating the
//! requests without losing the user's own changes.
//!
//! For each request the manifest keeps its file and a fingerprint of what the spec generated:
//! method, URL, query and header names, a hash of the body and the top level keys of a JSON or
//! form body. Comparing it with a new plan gives the added, changed and removed operations, and
//! [`update_request`] applies a change to the file on disk.

use std::collections::BTreeMap;

use brasa_core::{Body, KeyValue, Request};
use serde::{Deserialize, Serialize};

use super::generate::{Grouping, ImportOptions, OpenApiPlan, PlanFolder};

/// The file name of the manifest, inside the collection folder. Hidden, so the sidebar does not
/// list it, and versioned with the requests.
pub const MANIFEST_FILE: &str = ".openapi.toml";

/// The manifest format version.
const MANIFEST_VERSION: u32 = 1;

/// The comment on top of a manifest file.
const MANIFEST_HEADER: &str = "# Written by Brasa's import, to update this collection when its source changes.\n# Keep it with the requests. Editing it by hand is not needed.\n\n";

/// What an OpenAPI import remembers about a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// The format version.
    pub version: u32,
    /// The day of the import or the last update, `YYYY-MM-DD`.
    pub imported: String,
    /// Where the spec came from.
    pub source: ManifestSource,
    /// The options of the import.
    pub options: ManifestOptions,
    /// One entry per request, by request key (`[<version> ]<METHOD> <path>`).
    #[serde(default)]
    pub requests: BTreeMap<String, ManifestRequest>,
}

/// Where the spec of an import came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManifestSource {
    /// `file`, `url`, `folder`, `git` or `paste`.
    pub kind: String,
    /// The file or folder path, the URL or the repository; empty for pasted text.
    #[serde(default)]
    pub location: String,
    /// The branch of a Git repository, empty for its default branch or another source.
    #[serde(default)]
    pub branch: String,
}

/// The import options, as written in a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestOptions {
    /// `path` or `tags`.
    pub grouping: ManifestGrouping,
    /// Environments from the servers.
    pub environments: bool,
    /// Auth from the security schemes.
    pub auth: bool,
    /// Example bodies.
    pub bodies: bool,
    /// Status code tests.
    pub tests: bool,
    /// Request docs.
    pub docs: bool,
}

/// [`Grouping`] in a manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ManifestGrouping {
    /// [`Grouping::PathSegments`].
    Path,
    /// [`Grouping::Tags`].
    Tags,
}

impl From<&ImportOptions> for ManifestOptions {
    fn from(options: &ImportOptions) -> Self {
        ManifestOptions {
            grouping: match options.grouping {
                Grouping::PathSegments => ManifestGrouping::Path,
                Grouping::Tags => ManifestGrouping::Tags,
            },
            environments: options.environments,
            auth: options.auth,
            bodies: options.bodies,
            tests: options.tests,
            docs: options.docs,
        }
    }
}

impl From<ManifestOptions> for ImportOptions {
    fn from(options: ManifestOptions) -> Self {
        ImportOptions {
            environments: options.environments,
            auth: options.auth,
            bodies: options.bodies,
            tests: options.tests,
            docs: options.docs,
            grouping: match options.grouping {
                ManifestGrouping::Path => Grouping::PathSegments,
                ManifestGrouping::Tags => Grouping::Tags,
            },
        }
    }
}

/// What the spec generated for one request, and where it was written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ManifestRequest {
    /// The request file, relative to the collection folder, with `/` separators.
    pub file: String,
    /// The HTTP method.
    pub method: String,
    /// The spec path (`/customers/{id}`).
    pub path: String,
    /// The generated URL.
    pub url: String,
    /// The generated query parameter names, in order.
    #[serde(default)]
    pub query: Vec<String>,
    /// The generated header names, in order.
    #[serde(default)]
    pub headers: Vec<String>,
    /// A hash of the generated body, empty for no body.
    #[serde(default)]
    pub body: String,
    /// The top level keys of a generated JSON object or form body.
    #[serde(default)]
    pub body_keys: Vec<String>,
}

impl Manifest {
    /// A manifest for an import of `plan`, whose requests were written to `files` (request key
    /// to file, relative to the collection folder).
    pub fn new(
        imported: &str,
        source: ManifestSource,
        options: &ImportOptions,
        plan: &OpenApiPlan,
        files: &BTreeMap<String, String>,
    ) -> Self {
        let mut manifest = Manifest {
            version: MANIFEST_VERSION,
            imported: imported.to_string(),
            source,
            options: options.into(),
            requests: BTreeMap::new(),
        };
        manifest.record(plan, files);
        manifest
    }

    /// Records the fingerprint of every request of `plan` that `files` has a file for.
    pub fn record(&mut self, plan: &OpenApiPlan, files: &BTreeMap<String, String>) {
        visit(&plan.root, &mut |request| {
            if let Some(file) = files.get(&request.key) {
                self.requests.insert(
                    request.key.clone(),
                    fingerprint(file, &request.path, &request.request),
                );
            }
        });
    }

    /// The manifest as TOML, with its header comment.
    pub fn to_toml(&self) -> String {
        let body = toml::to_string(self).unwrap_or_default();
        format!("{MANIFEST_HEADER}{body}")
    }

    /// Reads a manifest written by [`Self::to_toml`].
    pub fn from_toml(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|error| error.to_string())
    }
}

fn visit(folder: &PlanFolder, f: &mut dyn FnMut(&super::generate::PlanRequest)) {
    folder.requests.iter().for_each(&mut *f);
    folder.folders.iter().for_each(|child| visit(child, f));
}

/// The fingerprint of a generated request.
pub fn fingerprint(file: &str, path: &str, request: &Request) -> ManifestRequest {
    ManifestRequest {
        file: file.to_string(),
        method: request.method.to_string(),
        path: path.to_string(),
        url: request.url.clone(),
        query: request
            .query
            .iter()
            .map(|entry| entry.key.clone())
            .collect(),
        headers: request
            .headers
            .iter()
            .map(|entry| entry.key.clone())
            .collect(),
        body: body_hash(&request.body),
        body_keys: body_keys(&request.body),
    }
}

/// A stable hash of a body, with its type: FNV-1a 64 over a canonical text, `""` for no body.
fn body_hash(body: &Body) -> String {
    let text = match body {
        Body::None => return String::new(),
        Body::Json(text) => format!("json\n{text}"),
        Body::Text(text) => format!("text\n{text}"),
        Body::Xml(text) => format!("xml\n{text}"),
        Body::Form(fields) | Body::Multipart(fields) => {
            let mut text = "form\n".to_string();
            for field in fields {
                text.push_str(&format!("{}={}\n", field.key, field.value));
            }
            text
        }
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn body_keys(body: &Body) -> Vec<String> {
    match body {
        Body::Json(text) => match serde_json::from_str::<serde_json::Value>(text) {
            Ok(serde_json::Value::Object(map)) => map.keys().cloned().collect(),
            _ => Vec::new(),
        },
        Body::Form(fields) | Body::Multipart(fields) => {
            fields.iter().map(|field| field.key.clone()).collect()
        }
        _ => Vec::new(),
    }
}

/// How an operation changed between the manifest and a new plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// New in the spec.
    Added,
    /// In both, with a different URL, method, parameters, headers or body.
    Changed,
    /// Gone from the spec.
    Removed,
    /// In both, the same.
    Unchanged,
}

/// One operation of an update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanChange {
    /// The request key.
    pub key: String,
    /// What happened to it.
    pub kind: ChangeKind,
    /// The HTTP method, for display.
    pub method: String,
    /// The spec path, for display.
    pub path: String,
    /// What changed, short (`query: +created_after`), for a changed operation.
    pub details: Vec<String>,
}

/// Compares the requests of the manifest with those of a new plan, in the plan's order, the
/// removed ones last.
pub fn diff_plan(manifest: &Manifest, plan: &OpenApiPlan) -> Vec<PlanChange> {
    let mut changes = Vec::new();
    visit(&plan.root, &mut |request| {
        let new = fingerprint("", &request.path, &request.request);
        let change = match manifest.requests.get(&request.key) {
            None => PlanChange {
                key: request.key.clone(),
                kind: ChangeKind::Added,
                method: new.method.clone(),
                path: new.path.clone(),
                details: Vec::new(),
            },
            Some(old) => {
                let details = describe(old, &new);
                PlanChange {
                    key: request.key.clone(),
                    kind: if details.is_empty() {
                        ChangeKind::Unchanged
                    } else {
                        ChangeKind::Changed
                    },
                    method: new.method.clone(),
                    path: new.path.clone(),
                    details,
                }
            }
        };
        changes.push(change);
    });
    for (key, old) in &manifest.requests {
        if !changes.iter().any(|change| &change.key == key) {
            changes.push(PlanChange {
                key: key.clone(),
                kind: ChangeKind::Removed,
                method: old.method.clone(),
                path: old.path.clone(),
                details: Vec::new(),
            });
        }
    }
    changes
}

/// What differs between two fingerprints, one short text per part.
fn describe(old: &ManifestRequest, new: &ManifestRequest) -> Vec<String> {
    let mut details = Vec::new();
    if old.method != new.method || old.url != new.url {
        details.push("URL".to_string());
    }
    if let Some(change) = list_change("query", &old.query, &new.query) {
        details.push(change);
    }
    if let Some(change) = list_change("headers", &old.headers, &new.headers) {
        details.push(change);
    }
    if old.body != new.body {
        details.push(
            list_change("body", &old.body_keys, &new.body_keys)
                .unwrap_or_else(|| "body".to_string()),
        );
    }
    details
}

/// `name: +added -removed` for two lists of names, `None` when they hold the same names.
fn list_change(name: &str, old: &[String], new: &[String]) -> Option<String> {
    let added: Vec<String> = new
        .iter()
        .filter(|item| !old.contains(item))
        .map(|item| format!("+{item}"))
        .collect();
    let removed: Vec<String> = old
        .iter()
        .filter(|item| !new.contains(item))
        .map(|item| format!("-{item}"))
        .collect();
    if added.is_empty() && removed.is_empty() {
        return None;
    }
    Some(format!(
        "{name}: {}",
        added
            .into_iter()
            .chain(removed)
            .collect::<Vec<_>>()
            .join(" ")
    ))
}

/// Applies a change of the spec to the request on disk: the method and URL become the new ones,
/// query parameters and headers the spec added are appended and the ones it dropped are
/// removed, while the values the user set on the others, and the entries the user added, stay.
/// The body becomes the new example only when the user did not edit it. Auth, scripts, tests and
/// docs are kept.
pub fn update_request(current: &Request, old: &ManifestRequest, new: &Request) -> Request {
    let mut updated = current.clone();
    updated.method = new.method.clone();
    updated.url = new.url.clone();
    updated.query = merge_entries(&current.query, &old.query, &new.query, false);
    updated.headers = merge_entries(&current.headers, &old.headers, &new.headers, true);
    if body_hash(&current.body) == old.body {
        updated.body = new.body.clone();
    }
    updated
}

fn merge_entries(
    current: &[KeyValue],
    old: &[String],
    new: &[KeyValue],
    fold_case: bool,
) -> Vec<KeyValue> {
    let same = |a: &str, b: &str| {
        if fold_case {
            a.eq_ignore_ascii_case(b)
        } else {
            a == b
        }
    };
    let mut merged: Vec<KeyValue> = current
        .iter()
        .filter(|entry| {
            let in_new = new.iter().any(|candidate| same(&candidate.key, &entry.key));
            let was_generated = old.iter().any(|key| same(key, &entry.key));
            in_new || !was_generated
        })
        .cloned()
        .collect();
    for entry in new {
        if !merged
            .iter()
            .any(|existing| same(&existing.key, &entry.key))
        {
            merged.push(entry.clone());
        }
    }
    merged
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::super::{NoExternalRefs, SpecVersion, build_plan, parse_spec};
    use super::*;
    use pretty_assertions::assert_eq;

    const OLD: &str = r##"
openapi: 3.0.3
info: {title: Acme, version: '1'}
servers: [{url: https://api.acme.dev}]
paths:
  /customers:
    get:
      summary: List customers
      parameters: [{name: limit, in: query}]
      responses: {'200': {description: ok}}
    post:
      summary: Create customer
      requestBody: {content: {application/json: {example: {name: Ada}}}}
      responses: {'201': {description: ok}}
  /customers/{id}:
    delete: {summary: Delete customer, responses: {'204': {description: ok}}}
  /charges:
    get: {summary: List charges, responses: {'200': {description: ok}}}
"##;

    const NEW: &str = r##"
openapi: 3.0.3
info: {title: Acme, version: '1'}
servers: [{url: https://api.acme.dev}]
paths:
  /customers:
    get:
      summary: List customers
      parameters: [{name: limit, in: query}, {name: created_after, in: query}]
      responses: {'200': {description: ok}}
    post:
      summary: Create customer
      requestBody: {content: {application/json: {example: {name: Ada, phone: '1'}}}}
      responses: {'201': {description: ok}}
  /charges:
    get: {summary: List charges, responses: {'200': {description: ok}}}
  /payouts:
    get: {summary: List payouts, responses: {'200': {description: ok}}}
"##;

    fn plan(text: &str) -> OpenApiPlan {
        let spec = parse_spec(text, "", &NoExternalRefs).expect("valid");
        build_plan(
            "Acme",
            &[SpecVersion {
                label: "",
                spec: &spec,
            }],
            &ImportOptions::default(),
        )
    }

    fn files(plan: &OpenApiPlan) -> BTreeMap<String, String> {
        let mut files = BTreeMap::new();
        visit(&plan.root, &mut |request| {
            files.insert(request.key.clone(), format!("{}.brasa", request.name));
        });
        files
    }

    fn manifest() -> Manifest {
        let old = plan(OLD);
        Manifest::new(
            "2026-10-28",
            ManifestSource {
                kind: "url".into(),
                location: "https://api.acme.dev/openapi.yaml".into(),
                branch: String::new(),
            },
            &ImportOptions::default(),
            &old,
            &files(&old),
        )
    }

    #[test]
    fn manifests_round_trip_through_toml() {
        let manifest = manifest();
        let text = manifest.to_toml();
        assert!(text.starts_with("# Written by Brasa"));
        assert!(text.contains("[requests.\"POST /customers\"]"), "{text}");
        assert_eq!(Manifest::from_toml(&text).expect("parses"), manifest);
        assert!(Manifest::from_toml("version = 1").is_err());
    }

    #[test]
    fn diffs_tell_added_changed_removed_and_unchanged() {
        let changes = diff_plan(&manifest(), &plan(NEW));
        let summary: Vec<(String, ChangeKind, Vec<String>)> = changes
            .into_iter()
            .map(|change| (change.key, change.kind, change.details))
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "GET /customers".into(),
                    ChangeKind::Changed,
                    vec!["query: +created_after".into()]
                ),
                (
                    "POST /customers".into(),
                    ChangeKind::Changed,
                    vec!["body: +phone".into()]
                ),
                ("GET /charges".into(), ChangeKind::Unchanged, vec![]),
                ("GET /payouts".into(), ChangeKind::Added, vec![]),
                ("DELETE /customers/{id}".into(), ChangeKind::Removed, vec![]),
            ]
        );
    }

    #[test]
    fn updates_keep_what_the_user_changed() {
        let old_plan = plan(OLD);
        let manifest = manifest();
        let new_plan = plan(NEW);
        let find = |plan: &OpenApiPlan, key: &str| {
            let mut found = None;
            visit(&plan.root, &mut |request| {
                if request.key == key {
                    found = Some(request.request.clone());
                }
            });
            found.expect("in the plan")
        };

        // The user set a value, added a header and a script.
        let mut current = find(&old_plan, "GET /customers");
        current.query[0].value = "50".into();
        current.headers.push(KeyValue::new("X-Debug", "1"));
        current.post_script = "test('mine', () => {});".into();
        let updated = update_request(
            &current,
            &manifest.requests["GET /customers"],
            &find(&new_plan, "GET /customers"),
        );
        assert_eq!(
            updated.query,
            vec![
                KeyValue {
                    key: "limit".into(),
                    value: "50".into(),
                    enabled: false
                },
                KeyValue {
                    key: "created_after".into(),
                    value: String::new(),
                    enabled: false
                }
            ]
        );
        assert_eq!(updated.headers, vec![KeyValue::new("X-Debug", "1")]);
        assert_eq!(updated.post_script, "test('mine', () => {});");

        // An untouched body takes the new example, an edited one stays.
        let current = find(&old_plan, "POST /customers");
        let new = find(&new_plan, "POST /customers");
        let updated = update_request(&current, &manifest.requests["POST /customers"], &new);
        assert_eq!(updated.body, new.body);
        let mut edited = current.clone();
        edited.body = Body::Json("{\"name\": \"Grace\"}".into());
        let updated = update_request(&edited, &manifest.requests["POST /customers"], &new);
        assert_eq!(updated.body, edited.body);
    }

    #[test]
    fn body_hashes_are_stable() {
        // Written to manifests, so it must never change between builds.
        assert_eq!(body_hash(&Body::Json("{}".into())), "71235dee35ea04b7");
        assert_eq!(body_hash(&Body::None), "");
    }
}
