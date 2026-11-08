//! Writing an OpenAPI import plan into a [`Workspace`], and updating a collection it created
//! when the spec changes (see `docs/openapi-import.md`).
//!
//! `brasa_format::openapi` builds the plan from the spec and compares it with the manifest of
//! an earlier import; this does the file work, the same way a Postman import does: never
//! overwriting a file it did not write, suffixing names that are taken.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use brasa_format::openapi::{
    ChangeKind, ImportOptions, MANIFEST_FILE, Manifest, ManifestSource, OpenApiPlan, PlanFolder,
    PlanRequest, diff_plan, update_request,
};

use crate::error::WorkspaceError;
use crate::ids::{id_to_path, path_to_id};
use crate::postman::{
    ImportReport, undo_import, unique_named_path, unique_path, write_environment,
};
use crate::sanitize::sanitize_file_name;
use crate::workspace::atomic_write;
use crate::{REQUEST_EXTENSION, Workspace};

/// Where an import comes from and when it happens, for its manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOrigin {
    /// The source of the spec.
    pub source: ManifestSource,
    /// Today, `YYYY-MM-DD`.
    pub date: String,
    /// The options the plan was built with.
    pub options: ImportOptions,
}

/// A collection an OpenAPI import created: its folder and its manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenApiCollection {
    /// The workspace id of the collection folder.
    pub folder: String,
    /// The manifest read from it.
    pub manifest: Manifest,
}

/// The result of [`Workspace::update_openapi`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateReport {
    /// Requests written for new operations.
    pub added: usize,
    /// Requests updated.
    pub changed: usize,
    /// Requests deleted.
    pub removed: usize,
    /// What could not be done, and the plan's own warnings.
    pub warnings: Vec<String>,
}

impl Workspace {
    /// Imports the requests of `plan` that `keep` accepts (by request key), and the plan's
    /// environments.
    ///
    /// Writes a new top-level folder named after the collection (`plan.root.name`, sanitized,
    /// with a ` (2)`, ` (3)`, ... suffix when the name is taken), with the plan's folders inside;
    /// folders left without a selected request are not created. The manifest
    /// ([`MANIFEST_FILE`]) records the source and every request written, for a later update.
    /// Each environment goes to `environments/<name>.env`, its secrets to
    /// `environments/<name>.local.env`, also suffixed when the name is taken. Existing files are
    /// never overwritten.
    pub fn import_openapi(
        &mut self,
        plan: &OpenApiPlan,
        keep: &dyn Fn(&str) -> bool,
        origin: &ImportOrigin,
    ) -> Result<ImportReport, WorkspaceError> {
        let mut created = Vec::new();
        let target = unique_path(self.root(), &sanitize_file_name(&plan.root.name));
        let written = write_plan(self.root(), &target, plan, keep, origin, &mut created);
        if let Err(error) = written {
            undo_import(self.root(), &target, &created);
            return Err(error);
        }
        self.rescan()?;
        Ok(ImportReport {
            created_files: created,
            warnings: plan.warnings.clone(),
        })
    }

    /// The collections an OpenAPI import created: every top-level folder with a manifest.
    /// A manifest that does not parse is skipped.
    pub fn openapi_collections(&self) -> Vec<OpenApiCollection> {
        openapi_collections_in(self.root())
    }

    /// Updates the collection in the folder `collection` (a workspace id) from a new `plan` of
    /// its spec. Of the changes [`diff_plan`] finds, only those `apply` accepts (by request key)
    /// are applied:
    ///
    /// - an added operation is written where the plan puts it, its folders created as needed;
    /// - a changed one is updated in place with [`update_request`], keeping the user's values,
    ///   scripts, tests and an edited body;
    /// - a removed one is deleted.
    ///
    /// A removed operation that is kept stops being tracked, so it is not offered again. The
    /// manifest is rewritten with `origin`. Environments are left as they are.
    pub fn update_openapi(
        &mut self,
        collection: &str,
        plan: &OpenApiPlan,
        apply: &dyn Fn(&str) -> bool,
        origin: &ImportOrigin,
    ) -> Result<UpdateReport, WorkspaceError> {
        let dir = id_to_path(self.root(), collection)?;
        let manifest_path = dir.join(MANIFEST_FILE);
        let text = fs::read_to_string(&manifest_path).map_err(|source| WorkspaceError::Io {
            path: manifest_path.clone(),
            source,
        })?;
        let mut manifest = Manifest::from_toml(&text)
            .map_err(|_| WorkspaceError::NotFound(format!("{collection}/{MANIFEST_FILE}")))?;
        let mut report = UpdateReport {
            warnings: plan.warnings.clone(),
            ..UpdateReport::default()
        };
        let requests = plan_requests(&plan.root);
        let mut files = BTreeMap::new();
        for change in diff_plan(&manifest, plan) {
            let selected = apply(&change.key);
            match change.kind {
                ChangeKind::Unchanged => {
                    if let Some(old) = manifest.requests.get(&change.key) {
                        files.insert(change.key.clone(), old.file.clone());
                    }
                }
                ChangeKind::Added if selected => {
                    let Some((folders, request)) = requests.get(change.key.as_str()) else {
                        continue;
                    };
                    let mut target = dir.clone();
                    for folder in folders {
                        target = target.join(sanitize_file_name(folder));
                    }
                    fs::create_dir_all(&target).map_err(|source| WorkspaceError::Io {
                        path: target.clone(),
                        source,
                    })?;
                    let path = unique_named_path(
                        &target,
                        &sanitize_file_name(&request.name),
                        REQUEST_EXTENSION,
                    );
                    atomic_write(&path, &brasa_format::serialize(&request.request))?;
                    files.insert(change.key.clone(), relative(&dir, &path));
                    report.added += 1;
                }
                ChangeKind::Changed => {
                    let Some(old) = manifest.requests.get(&change.key).cloned() else {
                        continue;
                    };
                    if !selected {
                        // Offered again next time: the manifest keeps what was generated before.
                        continue;
                    }
                    let Some((_, request)) = requests.get(change.key.as_str()) else {
                        continue;
                    };
                    let path = dir.join(&old.file);
                    let current = fs::read_to_string(&path)
                        .ok()
                        .and_then(|text| brasa_format::parse(&text).ok());
                    let Some(current) = current else {
                        report.warnings.push(format!(
                            "{}: {} is missing or does not parse, not updated",
                            change.key, old.file
                        ));
                        continue;
                    };
                    let updated = update_request(&current, &old, &request.request);
                    atomic_write(&path, &brasa_format::serialize(&updated))?;
                    files.insert(change.key.clone(), old.file.clone());
                    report.changed += 1;
                }
                ChangeKind::Removed => {
                    let Some(old) = manifest.requests.remove(&change.key) else {
                        continue;
                    };
                    if selected {
                        let path = dir.join(&old.file);
                        match fs::remove_file(&path) {
                            Ok(()) => {
                                report.removed += 1;
                                remove_empty_folders(&dir, &path);
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                            Err(source) => return Err(WorkspaceError::Io { path, source }),
                        }
                    }
                }
                ChangeKind::Added => {}
            }
        }
        manifest.imported = origin.date.clone();
        manifest.source = origin.source.clone();
        manifest.options = (&origin.options).into();
        // The written and unchanged requests take the plan's fingerprint; the changed ones the
        // user skipped keep their old one.
        manifest.record(plan, &files);
        atomic_write(&manifest_path, &manifest.to_toml())?;
        self.rescan()?;
        Ok(report)
    }
}

/// The collections an OpenAPI import created in the workspace folder `root`, without scanning the
/// workspace: every top-level folder with a manifest. A manifest that does not parse is skipped.
pub fn openapi_collections_in(root: &Path) -> Vec<OpenApiCollection> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut collections: Vec<OpenApiCollection> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let text = fs::read_to_string(entry.path().join(MANIFEST_FILE)).ok()?;
            let manifest = Manifest::from_toml(&text)
                .map_err(|error| log::warn!("{MANIFEST_FILE} in {:?}: {error}", entry.file_name()))
                .ok()?;
            Some(OpenApiCollection {
                folder: path_to_id(root, &entry.path()),
                manifest,
            })
        })
        .collect();
    collections.sort_by(|a, b| a.folder.cmp(&b.folder));
    collections
}

/// Writes the collection folder `target`, its manifest and the environments of `plan`.
fn write_plan(
    root: &Path,
    target: &Path,
    plan: &OpenApiPlan,
    keep: &dyn Fn(&str) -> bool,
    origin: &ImportOrigin,
    created: &mut Vec<String>,
) -> Result<(), WorkspaceError> {
    // A Postman environment on its own has no collection to write.
    if !plan.root.keys().is_empty() {
        create_dir(target)?;
        let mut files = BTreeMap::new();
        write_plan_folder(target, target, root, &plan.root, keep, &mut files, created)?;
        let manifest = Manifest::new(
            &origin.date,
            origin.source.clone(),
            &origin.options,
            plan,
            &files,
        );
        atomic_write(&target.join(MANIFEST_FILE), &manifest.to_toml())?;
    }
    for environment in &plan.environments {
        write_environment(
            root,
            &environment.name,
            &environment.base,
            &environment.secret,
            created,
        )?;
    }
    Ok(())
}

/// Removes the folders above `file` that the deletion left empty, up to `collection`, which
/// stays.
fn remove_empty_folders(collection: &Path, file: &Path) {
    let mut folder = file.parent();
    while let Some(current) = folder {
        if current == collection || !current.starts_with(collection) {
            break;
        }
        let empty = fs::read_dir(current).is_ok_and(|mut entries| entries.next().is_none());
        if !empty || fs::remove_dir(current).is_err() {
            break;
        }
        folder = current.parent();
    }
}

fn create_dir(path: &Path) -> Result<(), WorkspaceError> {
    fs::create_dir(path).map_err(|source| WorkspaceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// `path` relative to `base`, with `/` separators.
fn relative(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Writes the requests of `folder` that `keep` accepts inside `dir`, recursively, creating a
/// folder only when something goes in it. Records each request's file, relative to
/// `collection`, in `files`, and its workspace id in `created`.
fn write_plan_folder(
    dir: &Path,
    collection: &Path,
    root: &Path,
    folder: &PlanFolder,
    keep: &dyn Fn(&str) -> bool,
    files: &mut BTreeMap<String, String>,
    created: &mut Vec<String>,
) -> Result<(), WorkspaceError> {
    for child in &folder.folders {
        if !child.keys().iter().any(|key| keep(key)) {
            continue;
        }
        let child_path: PathBuf = unique_path(dir, &sanitize_file_name(&child.name));
        create_dir(&child_path)?;
        write_plan_folder(&child_path, collection, root, child, keep, files, created)?;
    }
    for request in folder.requests.iter().filter(|request| keep(&request.key)) {
        let path = unique_named_path(dir, &sanitize_file_name(&request.name), REQUEST_EXTENSION);
        atomic_write(&path, &brasa_format::serialize(&request.request))?;
        files.insert(request.key.clone(), relative(collection, &path));
        created.push(path_to_id(root, &path));
    }
    Ok(())
}

/// Every request of a plan by key, with the names of the folders it is in.
fn plan_requests(root: &PlanFolder) -> BTreeMap<&str, (Vec<&str>, &PlanRequest)> {
    fn walk<'a>(
        folder: &'a PlanFolder,
        path: &mut Vec<&'a str>,
        out: &mut BTreeMap<&'a str, (Vec<&'a str>, &'a PlanRequest)>,
    ) {
        for request in &folder.requests {
            out.insert(request.key.as_str(), (path.clone(), request));
        }
        for child in &folder.folders {
            path.push(&child.name);
            walk(child, path, out);
            path.pop();
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut Vec::new(), &mut out);
    out
}
