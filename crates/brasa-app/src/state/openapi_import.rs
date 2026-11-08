//! The import wizard's state (GitHub #201, #211, #222): the format, the source, the loaded specs,
//! Postman exports or Bruno collection, the options, the plan they build and which of its requests are selected, plus
//! the rows of the preview tree.
//! Plain Rust, so the wizard's logic is tested without a window; `views/openapi_import.rs`
//! renders it.
//!
//! Loading a spec reads a file, scans a folder, clones a repository or fetches a URL, and with it
//! every document its `$ref`s point to. It is blocking, the view runs it on the background
//! executor.

use std::collections::{HashMap, HashSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use brasa_core::{Method, ResolvedField, ResolvedRequest};
use brasa_format::bruno::{
    BrunoFile, BrunoImport, is_bruno_file, is_collection_root, parse_bruno_collection,
};
use brasa_format::openapi::{
    CatalogDefinition, ChangeKind, ImportOptions, ManifestSource, NoExternalRefs, OpenApiPlan,
    PlanChange, PlanFolder, RefLoader, Spec, SpecVersion, build_plan, diff_plan,
    looks_like_catalog, looks_like_spec, parse_catalog, parse_spec,
};
use brasa_format::postman::{
    ImportPlan, ImportedEnvironment, IssueLevel, MANIFEST_KIND, PostmanFormat, PostmanOptions,
    RequestIssue, build_postman_plan, detect_format, parse_collection, parse_environment,
};
use brasa_runner::Client;
use brasa_workspace::OpenApiCollection;
use rust_i18n::t;

use crate::state::env_color::EnvColor;
use crate::state::number::format_integer;

/// What the wizard imports. Detected from the content; the first step's chips preselect it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportFormat {
    /// OpenAPI or Swagger specs.
    OpenApi,
    /// Postman collections, environments and globals.
    Postman,
    /// A Bruno collection, `.bru` or OpenCollection YAML (#222).
    Bruno,
}

/// The sources of `format`, in the order of the source tabs.
pub fn sources_of(format: ImportFormat) -> &'static [SourceKind] {
    match format {
        ImportFormat::OpenApi => &[
            SourceKind::File,
            SourceKind::Url,
            SourceKind::Paste,
            SourceKind::Folder,
            SourceKind::Git,
        ],
        ImportFormat::Postman => &[SourceKind::File, SourceKind::Paste, SourceKind::Folder],
        // A Bruno collection is a folder, so it comes first.
        ImportFormat::Bruno => &[SourceKind::Folder, SourceKind::File, SourceKind::Paste],
    }
}

/// Where the wizard reads the spec from (the source tabs of the first step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    /// A spec file (or a `catalog-info.yaml`) picked or dropped.
    File,
    /// A spec fetched from a URL.
    Url,
    /// Spec text pasted into the wizard.
    Paste,
    /// Every spec of a folder and its subfolders (GitHub #202).
    Folder,
    /// Every spec of a branch of a Git repository, cloned to a temporary folder (GitHub #202).
    Git,
}

/// The wizard's steps, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Pick the source and load the spec.
    Source,
    /// Collection name, folders and what to generate.
    Options,
    /// The tree of requests, to choose which ones to import.
    Preview,
    /// The import finished.
    Done,
}

/// A spec read by the wizard.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedSpec {
    /// The spec.
    pub spec: Spec,
    /// What the source list shows under the title: the file name, its path in the folder or
    /// repository, or the URL. Empty for pasted text.
    pub source: String,
    /// Where it was read from (a canonical path or a URL), to read each file once.
    pub location: String,
}

/// A Postman export read by the wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostmanFile {
    /// What it is.
    pub format: PostmanFormat,
    /// The collection or environment name.
    pub name: String,
    /// The file it came from, as the source list shows it. Empty for pasted text.
    pub source: String,
    /// `info._postman_id` of a collection.
    pub postman_id: String,
    /// The format version of a collection (`v2.1`).
    pub version: String,
    /// How many requests a collection has, or variables an environment.
    pub items: usize,
    /// How many folders a collection has, or secret variables an environment.
    pub others: usize,
    /// The export's text, parsed again when the options change.
    text: Arc<str>,
}

impl PostmanFile {
    /// Reads `text`. `None` when it is not a Postman export; an error when it says it is one and
    /// does not parse.
    pub fn read(text: &str, source: &str) -> Option<Result<PostmanFile, String>> {
        let format = detect_format(text)?;
        let file = |name: String, postman_id, version, items, others| PostmanFile {
            format,
            name,
            source: source.to_string(),
            postman_id,
            version,
            items,
            others,
            text: Arc::from(text),
        };
        Some(if format == PostmanFormat::Collection {
            parse_collection(text, &PostmanOptions::default())
                .map(|plan| {
                    let version = plan.schema_version.trim_end_matches(".0").to_string();
                    let (requests, folders) = count_folder(&plan.root);
                    file(
                        plan.collection_name,
                        plan.postman_id,
                        version,
                        requests,
                        folders,
                    )
                })
                .map_err(|error| error.to_string())
        } else {
            parse_environment(text)
                .map(|environment| {
                    file(
                        environment.name,
                        String::new(),
                        String::new(),
                        environment.base.len() + environment.secret.len(),
                        environment.secret.len(),
                    )
                })
                .map_err(|error| error.to_string())
        })
    }
}

/// How many requests and folders `folder` has under it.
fn count_folder(folder: &brasa_format::postman::ImportFolder) -> (usize, usize) {
    folder.folders.iter().fold(
        (folder.requests.len(), folder.folders.len()),
        |(requests, folders), child| {
            let (more_requests, more_folders) = count_folder(child);
            (requests + more_requests, folders + more_folders)
        },
    )
}

/// One environment of a Bruno collection, as the wizard lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrunoEnvironment {
    /// Its name.
    pub name: String,
    /// How many variables it has, secret ones included.
    pub variables: usize,
    /// How many of them are secret, whose values are not in the collection.
    pub secrets: usize,
    /// Its color in Bruno, mapped to the nearest of Brasa's.
    pub color: Option<EnvColor>,
}

/// A Bruno collection read by the wizard (#222).
#[derive(Debug, Clone, PartialEq)]
pub struct BrunoSource {
    /// The collection name.
    pub name: String,
    /// Where it came from, as the source list shows it: its root file, or the file read.
    pub source: String,
    /// Its formats, as the source list shows them: `.bru`, `YAML` or both.
    pub formats: String,
    /// How many requests it has.
    pub requests: usize,
    /// How many folders it has.
    pub folders: usize,
    /// Its environments, in file order.
    pub environments: Vec<BrunoEnvironment>,
    /// Whether it has a root file (`bruno.json` or `opencollection.yml`) rather than being loose
    /// request files.
    pub has_root: bool,
    /// The collection's files, parsed again when the options change.
    files: Arc<[BrunoFile]>,
    /// The name used when the collection names none.
    fallback: String,
}

impl BrunoSource {
    /// Reads the files of a collection. `None` when they are not one; `fallback` names it when
    /// it names none, `shown` is how the source list names where it came from.
    pub fn read(files: Vec<BrunoFile>, fallback: &str, shown: &str) -> Option<BrunoSource> {
        let import = parse_bruno_collection(&files, fallback, &PostmanOptions::default()).ok()?;
        let (requests, folders) = count_folder(&import.plan.root);
        if requests == 0 && import.environments.is_empty() {
            return None;
        }
        let ext = |file: &BrunoFile, extensions: &[&str]| {
            extensions
                .iter()
                .any(|extension| file.path.to_ascii_lowercase().ends_with(extension))
        };
        let bru = files.iter().any(|file| ext(file, &[".bru"]));
        let yaml = files.iter().any(|file| ext(file, &[".yml", ".yaml"]));
        let formats = match (bru, yaml) {
            (true, true) => ".bru, YAML",
            (false, true) => "YAML",
            _ => ".bru",
        };
        let root = files
            .iter()
            .map(|file| file.path.as_str())
            .filter(|path| is_collection_root(path))
            .min_by_key(|path| path.matches('/').count())
            .map(str::to_string);
        Some(BrunoSource {
            name: import.plan.collection_name.clone(),
            source: root.clone().unwrap_or_else(|| shown.to_string()),
            formats: formats.to_string(),
            requests,
            folders,
            environments: bruno_environments(&import),
            has_root: root.is_some(),
            files: Arc::from(files),
            fallback: fallback.to_string(),
        })
    }

    /// The collection with `options`.
    fn parse(&self, options: &PostmanOptions) -> Option<BrunoImport> {
        parse_bruno_collection(&self.files, &self.fallback, options).ok()
    }

    /// This collection and the files of `other`, read together: several request files picked
    /// at once.
    fn combined(&self, other: &BrunoSource) -> Option<BrunoSource> {
        let files: Vec<BrunoFile> = self
            .files
            .iter()
            .chain(other.files.iter())
            .cloned()
            .collect();
        BrunoSource::read(files, &self.fallback, &self.source)
    }
}

/// The environments of `import` as the wizard lists them.
fn bruno_environments(import: &BrunoImport) -> Vec<BrunoEnvironment> {
    import
        .environments
        .iter()
        .map(|environment| BrunoEnvironment {
            name: environment.name.clone(),
            variables: environment.base.len() + environment.secret.len(),
            secrets: environment.secret.len(),
            color: import
                .environment_colors
                .iter()
                .find(|(name, _)| *name == environment.name)
                .and_then(|(_, hex)| EnvColor::nearest(hex)),
        })
        .collect()
}

/// A file that looked like a spec, or that a catalog pointed to, and could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The file, as the source list shows it.
    pub source: String,
    /// Why.
    pub error: String,
}

/// A Backstage catalog file that was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogFile {
    /// The file, as the source list shows it.
    pub source: String,
    /// How many OpenAPI APIs it declares.
    pub apis: usize,
}

/// Everything a source gave.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Loaded {
    /// The specs read, in source order.
    pub specs: Vec<LoadedSpec>,
    /// The files that could not be read.
    pub failures: Vec<Failure>,
    /// The catalog files read.
    pub catalogs: Vec<CatalogFile>,
    /// The Postman exports read.
    pub postman: Vec<PostmanFile>,
    /// The Bruno collection read.
    pub bruno: Option<BrunoSource>,
}

impl Loaded {
    /// Adds what another file of the source gave.
    pub fn merge(&mut self, other: Loaded) {
        self.specs.extend(other.specs);
        self.failures.extend(other.failures);
        self.catalogs.extend(other.catalogs);
        self.postman.extend(other.postman);
        self.bruno = match (self.bruno.take(), other.bruno) {
            (Some(mine), Some(theirs)) => mine.combined(&theirs).or(Some(mine)),
            (mine, theirs) => mine.or(theirs),
        };
    }

    /// The format what was read is in: specs win, as they cannot be told apart from a Postman file
    /// of the same folder by anything else. `None` when nothing was read.
    pub fn format(&self) -> Option<ImportFormat> {
        // A Bruno root file says what the folder is better than anything else.
        if self.bruno.as_ref().is_some_and(|bruno| bruno.has_root) {
            return Some(ImportFormat::Bruno);
        }
        if !self.specs.is_empty() {
            Some(ImportFormat::OpenApi)
        } else if !self.postman.is_empty() {
            Some(ImportFormat::Postman)
        } else if self.bruno.is_some() {
            Some(ImportFormat::Bruno)
        } else {
            None
        }
    }
}

/// The `Accept` header of a spec download: the YAML and JSON types servers use for specs.
const SPEC_ACCEPT: &str = "application/yaml, application/x-yaml, application/json;q=0.9, */*;q=0.8";

/// The folders a folder scan never enters, besides hidden ones: dependencies and build output.
const SKIPPED_FOLDERS: &[&str] = &["node_modules", "target", "vendor", "dist", "build"];

/// How deep a folder scan goes below the folder picked.
const MAX_SCAN_DEPTH: usize = 8;

/// How many files a folder scan looks at, at most.
const MAX_SCAN_FILES: usize = 5_000;

/// The biggest file a folder scan reads.
const MAX_SCAN_FILE_SIZE: u64 = 32 * 1024 * 1024;

/// How many bytes of a `.zip` export are read, unpacked, at most: a guard against a file that
/// unpacks to far more than it looks.
const MAX_ZIP_SIZE: u64 = 512 * 1024 * 1024;

/// Reads the file at `path`: a spec, or a Backstage catalog whose OpenAPI APIs are read in turn.
/// `shown` is how the source list names it. A catalog with no readable spec is not an error:
/// the result lists the failures.
pub fn load_path(path: &Path, shown: &str, client: &Client) -> Result<Loaded, String> {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        return load_zip(path);
    }
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    if let Some(found) = load_postman(&text, shown) {
        return found;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if is_bruno_file(&name, &text) {
        // The root file of a collection stands for its whole folder.
        if is_collection_root(&name)
            && let Some(folder) = path.parent()
        {
            return load_bruno_folder(folder);
        }
        let stem = name
            .rsplit_once('.')
            .map_or(name.as_str(), |(stem, _)| stem);
        let files = vec![BrunoFile {
            path: name.clone(),
            text,
        }];
        return BrunoSource::read(files, stem, shown)
            .map(|bruno| Loaded {
                bruno: Some(bruno),
                ..Loaded::default()
            })
            .ok_or_else(|| "not a Bruno request".to_string());
    }
    let location = canonical(path);
    if is_catalog_name(path) || (!looks_like_spec(&text) && looks_like_catalog(&text)) {
        return load_catalog(&text, &location, shown, client);
    }
    let spec = parse_spec(&text, &location, &SourceLoader { client })
        .map_err(|error| error.to_string())?;
    Ok(Loaded {
        specs: vec![LoadedSpec {
            spec,
            source: shown.to_string(),
            location,
        }],
        ..Loaded::default()
    })
}

/// What `text` gives when it is a Postman export.
fn load_postman(text: &str, shown: &str) -> Option<Result<Loaded, String>> {
    let file = PostmanFile::read(text, shown)?;
    Some(file.map(|file| Loaded {
        postman: vec![file],
        ..Loaded::default()
    }))
}

/// Reads the Postman exports of a `.zip`, Postman's data export. What is not one is ignored, a
/// file that says it is one and does not parse is listed as a failure.
fn load_zip(path: &Path) -> Result<Loaded, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    if let Some(found) = load_bruno_zip(&mut archive, path)? {
        return Ok(found);
    }
    let mut loaded = Loaded::default();
    let mut left = MAX_ZIP_SIZE;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let name = entry
            .name()
            .map_err(|error| error.to_string())?
            .into_owned();
        if !entry.is_file() || !name.to_ascii_lowercase().ends_with(".json") {
            continue;
        }
        let mut text = String::new();
        let read = entry.take(left).read_to_string(&mut text);
        let Ok(read) = read else {
            continue;
        };
        left -= read as u64;
        if left == 0 {
            return Err("the .zip unpacks to more than 512 MB".to_string());
        }
        match PostmanFile::read(&text, &name) {
            Some(Ok(file)) => loaded.postman.push(file),
            Some(Err(error)) => loaded.failures.push(Failure {
                source: name,
                error,
            }),
            None => {}
        }
    }
    if loaded.postman.is_empty() && loaded.failures.is_empty() {
        return Err("the .zip has no Postman export".to_string());
    }
    Ok(loaded)
}

/// The extensions of the files a Bruno collection is made of.
const BRUNO_EXTENSIONS: &[&str] = &[".bru", ".yml", ".yaml", ".json", ".env"];

/// Whether `name` is a file a Bruno collection is made of.
fn is_bruno_part(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    BRUNO_EXTENSIONS
        .iter()
        .any(|extension| lower.ends_with(extension))
}

/// The Bruno collection of a `.zip`, `None` when it holds no `bruno.json`, `opencollection.yml`
/// or `.bru` file.
fn load_bruno_zip<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &Path,
) -> Result<Option<Loaded>, String> {
    let mut entries: Vec<(usize, String)> = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|error| error.to_string())?;
        if entry.is_file() {
            let name = entry
                .name()
                .map_err(|error| error.to_string())?
                .into_owned();
            entries.push((index, name));
        }
    }
    let is_bruno = entries
        .iter()
        .any(|(_, name)| is_collection_root(name) || name.to_ascii_lowercase().ends_with(".bru"));
    if !is_bruno {
        return Ok(None);
    }
    let mut files = Vec::new();
    let mut left = MAX_ZIP_SIZE;
    for (index, name) in entries.into_iter().filter(|(_, name)| is_bruno_part(name)) {
        let entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let mut text = String::new();
        let Ok(read) = entry.take(left).read_to_string(&mut text) else {
            continue;
        };
        left -= read as u64;
        if left == 0 {
            return Err("the .zip unpacks to more than 512 MB".to_string());
        }
        files.push(BrunoFile { path: name, text });
    }
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let shown = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(BrunoSource::read(files, &stem, &shown).map(|bruno| Loaded {
        bruno: Some(bruno),
        ..Loaded::default()
    }))
}

/// The folder at most this deep below the folder picked that holds a Bruno root file.
const MAX_BRUNO_ROOT_DEPTH: usize = 2;

/// The shallowest folder under `dir`, `dir` included, with `bruno.json` or `opencollection.yml`.
fn find_bruno_root(dir: &Path, depth: usize) -> Option<PathBuf> {
    let has_root = |folder: &Path| {
        [
            brasa_format::bruno::BRUNO_JSON,
            brasa_format::bruno::OPENCOLLECTION_YML,
        ]
        .iter()
        .any(|name| folder.join(name).is_file())
    };
    if has_root(dir) {
        return Some(dir.to_path_buf());
    }
    if depth >= MAX_BRUNO_ROOT_DEPTH {
        return None;
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            !name.starts_with('.') && !SKIPPED_FOLDERS.contains(&name.as_str())
        })
        .map(|entry| entry.path())
        .collect();
    children.sort();
    children
        .iter()
        .find_map(|child| find_bruno_root(child, depth + 1))
}

/// The files of a Bruno collection under `dir`, with paths relative to it.
fn collect_bruno_files(root: &Path, dir: &Path, depth: usize, out: &mut Vec<BrunoFile>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if out.len() >= MAX_SCAN_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if depth < MAX_SCAN_DEPTH
                && !name.starts_with('.')
                && !SKIPPED_FOLDERS.contains(&name.as_str())
            {
                collect_bruno_files(root, &path, depth + 1, out);
            }
        } else if kind.is_file()
            && is_bruno_part(&name)
            && entry
                .metadata()
                .is_ok_and(|metadata| metadata.len() <= MAX_SCAN_FILE_SIZE)
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            out.push(BrunoFile {
                path: relative_name(root, &path),
                text,
            });
        }
    }
}

/// Reads the Bruno collection whose root is `folder`.
fn load_bruno_folder(folder: &Path) -> Result<Loaded, String> {
    let mut files = Vec::new();
    collect_bruno_files(folder, folder, 0, &mut files);
    let name = folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    BrunoSource::read(files, &name, &name)
        .map(|bruno| Loaded {
            bruno: Some(bruno),
            ..Loaded::default()
        })
        .ok_or_else(|| "the folder has no Bruno request".to_string())
}

/// Reads several files, what they give put together. A file that cannot be read is listed as a
/// failure, so the others still count.
pub fn load_files(paths: &[PathBuf], client: &Client) -> Result<Loaded, String> {
    let mut loaded = Loaded::default();
    for path in paths {
        let shown = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        // A folder dropped on the window reads like the Folder source.
        let found = if path.is_dir() {
            scan_folder(path, client)
        } else {
            load_path(path, &shown, client)
        };
        match found {
            Ok(found) => loaded.merge(found),
            Err(error) if paths.len() > 1 => loaded.failures.push(Failure {
                source: shown,
                error,
            }),
            Err(error) => return Err(error),
        }
    }
    Ok(loaded)
}

/// Reads pasted text: a Postman export or a spec.
pub fn load_pasted(text: &str) -> Result<Loaded, String> {
    if let Some(found) = load_postman(text, "") {
        return found;
    }
    for name in ["request.bru", "request.yml"] {
        if is_bruno_file(name, text) {
            let files = vec![BrunoFile {
                path: name.to_string(),
                text: text.to_string(),
            }];
            if let Some(bruno) = BrunoSource::read(files, "request", "") {
                return Ok(Loaded {
                    bruno: Some(bruno),
                    ..Loaded::default()
                });
            }
        }
    }
    load_text(text).map(|spec| Loaded {
        specs: vec![spec],
        ..Loaded::default()
    })
}

/// Whether `path` is named like a Backstage catalog file.
fn is_catalog_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.eq_ignore_ascii_case("catalog-info.yaml")
                || name.eq_ignore_ascii_case("catalog-info.yml")
        })
}

fn canonical(path: &Path) -> String {
    dunce::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Reads the OpenAPI APIs of the catalog text found at `location`.
fn load_catalog(
    text: &str,
    location: &str,
    shown: &str,
    client: &Client,
) -> Result<Loaded, String> {
    let apis = parse_catalog(text).map_err(|error| error.to_string())?;
    let mut loaded = Loaded {
        catalogs: vec![CatalogFile {
            source: shown.to_string(),
            apis: apis.len(),
        }],
        ..Loaded::default()
    };
    let directory = Path::new(location)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let shown_directory = shown
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    for api in apis {
        let result = match &api.definition {
            CatalogDefinition::Inline(spec) => parse_spec(spec, location, &SourceLoader { client })
                .map(|spec| LoadedSpec {
                    spec,
                    source: format!("{shown} ({})", api.name),
                    location: format!("{location}#{}", api.name),
                })
                .map_err(|error| (format!("{shown} ({})", api.name), error.to_string())),
            CatalogDefinition::Reference(reference) if is_url(reference) => {
                load_url(reference, client).map_err(|error| (reference.clone(), error))
            }
            CatalogDefinition::Reference(reference) => {
                let relative = reference.trim_start_matches("./");
                let shown = if shown_directory.is_empty() {
                    relative.to_string()
                } else {
                    format!("{shown_directory}/{relative}")
                };
                load_path(&directory.join(relative), &shown, client)
                    .map_err(|error| (shown.clone(), error))
                    .map(|mut found| found.specs.remove(0))
            }
        };
        match result {
            Ok(spec) => loaded.specs.push(spec),
            Err((source, error)) => loaded.failures.push(Failure { source, error }),
        }
    }
    Ok(loaded)
}

/// Reads every spec under `root`: files that look like a spec root, and the specs of every
/// catalog file. Hidden folders, dependency and build folders are skipped. A spec found both
/// directly and through a catalog is read once.
pub fn scan_folder(root: &Path, client: &Client) -> Result<Loaded, String> {
    // A Bruno collection picked by its own folder.
    if find_bruno_root(root, MAX_BRUNO_ROOT_DEPTH).is_some() {
        return load_bruno_folder(root);
    }
    let mut files = Vec::new();
    collect_files(root, 0, &mut files);
    files.sort();
    let mut loaded = Loaded::default();
    let mut seen: HashSet<String> = HashSet::new();
    // Catalogs first, so their specs keep the catalog's order.
    let (catalogs, others): (Vec<PathBuf>, Vec<PathBuf>) =
        files.into_iter().partition(|path| is_catalog_name(path));
    for path in catalogs.into_iter().chain(others) {
        let shown = relative_name(root, &path);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match PostmanFile::read(&text, &shown) {
            Some(Ok(file)) => {
                loaded.postman.push(file);
                continue;
            }
            Some(Err(error)) => {
                loaded.failures.push(Failure {
                    source: shown,
                    error,
                });
                continue;
            }
            None => {}
        }
        let catalog = is_catalog_name(&path);
        if !catalog && !looks_like_spec(&text) {
            continue;
        }
        if !catalog && seen.contains(&canonical(&path)) {
            continue;
        }
        match load_path(&path, &shown, client) {
            Ok(found) => {
                for spec in found.specs {
                    if seen.insert(spec.location.clone()) {
                        loaded.specs.push(spec);
                    }
                }
                loaded.failures.extend(found.failures);
                loaded.catalogs.extend(found.catalogs);
            }
            Err(error) => loaded.failures.push(Failure {
                source: shown,
                error,
            }),
        }
    }
    // Nothing else found: a Bruno collection a folder or two below.
    if loaded.specs.is_empty()
        && loaded.postman.is_empty()
        && let Some(bruno_root) = find_bruno_root(root, 0)
    {
        return load_bruno_folder(&bruno_root);
    }
    Ok(loaded)
}

/// The YAML and JSON files under `dir`, up to [`MAX_SCAN_DEPTH`] folders deep and
/// [`MAX_SCAN_FILES`] files.
fn collect_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if out.len() >= MAX_SCAN_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            if depth < MAX_SCAN_DEPTH && !SKIPPED_FOLDERS.contains(&name.as_str()) {
                collect_files(&path, depth + 1, out);
            }
        } else if kind.is_file() {
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase);
            let small = entry
                .metadata()
                .is_ok_and(|metadata| metadata.len() <= MAX_SCAN_FILE_SIZE);
            if matches!(extension.as_deref(), Some("yaml" | "yml" | "json")) && small {
                out.push(path);
            }
        }
    }
}

/// `path` relative to `root`, with `/` separators.
fn relative_name(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Clones the last commit of `branch` (the default branch when empty) of `url` into a new
/// temporary folder and reads its specs. The folder is deleted when the returned guard drops.
pub fn clone_repository(
    git: &telaio_git::Git,
    url: &str,
    branch: &str,
    client: &Client,
) -> Result<(tempfile::TempDir, Loaded), String> {
    let url = url.trim();
    if url.is_empty() {
        return Err("no repository".to_string());
    }
    let parent = tempfile::Builder::new()
        .prefix("brasa-openapi-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let target = parent.path().join("repository");
    let branch = Some(branch.trim()).filter(|branch| !branch.is_empty());
    git.clone_shallow(url, branch, &target)
        .map_err(|error| error.to_string())?;
    let loaded = scan_folder(&target, client)?;
    Ok((parent, loaded))
}

/// Fetches the spec at `url` with `client`, and its external references relative to it.
pub fn load_url(url: &str, client: &Client) -> Result<LoadedSpec, String> {
    let url = url.trim();
    if !is_url(url) {
        return Err("the URL must start with http:// or https://".to_string());
    }
    let text = fetch(client, url)?;
    let spec =
        parse_spec(&text, url, &SourceLoader { client }).map_err(|error| error.to_string())?;
    Ok(LoadedSpec {
        spec,
        source: url.to_string(),
        location: url.to_string(),
    })
}

/// Reads pasted spec text, which has no location for external references.
pub fn load_text(text: &str) -> Result<LoadedSpec, String> {
    let spec = parse_spec(text, "", &NoExternalRefs).map_err(|error| error.to_string())?;
    Ok(LoadedSpec {
        spec,
        source: String::new(),
        location: String::new(),
    })
}

/// Downloads `url` as text, failing on anything but a 2xx status.
fn fetch(client: &Client, url: &str) -> Result<String, String> {
    let request = ResolvedRequest {
        method: Method::Get,
        url: url.to_string(),
        headers: vec![ResolvedField {
            name: "Accept".to_string(),
            value: SPEC_ACCEPT.to_string(),
        }],
        ..ResolvedRequest::default()
    };
    let response = client.send(&request).map_err(|error| error.to_string())?;
    if !(200..300).contains(&response.status) {
        return Err(format!("the server answered HTTP {}", response.status));
    }
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

/// Loads the documents a spec's external `$ref`s point to: files relative to a file, URLs
/// relative to a URL, and absolute URLs from anywhere.
struct SourceLoader<'a> {
    client: &'a Client,
}

impl RefLoader for SourceLoader<'_> {
    fn load(&self, base: &str, reference: &str) -> Result<(String, String), String> {
        if is_url(reference) || is_url(base) {
            let url = join_url(base, reference);
            let text = fetch(self.client, &url)?;
            return Ok((url, text));
        }
        let path = Path::new(base)
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(reference);
        let path = dunce::canonicalize(&path).map_err(|error| error.to_string())?;
        let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        Ok((path.to_string_lossy().into_owned(), text))
    }
}

fn is_url(text: &str) -> bool {
    text.starts_with("http://") || text.starts_with("https://")
}

/// Resolves `reference` against the URL `base`, with `.` and `..` segments removed.
pub fn join_url(base: &str, reference: &str) -> String {
    if is_url(reference) {
        return reference.to_string();
    }
    let Some((scheme, rest)) = base.split_once("://") else {
        return reference.to_string();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (authority, path) = rest.split_at(authority_end);
    if let Some(rest) = reference.strip_prefix("//") {
        return format!("{scheme}://{rest}");
    }
    let joined = if reference.starts_with('/') {
        reference.to_string()
    } else {
        let path = path.split(['?', '#']).next().unwrap_or_default();
        let directory = path.rsplit_once('/').map_or("", |(directory, _)| directory);
        format!("{directory}/{reference}")
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/').skip(1) {
        match segment {
            "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    format!("{scheme}://{authority}/{}", segments.join("/"))
}

/// The selection state of a folder of the preview tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// Every request under it is selected.
    All,
    /// Some are.
    Some,
    /// None is.
    None,
}

/// One visible row of the preview tree.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewRow {
    /// The nesting level, 0 at the top of the collection.
    pub depth: usize,
    /// The folder's path or the request's key, unique among the rows.
    pub id: String,
    /// The folder or request name.
    pub name: String,
    /// What the row is.
    pub kind: RowKind,
}

/// The kinds of [`PreviewRow`].
#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    /// A folder.
    Folder {
        /// Whether its children are shown.
        open: bool,
        /// Its selection state.
        check: Check,
        /// How many requests under it are selected.
        selected: usize,
        /// How many requests there are under it.
        total: usize,
    },
    /// A request.
    Request {
        /// The HTTP method.
        method: Method,
        /// Whether it is selected.
        checked: bool,
        /// What the preview shows at its right, for a Postman request: its path.
        path: String,
        /// The most serious thing to tell about it.
        issue: Option<RequestIssue>,
    },
}

/// The short text of `issue`, for a row of the preview and the import report.
pub fn issue_text(issue: &RequestIssue) -> String {
    match issue {
        RequestIssue::ScriptConverted => t!("import.issue.script_converted").into_owned(),
        RequestIssue::ScriptPartial { converted, total } => t!(
            "import.issue.script_partial",
            converted = format_integer(*converted as u64),
            total = format_integer(*total as u64)
        )
        .into_owned(),
        RequestIssue::SendRequest => t!("import.issue.send_request").into_owned(),
        RequestIssue::ScriptNotConverted => t!("import.issue.script_not_converted").into_owned(),
        RequestIssue::OAuth2Token => t!("import.issue.oauth2").into_owned(),
        RequestIssue::AuthUnsupported(kind) => {
            t!("import.issue.auth_unsupported", kind = kind.as_str()).into_owned()
        }
        RequestIssue::FormDataFile => t!("import.issue.form_data_file").into_owned(),
        RequestIssue::FormDataManyFiles => t!("import.issue.form_data_many").into_owned(),
        RequestIssue::BodyUnsupported(kind) => {
            t!("import.issue.body_unsupported", kind = kind.as_str()).into_owned()
        }
    }
}

/// The text of the import report: every request with something to look at, then the warnings of
/// the import. Empty when there is nothing to report.
pub fn report_text(
    collection: &str,
    issues: &[(String, Vec<RequestIssue>)],
    warnings: &[String],
) -> String {
    let worth: Vec<_> = issues
        .iter()
        .filter(|(_, issues)| issues.iter().any(|issue| issue.level() != IssueLevel::Note))
        .collect();
    if worth.is_empty() && warnings.is_empty() {
        return String::new();
    }
    let mut text = format!("# {}\n\n", t!("import.report.title", name = collection));
    if !worth.is_empty() {
        text.push_str(&format!("## {}\n\n", t!("import.report.requests")));
        for (name, issues) in worth {
            let notes: Vec<String> = issues
                .iter()
                .filter(|issue| issue.level() != IssueLevel::Note)
                .map(issue_text)
                .collect();
            text.push_str(&format!("- `{name}`: {}\n", notes.join(", ")));
        }
        text.push('\n');
    }
    if !warnings.is_empty() {
        text.push_str(&format!("## {}\n\n", t!("import.report.warnings")));
        for warning in warnings {
            text.push_str(&format!("- {warning}\n"));
        }
    }
    text
}

/// The separator of folder paths in [`PreviewRow::id`], a character no folder name has.
const PATH_SEPARATOR: char = '\u{1f}';

/// The wizard's state.
pub struct Wizard {
    /// The current step.
    pub step: Step,
    /// What is being imported.
    pub format: ImportFormat,
    /// The selected source tab.
    pub source: SourceKind,
    /// The import options of the second step.
    pub options: ImportOptions,
    /// The "Convert" options of a Postman import.
    pub postman_options: PostmanOptions,
    /// The Postman exports loaded in the first step.
    postman: Vec<PostmanFile>,
    /// Which of them are imported, one flag per file.
    postman_enabled: Vec<bool>,
    /// The Bruno collection loaded in the first step.
    bruno: Option<BrunoSource>,
    /// Which of its environments are imported, one flag per environment.
    bruno_environments: Vec<bool>,
    /// The "Convert" options of a Bruno import: Postman's, with the docs on.
    pub bruno_options: PostmanOptions,
    /// Whether a Bruno import writes the environments' colors to `.brasa/project.toml`.
    pub bruno_colors: bool,
    /// What to tell about the requests of the plan, for a Postman or Bruno import.
    issues: HashMap<String, Vec<RequestIssue>>,
    /// See [`Self::compat_counts`].
    compat: (usize, usize, usize),
    /// The specs loaded in the first step.
    specs: Vec<LoadedSpec>,
    /// Which of them are imported (the Versions list), one flag per spec.
    enabled: Vec<bool>,
    /// The files of the source that could not be read.
    failures: Vec<Failure>,
    /// The catalog files of the source.
    catalogs: Vec<CatalogFile>,
    /// The plan of the loaded specs with the current options and collection name.
    plan: Option<Arc<OpenApiPlan>>,
    /// The plan with every option on, for the summaries of what the options generate.
    full_plan: Option<OpenApiPlan>,
    /// The requests the user unchecked, by key. Kept across plan rebuilds, keys do not change.
    excluded: HashSet<String>,
    /// The collapsed folders, by path.
    closed: HashSet<String>,
    /// The preview rows, rebuilt when the plan, the selection or a folder changes.
    rows: Arc<[PreviewRow]>,
    /// `(selected, total, folders)` of the plan, counted with the rows so render reads them.
    counts: (usize, usize, usize),
    /// The collection an earlier import of the same source created, if any (GitHub #203).
    existing: Option<OpenApiCollection>,
    /// Whether the Preview step updates [`Self::existing`] instead of importing a new
    /// collection.
    update: bool,
    /// What changed since [`Self::existing`] was imported, the unchanged operations left out:
    /// added first, then changed, then removed.
    changes: Arc<[PlanChange]>,
    /// The changes the user unchecked, by request key. Removed operations start unchecked.
    skipped: HashSet<String>,
    /// How many operations are unchanged.
    unchanged: usize,
}

/// The kind of manifest source of `kind`, as written in `.openapi.toml`.
pub fn source_kind_name(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::File => "file",
        SourceKind::Url => "url",
        SourceKind::Paste => "paste",
        SourceKind::Folder => "folder",
        SourceKind::Git => "git",
    }
}

/// The source kind of a manifest's `kind`, `None` for pasted text or an unknown kind: those
/// cannot be read again.
pub fn source_kind_of(name: &str) -> Option<SourceKind> {
    match name {
        "file" => Some(SourceKind::File),
        "url" => Some(SourceKind::Url),
        "folder" => Some(SourceKind::Folder),
        "git" => Some(SourceKind::Git),
        _ => None,
    }
}

/// The collection of `collections` imported from `source`. Pasted text has no source to match.
pub fn find_existing(
    collections: Vec<OpenApiCollection>,
    source: &ManifestSource,
) -> Option<OpenApiCollection> {
    if source.kind == "paste" || source.location.is_empty() {
        return None;
    }
    collections.into_iter().find(|collection| {
        let known = &collection.manifest.source;
        known.kind == source.kind
            && known.location == source.location
            && known.branch.trim() == source.branch.trim()
    })
}

impl Wizard {
    /// A wizard on its first step, with `source` selected.
    pub fn new(format: ImportFormat, source: SourceKind) -> Self {
        Wizard {
            step: Step::Source,
            format,
            source,
            options: ImportOptions::default(),
            postman_options: PostmanOptions::default(),
            postman: Vec::new(),
            postman_enabled: Vec::new(),
            bruno: None,
            bruno_environments: Vec::new(),
            bruno_options: PostmanOptions {
                docs: true,
                ..PostmanOptions::default()
            },
            bruno_colors: true,
            issues: HashMap::new(),
            compat: (0, 0, 0),
            specs: Vec::new(),
            enabled: Vec::new(),
            failures: Vec::new(),
            catalogs: Vec::new(),
            plan: None,
            full_plan: None,
            excluded: HashSet::new(),
            closed: HashSet::new(),
            rows: Arc::from(Vec::new()),
            counts: (0, 0, 0),
            existing: None,
            update: false,
            changes: Arc::from(Vec::new()),
            skipped: HashSet::new(),
            unchanged: 0,
        }
    }

    /// The loaded specs.
    pub fn specs(&self) -> &[LoadedSpec] {
        &self.specs
    }

    /// The files of the source that could not be read.
    pub fn failures(&self) -> &[Failure] {
        &self.failures
    }

    /// The catalog files of the source.
    pub fn catalogs(&self) -> &[CatalogFile] {
        &self.catalogs
    }

    /// Whether nothing was loaded, not even a failure.
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
            && self.failures.is_empty()
            && self.catalogs.is_empty()
            && self.postman.is_empty()
            && self.bruno.is_none()
    }

    /// Whether the source gave something to import.
    pub fn has_content(&self) -> bool {
        !self.specs.is_empty() || !self.postman.is_empty() || self.bruno.is_some()
    }

    /// Whether the wizard imports for a collection tool, Postman or Bruno, whose requests come
    /// with issues to review, rather than OpenAPI specs.
    pub fn is_collection_tool(&self) -> bool {
        matches!(self.format, ImportFormat::Postman | ImportFormat::Bruno)
    }

    /// The Bruno collection loaded.
    pub fn bruno(&self) -> Option<&BrunoSource> {
        self.bruno.as_ref()
    }

    /// Whether the Bruno environment at `index` is imported.
    pub fn is_bruno_environment_enabled(&self, index: usize) -> bool {
        self.bruno_environments.get(index).copied().unwrap_or(false)
    }

    /// Flips the Bruno environment at `index`.
    pub fn toggle_bruno_environment(&mut self, index: usize) {
        if let Some(flag) = self.bruno_environments.get_mut(index) {
            *flag = !*flag;
        }
    }

    /// The colors to write to `.brasa/project.toml` after a Bruno import: each imported
    /// environment that has one, when the option is on.
    pub fn bruno_environment_colors(&self) -> Vec<(String, EnvColor)> {
        let Some(bruno) = &self.bruno else {
            return Vec::new();
        };
        if !self.bruno_colors {
            return Vec::new();
        }
        bruno
            .environments
            .iter()
            .enumerate()
            .filter(|(index, _)| self.is_bruno_environment_enabled(*index))
            .filter_map(|(_, environment)| Some((environment.name.clone(), environment.color?)))
            .collect()
    }

    /// The Postman exports loaded.
    pub fn postman_files(&self) -> &[PostmanFile] {
        &self.postman
    }

    /// Whether the Postman export at `index` is imported.
    pub fn is_postman_enabled(&self, index: usize) -> bool {
        self.postman_enabled.get(index).copied().unwrap_or(false)
    }

    /// Flips the Postman export at `index`.
    pub fn toggle_postman(&mut self, index: usize) {
        if let Some(flag) = self.postman_enabled.get_mut(index) {
            *flag = !*flag;
        }
    }

    /// Whether a Postman import has a collection to write.
    pub fn has_collection(&self) -> bool {
        if self.format == ImportFormat::Bruno {
            return self.bruno.as_ref().is_some_and(|bruno| bruno.requests > 0);
        }
        self.enabled_postman()
            .any(|file| file.format == PostmanFormat::Collection)
    }

    fn enabled_postman(&self) -> impl Iterator<Item = &PostmanFile> {
        self.postman
            .iter()
            .zip(&self.postman_enabled)
            .filter(|(_, enabled)| **enabled)
            .map(|(file, _)| file)
    }

    /// The manifest source of a Postman import: the collection's id, which identifies it wherever
    /// the file is. Several collections, or one without an id, cannot be told again later.
    pub fn postman_source(&self) -> ManifestSource {
        let mut collections = self
            .postman
            .iter()
            .filter(|file| file.format == PostmanFormat::Collection);
        let location = match (collections.next(), collections.next()) {
            (Some(only), None) => only.postman_id.clone(),
            _ => String::new(),
        };
        ManifestSource {
            kind: MANIFEST_KIND.to_string(),
            location,
            branch: String::new(),
        }
    }

    /// The manifest source of a Bruno import (#223): the collection's name, which identifies it
    /// wherever its folder or `.zip` is. Loose request files, with no `bruno.json` or
    /// `opencollection.yml`, name no collection and cannot be told again later.
    pub fn bruno_source(&self) -> ManifestSource {
        let location = self
            .bruno
            .as_ref()
            .filter(|bruno| bruno.has_root)
            .map(|bruno| bruno.name.clone())
            .unwrap_or_default();
        ManifestSource {
            kind: brasa_format::bruno::MANIFEST_KIND.to_string(),
            location,
            branch: String::new(),
        }
    }

    /// The most serious thing to tell about the request `key`.
    pub fn issue_of(&self, key: &str) -> Option<&RequestIssue> {
        self.issues
            .get(key)?
            .iter()
            .max_by_key(|issue| issue.level())
    }

    /// What to tell about each selected request of a Postman import, in tree order: the request
    /// name and its issues. Requests with nothing to tell are left out.
    pub fn selected_issues(&self) -> Vec<(String, Vec<RequestIssue>)> {
        fn walk(
            wizard: &Wizard,
            folder: &PlanFolder,
            path: &str,
            out: &mut Vec<(String, Vec<RequestIssue>)>,
        ) {
            for request in &folder.requests {
                if !wizard.is_selected(&request.key) {
                    continue;
                }
                if let Some(issues) = wizard.issues.get(&request.key) {
                    out.push((format!("{path}{}", request.name), issues.clone()));
                }
            }
            for child in &folder.folders {
                walk(wizard, child, &format!("{path}{}/", child.name), out);
            }
        }
        let mut out = Vec::new();
        if let Some(plan) = &self.plan {
            walk(self, &plan.root, "", &mut out);
        }
        out
    }

    /// How many selected requests are complete, have a warning and have something unsupported,
    /// for the counts of the Preview step. Kept with the rows, so render does not walk the tree.
    pub fn compat_counts(&self) -> (usize, usize, usize) {
        self.compat
    }

    fn count_compat(&self) -> (usize, usize, usize) {
        let (mut warning, mut unsupported) = (0, 0);
        for (_, issues) in self.selected_issues() {
            match issues.iter().map(RequestIssue::level).max() {
                Some(IssueLevel::Unsupported) => unsupported += 1,
                Some(IssueLevel::Warning) => warning += 1,
                _ => {}
            }
        }
        (
            self.selected_count() - warning - unsupported,
            warning,
            unsupported,
        )
    }

    /// Replaces what the source gave, which starts the selection over.
    pub fn set_loaded(&mut self, loaded: Loaded) {
        let format = loaded.format();
        self.set_specs(loaded.specs);
        self.failures = loaded.failures;
        self.catalogs = loaded.catalogs;
        if let Some(format) = format {
            self.format = format;
        }
        if self.format == ImportFormat::Bruno {
            self.bruno_environments = loaded
                .bruno
                .as_ref()
                .map_or_else(Vec::new, |bruno| vec![true; bruno.environments.len()]);
            self.bruno = loaded.bruno;
            return;
        }
        if self.format == ImportFormat::Postman {
            self.postman_enabled = vec![true; loaded.postman.len()];
            self.postman = loaded.postman;
        } else if !loaded.postman.is_empty() {
            for file in loaded.postman {
                self.failures.push(Failure {
                    source: file.source,
                    error: "a Postman export next to OpenAPI specs, import it on its own"
                        .to_string(),
                });
            }
        }
    }

    /// Switches the format, which starts over: the sources are not the same.
    pub fn set_format(&mut self, format: ImportFormat) {
        self.format = format;
        self.set_specs(Vec::new());
        if !sources_of(format).contains(&self.source) {
            self.source = SourceKind::File;
        }
    }

    /// Replaces the loaded specs, which starts the selection over.
    pub fn set_specs(&mut self, specs: Vec<LoadedSpec>) {
        self.enabled = vec![true; specs.len()];
        self.specs = specs;
        self.postman.clear();
        self.postman_enabled.clear();
        self.bruno = None;
        self.bruno_environments.clear();
        self.issues.clear();
        self.failures.clear();
        self.catalogs.clear();
        self.plan = None;
        self.full_plan = None;
        self.excluded.clear();
        self.closed.clear();
        self.rows = Arc::from(Vec::new());
        self.counts = (0, 0, 0);
        self.existing = None;
        self.update = false;
        self.changes = Arc::from(Vec::new());
        self.skipped.clear();
        self.unchanged = 0;
    }

    /// Records the collection an earlier import of this source created: the Preview step then
    /// offers to update it, with its options.
    pub fn set_existing(&mut self, existing: Option<OpenApiCollection>) {
        if let Some(collection) = &existing {
            self.options = collection.manifest.options.into();
        }
        self.update = existing.is_some();
        self.existing = existing;
        self.skipped.clear();
    }

    /// The collection an earlier import of this source created.
    pub fn existing(&self) -> Option<&OpenApiCollection> {
        self.existing.as_ref()
    }

    /// Whether the Preview step updates the existing collection.
    pub fn is_update(&self) -> bool {
        self.update && self.existing.is_some()
    }

    /// Chooses between updating the existing collection and importing a new one.
    pub fn set_update(&mut self, update: bool) {
        self.update = update;
    }

    /// The changes since the existing collection was imported, unchanged operations left out.
    pub fn changes(&self) -> Arc<[PlanChange]> {
        self.changes.clone()
    }

    /// `(added, changed, removed, unchanged)`.
    pub fn change_counts(&self) -> (usize, usize, usize, usize) {
        let count = |kind| {
            self.changes
                .iter()
                .filter(|change| change.kind == kind)
                .count()
        };
        (
            count(ChangeKind::Added),
            count(ChangeKind::Changed),
            count(ChangeKind::Removed),
            self.unchanged,
        )
    }

    /// Whether the change of `key` is applied.
    pub fn is_change_selected(&self, key: &str) -> bool {
        !self.skipped.contains(key)
    }

    /// Applies the change of `key`, or not.
    pub fn toggle_change(&mut self, key: &str) {
        if !self.skipped.remove(key) {
            self.skipped.insert(key.to_string());
        }
    }

    /// How many changes are applied.
    pub fn selected_change_count(&self) -> usize {
        self.changes
            .iter()
            .filter(|change| self.is_change_selected(&change.key))
            .count()
    }

    /// The name the collection gets unless the user types another one: the first spec's title.
    pub fn suggested_name(&self) -> String {
        if self.format == ImportFormat::Bruno {
            return self
                .bruno
                .as_ref()
                .map_or_else(|| "Bruno".to_string(), |bruno| bruno.name.clone());
        }
        if self.format == ImportFormat::Postman {
            let collections: Vec<&PostmanFile> = self
                .postman
                .iter()
                .filter(|file| file.format == PostmanFormat::Collection)
                .collect();
            return match collections.as_slice() {
                [only] => only.name.clone(),
                [] => self
                    .postman
                    .first()
                    .map_or_else(|| "Postman".to_string(), |file| file.name.clone()),
                _ => "Postman".to_string(),
            };
        }
        self.specs
            .first()
            .map(|loaded| loaded.spec.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "OpenAPI".to_string())
    }

    /// Whether the spec at `index` is imported.
    pub fn is_enabled(&self, index: usize) -> bool {
        self.enabled.get(index).copied().unwrap_or(false)
    }

    /// Imports the spec at `index`, or stops importing it. The plan has to be rebuilt.
    pub fn toggle_version(&mut self, index: usize) {
        if let Some(flag) = self.enabled.get_mut(index) {
            *flag = !*flag;
            self.full_plan = None;
        }
    }

    /// How many specs are imported.
    pub fn enabled_count(&self) -> usize {
        self.enabled.iter().filter(|enabled| **enabled).count()
    }

    /// The folder of each spec when several are imported together: `v<major>` of the API
    /// versions when they differ, else the titles when they differ, else the file names.
    pub fn version_labels(&self) -> Vec<String> {
        if self.specs.len() < 2 {
            return vec![String::new(); self.specs.len()];
        }
        let unique = |labels: &[String]| {
            let set: HashSet<&String> = labels.iter().collect();
            set.len() == labels.len() && labels.iter().all(|label| !label.is_empty())
        };
        let majors: Vec<String> = self
            .specs
            .iter()
            .map(|loaded| {
                let major = loaded
                    .spec
                    .api_version
                    .trim()
                    .trim_start_matches(['v', 'V']);
                let major = major.split('.').next().unwrap_or_default();
                if major.is_empty() {
                    String::new()
                } else {
                    format!("v{major}")
                }
            })
            .collect();
        if unique(&majors) {
            return majors;
        }
        let titles: Vec<String> = self
            .specs
            .iter()
            .map(|loaded| loaded.spec.title.clone())
            .collect();
        if unique(&titles) {
            return titles;
        }
        let mut names: Vec<String> = self
            .specs
            .iter()
            .map(|loaded| {
                let file = loaded.source.rsplit('/').next().unwrap_or_default();
                file.rsplit_once('.')
                    .map_or(file, |(stem, _)| stem)
                    .to_string()
            })
            .collect();
        if !unique(&names) {
            for (index, name) in names.iter_mut().enumerate() {
                *name = format!("{name} ({})", index + 1);
            }
        }
        names
    }

    /// Builds the plan of the loaded specs for a collection named `name`, with the current
    /// options, and the preview rows. Call it when the specs, the name or an option change,
    /// never on every render.
    pub fn rebuild(&mut self, name: &str) {
        if self.format == ImportFormat::Postman {
            self.rebuild_postman(name);
            return;
        }
        if self.format == ImportFormat::Bruno {
            self.rebuild_bruno(name);
            return;
        }
        let labels = self.version_labels();
        let versions: Vec<SpecVersion<'_>> = self
            .specs
            .iter()
            .zip(&labels)
            .zip(&self.enabled)
            .filter(|(_, enabled)| **enabled)
            .map(|((loaded, label), _)| SpecVersion {
                label,
                spec: &loaded.spec,
            })
            .collect();
        let name = if name.trim().is_empty() {
            self.suggested_name()
        } else {
            name.trim().to_string()
        };
        let plan = build_plan(&name, &versions, &self.options);
        if self.full_plan.is_none() {
            self.full_plan = Some(build_plan(&name, &versions, &ImportOptions::default()));
        }
        self.set_plan(plan);
    }

    /// [`Self::rebuild`] for a Postman import.
    fn rebuild_postman(&mut self, name: &str) {
        let options = self.postman_options;
        let mut collections: Vec<ImportPlan> = Vec::new();
        let mut environments: Vec<ImportedEnvironment> = Vec::new();
        let mut globals: Vec<ImportedEnvironment> = Vec::new();
        for file in self.enabled_postman() {
            // Each of them parsed when it was read, so these only fail on what cannot happen.
            match file.format {
                PostmanFormat::Collection => {
                    collections.extend(parse_collection(&file.text, &options).ok());
                }
                PostmanFormat::Environment => {
                    environments.extend(parse_environment(&file.text).ok());
                }
                PostmanFormat::Globals => globals.extend(parse_environment(&file.text).ok()),
            }
        }
        let name = if name.trim().is_empty() {
            self.suggested_name()
        } else {
            name.trim().to_string()
        };
        let built = build_postman_plan(&name, &collections, &environments, &globals, &options);
        self.issues = built.issues;
        self.set_plan(built.plan);
    }

    /// [`Self::rebuild`] for a Bruno import: the same plan as a Postman import's.
    fn rebuild_bruno(&mut self, name: &str) {
        let options = self.bruno_options;
        let Some(import) = self.bruno.as_ref().and_then(|bruno| bruno.parse(&options)) else {
            return;
        };
        let environments: Vec<ImportedEnvironment> = import
            .environments
            .into_iter()
            .enumerate()
            .filter(|(index, _)| self.is_bruno_environment_enabled(*index))
            .map(|(_, environment)| environment)
            .collect();
        let name = if name.trim().is_empty() {
            self.suggested_name()
        } else {
            name.trim().to_string()
        };
        let built = build_postman_plan(&name, &[import.plan], &environments, &[], &options);
        self.issues = built.issues;
        self.set_plan(built.plan);
    }

    /// Makes `plan` the current plan, with the changes since an earlier import and the rows.
    fn set_plan(&mut self, plan: OpenApiPlan) {
        if let Some(existing) = &self.existing {
            let first = self.changes.is_empty() && self.unchanged == 0;
            let mut changes = diff_plan(&existing.manifest, &plan);
            self.unchanged = changes
                .iter()
                .filter(|change| change.kind == ChangeKind::Unchanged)
                .count();
            changes.retain(|change| change.kind != ChangeKind::Unchanged);
            // The design's order: added, changed, removed, each in plan order.
            changes.sort_by_key(|change| match change.kind {
                ChangeKind::Added => 0,
                ChangeKind::Changed => 1,
                _ => 2,
            });
            if first {
                // Removed operations stay unless the user checks them.
                self.skipped.extend(
                    changes
                        .iter()
                        .filter(|change| change.kind == ChangeKind::Removed)
                        .map(|change| change.key.clone()),
                );
            }
            self.changes = Arc::from(changes);
        }
        self.plan = Some(Arc::new(plan));
        self.refresh_rows();
    }

    /// The current plan, once [`Self::rebuild`] ran.
    pub fn plan(&self) -> Option<&Arc<OpenApiPlan>> {
        self.plan.as_ref()
    }

    /// The preview rows.
    pub fn rows(&self) -> Arc<[PreviewRow]> {
        self.rows.clone()
    }

    /// Whether the request `key` is selected.
    pub fn is_selected(&self, key: &str) -> bool {
        !self.excluded.contains(key)
    }

    /// The requests the user unchecked.
    pub fn excluded(&self) -> &HashSet<String> {
        &self.excluded
    }

    /// How many requests the plan has.
    pub fn total_count(&self) -> usize {
        self.counts.1
    }

    /// How many of them are selected.
    pub fn selected_count(&self) -> usize {
        self.counts.0
    }

    /// How many folders the plan has.
    pub fn folder_count(&self) -> usize {
        self.counts.2
    }

    /// A path of the specs, to show what grouping by path segments gives.
    pub fn sample_path(&self) -> Option<&str> {
        let operations = self.specs.iter().flat_map(|loaded| &loaded.spec.operations);
        // The first path with two segments shows the nesting best.
        let mut first = None;
        for operation in operations {
            first = first.or(Some(operation.path.as_str()));
            if operation.path.matches('/').count() >= 2 {
                return Some(&operation.path);
            }
        }
        first
    }

    /// A tag of the specs, to show what grouping by tags gives.
    pub fn sample_tag(&self) -> Option<&str> {
        self.specs
            .iter()
            .flat_map(|loaded| &loaded.spec.operations)
            .find_map(|operation| operation.tags.first())
            .map(String::as_str)
    }

    /// How many operations the loaded specs have.
    pub fn operation_count(&self) -> usize {
        if self.format == ImportFormat::Bruno {
            return self.bruno.as_ref().map_or(0, |bruno| bruno.requests);
        }
        if self.format == ImportFormat::Postman {
            return self
                .enabled_postman()
                .filter(|file| file.format == PostmanFormat::Collection)
                .map(|file| file.items)
                .sum();
        }
        self.specs
            .iter()
            .zip(&self.enabled)
            .filter(|(_, enabled)| **enabled)
            .map(|(loaded, _)| loaded.spec.operations.len())
            .sum()
    }

    /// The `baseUrl` of each environment the "Environment from servers" option creates.
    pub fn base_urls(&self) -> Vec<String> {
        self.full_plan.as_ref().map_or_else(Vec::new, |plan| {
            plan.environments
                .iter()
                .filter_map(|environment| environment.base.first().map(|entry| entry.value.clone()))
                .collect()
        })
    }

    /// The variables the "Auth variables" option creates, stored in `.local.env`.
    pub fn auth_variables(&self) -> Vec<String> {
        self.full_plan.as_ref().map_or_else(Vec::new, |plan| {
            plan.environments
                .first()
                .map(|environment| {
                    environment
                        .secret
                        .iter()
                        .map(|entry| entry.key.clone())
                        .collect()
                })
                .unwrap_or_default()
        })
    }

    /// Flips the request `key`.
    pub fn toggle_request(&mut self, key: &str) {
        if !self.excluded.remove(key) {
            self.excluded.insert(key.to_string());
        }
        self.refresh_rows();
    }

    /// Selects every request under the folder at `path`, or unselects them all when they all
    /// are selected already.
    pub fn toggle_folder(&mut self, path: &str) {
        let Some(keys) = self
            .plan
            .as_ref()
            .and_then(|plan| folder_at(&plan.root, path))
            .map(PlanFolder::keys)
        else {
            return;
        };
        let all = keys.iter().all(|key| self.is_selected(key));
        for key in keys {
            if all {
                self.excluded.insert(key);
            } else {
                self.excluded.remove(&key);
            }
        }
        self.refresh_rows();
    }

    /// Selects every request, or none.
    pub fn select_all(&mut self, selected: bool) {
        self.excluded.clear();
        if !selected && let Some(plan) = &self.plan {
            self.excluded.extend(plan.root.keys());
        }
        self.refresh_rows();
    }

    /// Opens or collapses the folder at `path`.
    pub fn toggle_open(&mut self, path: &str) {
        if !self.closed.remove(path) {
            self.closed.insert(path.to_string());
        }
        self.refresh_rows();
    }

    fn refresh_rows(&mut self) {
        let mut rows = Vec::new();
        self.counts = (0, 0, 0);
        if let Some(plan) = &self.plan {
            self.push_rows(&plan.root, "", 0, &mut rows);
            let keys = plan.root.keys();
            let selected = keys.iter().filter(|key| self.is_selected(key)).count();
            self.counts = (selected, keys.len(), plan.root.folder_count());
        }
        self.rows = Arc::from(rows);
        self.compat = self.count_compat();
    }

    fn push_rows(&self, folder: &PlanFolder, path: &str, depth: usize, rows: &mut Vec<PreviewRow>) {
        for child in &folder.folders {
            let child_path = if path.is_empty() {
                child.name.clone()
            } else {
                format!("{path}{PATH_SEPARATOR}{}", child.name)
            };
            let keys = child.keys();
            let selected = keys.iter().filter(|key| self.is_selected(key)).count();
            let open = !self.closed.contains(&child_path);
            rows.push(PreviewRow {
                depth,
                id: child_path.clone(),
                name: child.name.clone(),
                kind: RowKind::Folder {
                    open,
                    check: if selected == keys.len() {
                        Check::All
                    } else if selected == 0 {
                        Check::None
                    } else {
                        Check::Some
                    },
                    selected,
                    total: keys.len(),
                },
            });
            if open {
                self.push_rows(child, &child_path, depth + 1, rows);
            }
        }
        for request in &folder.requests {
            rows.push(PreviewRow {
                depth,
                id: request.key.clone(),
                name: request.name.clone(),
                kind: RowKind::Request {
                    method: request.request.method.clone(),
                    checked: self.is_selected(&request.key),
                    path: if self.format == ImportFormat::Postman {
                        request.path.clone()
                    } else {
                        String::new()
                    },
                    issue: self.issue_of(&request.key).cloned(),
                },
            });
        }
    }
}

/// The folder at `path` (names joined with [`PATH_SEPARATOR`]) under `root`.
fn folder_at<'a>(root: &'a PlanFolder, path: &str) -> Option<&'a PlanFolder> {
    path.split(PATH_SEPARATOR).try_fold(root, |folder, name| {
        folder.folders.iter().find(|child| child.name == name)
    })
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use brasa_runner::SendOptions;
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
    get: {summary: List customers, tags: [Customers], responses: {}}
    post: {summary: Create customer, tags: [Customers], responses: {}}
  /customers/{id}:
    get: {summary: Get customer, tags: [Customers], responses: {}}
  /charges:
    get: {summary: List charges, responses: {}}
"##;

    fn wizard() -> Wizard {
        let mut wizard = Wizard::new(ImportFormat::OpenApi, SourceKind::Paste);
        wizard.set_specs(vec![load_text(SPEC).expect("valid")]);
        wizard.rebuild("");
        wizard
    }

    fn row_names(wizard: &Wizard) -> Vec<String> {
        wizard
            .rows()
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.name))
            .collect()
    }

    #[test]
    fn rows_follow_the_tree_and_the_open_folders() {
        let mut wizard = wizard();
        assert_eq!(wizard.suggested_name(), "Acme Payments API");
        assert_eq!(
            row_names(&wizard),
            vec![
                "customers",
                "  {id}",
                "    Get customer",
                "  List customers",
                "  Create customer",
                "charges",
                "  List charges"
            ]
        );
        wizard.toggle_open("customers");
        assert_eq!(
            row_names(&wizard),
            vec!["customers", "charges", "  List charges"]
        );
        assert_eq!(wizard.folder_count(), 3);
        assert_eq!(wizard.base_urls(), vec!["https://api.acme.dev".to_string()]);
        assert_eq!(wizard.auth_variables(), vec!["bearerAuth".to_string()]);
    }

    #[test]
    fn selection_counts_and_folder_checks() {
        let mut wizard = wizard();
        assert_eq!((wizard.selected_count(), wizard.total_count()), (4, 4));
        wizard.toggle_request("GET /customers/{id}");
        let customers = &wizard.rows()[0];
        assert_eq!(
            customers.kind,
            RowKind::Folder {
                open: true,
                check: Check::Some,
                selected: 2,
                total: 3
            }
        );
        // A partly selected folder selects everything first.
        wizard.toggle_folder("customers");
        assert_eq!(wizard.selected_count(), 4);
        wizard.toggle_folder("customers");
        assert_eq!(wizard.selected_count(), 1);
        wizard.select_all(false);
        assert_eq!(wizard.selected_count(), 0);
        wizard.select_all(true);
        assert_eq!(wizard.selected_count(), 4);
    }

    #[test]
    fn the_selection_survives_option_changes() {
        let mut wizard = wizard();
        wizard.toggle_request("GET /charges");
        wizard.options.grouping = brasa_format::openapi::Grouping::Tags;
        wizard.rebuild("My API");
        assert_eq!(wizard.plan().expect("built").root.name, "My API");
        assert_eq!(
            row_names(&wizard),
            vec![
                "Customers",
                "  List customers",
                "  Create customer",
                "  Get customer",
                "List charges"
            ]
        );
        assert!(!wizard.is_selected("GET /charges"));
        assert_eq!(wizard.selected_count(), 3);
    }

    #[test]
    fn samples_come_from_the_spec() {
        let wizard = wizard();
        assert_eq!(wizard.sample_path(), Some("/customers/{id}"));
        assert_eq!(wizard.sample_tag(), Some("Customers"));
    }

    #[test]
    fn loading_new_specs_starts_over() {
        let mut wizard = wizard();
        wizard.select_all(false);
        wizard.set_specs(vec![load_text(SPEC).expect("valid")]);
        wizard.rebuild("");
        assert_eq!(wizard.selected_count(), 4);
    }

    #[test]
    fn several_specs_get_version_folders() {
        let mut wizard = Wizard::new(ImportFormat::OpenApi, SourceKind::File);
        let v1 = load_text(&SPEC.replace("2.0.0", "1.4.0")).expect("valid");
        let mut v2 = load_text(SPEC).expect("valid");
        v2.source = "specs/payments-v2.yaml".to_string();
        wizard.set_specs(vec![v1.clone(), v2.clone()]);
        assert_eq!(wizard.version_labels(), vec!["v1", "v2"]);
        // Same major version, different titles: the titles.
        let mut other = v2.clone();
        other.spec.title = "Other API".into();
        wizard.set_specs(vec![v2.clone(), other.clone()]);
        assert_eq!(
            wizard.version_labels(),
            vec!["Acme Payments API", "Other API"]
        );
        // Same version and title: the file names.
        other.spec.title = v2.spec.title.clone();
        other.source = "other.yaml".into();
        wizard.set_specs(vec![v2.clone(), other]);
        assert_eq!(wizard.version_labels(), vec!["payments-v2", "other"]);
        // A disabled version leaves the plan.
        wizard.set_specs(vec![v1, v2]);
        wizard.toggle_version(0);
        wizard.rebuild("");
        assert_eq!(wizard.enabled_count(), 1);
        assert_eq!(wizard.operation_count(), 4);
        assert_eq!(wizard.total_count(), 4);
        assert_eq!(wizard.plan().expect("built").root.folders[0].name, "v2");
    }

    #[test]
    fn an_existing_import_turns_the_preview_into_an_update() {
        use brasa_format::openapi::{Manifest, ManifestSource};
        let mut wizard = wizard();
        let old = wizard.plan().expect("built").clone();
        let source = ManifestSource {
            kind: "url".into(),
            location: "https://x/spec.yaml".into(),
            branch: String::new(),
        };
        let mut files = std::collections::BTreeMap::new();
        for key in ["GET /customers", "GET /charges", "DELETE /gone"] {
            files.insert(key.to_string(), format!("{key}.brasa"));
        }
        let mut manifest = Manifest::new(
            "2026-10-28",
            source.clone(),
            &ImportOptions::default(),
            &old,
            &files,
        );
        manifest.requests.insert(
            "DELETE /gone".into(),
            brasa_format::openapi::ManifestRequest {
                method: "DELETE".into(),
                path: "/gone".into(),
                ..Default::default()
            },
        );
        let collection = OpenApiCollection {
            folder: "Acme".into(),
            manifest,
        };
        assert!(
            find_existing(
                vec![collection.clone()],
                &ManifestSource {
                    kind: "paste".into(),
                    ..source.clone()
                }
            )
            .is_none()
        );
        let found = find_existing(vec![collection], &source).expect("the same source");

        wizard.set_existing(Some(found));
        wizard.rebuild("");
        assert!(wizard.is_update());
        // POST /customers and GET /customers/{id} were not imported: added. DELETE /gone: removed.
        assert_eq!(wizard.change_counts(), (2, 0, 1, 2));
        let kinds: Vec<ChangeKind> = wizard.changes().iter().map(|change| change.kind).collect();
        assert_eq!(
            kinds,
            vec![ChangeKind::Added, ChangeKind::Added, ChangeKind::Removed]
        );
        assert_eq!(
            wizard.selected_change_count(),
            2,
            "removed ones start unchecked"
        );
        wizard.toggle_change("DELETE /gone");
        wizard.rebuild("");
        assert_eq!(
            wizard.selected_change_count(),
            3,
            "the choice survives a rebuild"
        );
        wizard.set_update(false);
        assert!(!wizard.is_update());
    }

    const POSTMAN_COLLECTION: &str = r#"{
      "info": {
        "_postman_id": "4f1c",
        "name": "Acme",
        "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json"
      },
      "variable": [{"key": "baseUrl", "value": "https://acme.dev"}],
      "item": [
        {"name": "Ping", "request": {"method": "GET", "url": "{{baseUrl}}/ping"}},
        {"name": "Auth", "item": [
          {"name": "Report", "request": {"method": "GET", "url": "{{baseUrl}}/r",
            "auth": {"type": "hawk"}}},
          {"name": "Login", "request": {"method": "POST", "url": "{{baseUrl}}/login"},
            "event": [{"listen": "test", "script": {"exec": ["pm.sendRequest('x', () => {});"]}}]},
          {"name": "Check", "request": {"method": "GET", "url": "{{baseUrl}}/c"},
            "event": [{"listen": "test", "script": {"exec": ["pm.response.to.have.status(200);"]}}]}
        ]}
      ]
    }"#;
    const POSTMAN_ENVIRONMENT: &str = r#"{"name": "Production",
      "_postman_variable_scope": "environment",
      "values": [{"key": "baseUrl", "value": "https://prod", "enabled": true}]}"#;

    fn postman_wizard() -> Wizard {
        let mut wizard = Wizard::new(ImportFormat::Postman, SourceKind::File);
        let mut loaded = load_pasted(POSTMAN_COLLECTION).expect("a collection");
        loaded.merge(load_pasted(POSTMAN_ENVIRONMENT).expect("an environment"));
        wizard.set_loaded(loaded);
        wizard
    }

    #[test]
    fn a_postman_export_is_detected_and_listed() {
        let mut wizard = Wizard::new(ImportFormat::OpenApi, SourceKind::Paste);
        wizard.set_loaded(load_pasted(POSTMAN_COLLECTION).expect("a collection"));
        assert_eq!(wizard.format, ImportFormat::Postman);
        let [file] = wizard.postman_files() else {
            panic!("one file");
        };
        assert_eq!(file.name, "Acme");
        assert_eq!(file.version, "v2.1");
        assert_eq!((file.items, file.others), (4, 1));
        assert_eq!(wizard.suggested_name(), "Acme");
        assert_eq!(wizard.postman_source().location, "4f1c");
    }

    #[test]
    fn a_postman_plan_has_the_selection_and_the_compatibility() {
        let mut wizard = postman_wizard();
        wizard.rebuild("Acme");
        assert_eq!(wizard.total_count(), 4);
        assert_eq!(wizard.compat_counts(), (2, 1, 1));
        wizard.toggle_request("Auth/GET Report");
        assert_eq!(wizard.compat_counts(), (2, 1, 0));
        let plan = wizard.plan().expect("a plan");
        assert_eq!(plan.environments[0].name, "Production");
    }

    #[test]
    fn unchecking_the_environment_falls_back_to_the_collection_variables() {
        let mut wizard = postman_wizard();
        wizard.toggle_postman(1);
        wizard.rebuild("Acme");
        let plan = wizard.plan().expect("a plan");
        assert_eq!(plan.environments[0].name, "Acme");
    }

    #[test]
    fn switching_the_format_starts_over() {
        let mut wizard = postman_wizard();
        wizard.source = SourceKind::Paste;
        wizard.set_format(ImportFormat::OpenApi);
        assert!(!wizard.has_content());
        wizard.source = SourceKind::Git;
        wizard.set_format(ImportFormat::Postman);
        assert_eq!(wizard.source, SourceKind::File);
    }

    #[test]
    fn a_zip_export_gives_its_postman_files() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("export.zip");
        let file = std::fs::File::create(&path).expect("create");
        let mut archive = zip::ZipWriter::new(file);
        for (name, text) in [
            ("dump/a.postman_collection.json", POSTMAN_COLLECTION),
            ("dump/b.postman_environment.json", POSTMAN_ENVIRONMENT),
            ("dump/other.json", "{}"),
            ("dump/notes.txt", POSTMAN_COLLECTION),
        ] {
            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("entry");
            archive.write_all(text.as_bytes()).expect("write");
        }
        archive.finish().expect("finish");

        let loaded = load_files(&[path], &Client::new(SendOptions::default())).expect("a zip");
        let names: Vec<&str> = loaded
            .postman
            .iter()
            .map(|file| file.name.as_str())
            .collect();
        assert_eq!(names, vec!["Acme", "Production"]);
        assert_eq!(loaded.format(), Some(ImportFormat::Postman));
    }

    #[test]
    fn a_folder_scan_finds_postman_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("a.json"), POSTMAN_COLLECTION).expect("write");
        std::fs::write(dir.path().join("readme.json"), "{}").expect("write");
        let loaded =
            scan_folder(dir.path(), &Client::new(SendOptions::default())).expect("a folder");
        assert_eq!(loaded.postman.len(), 1);
    }

    #[test]
    fn the_report_lists_what_needs_a_look_and_nothing_else() {
        let report = report_text(
            "Acme",
            &[
                ("Check".to_string(), vec![RequestIssue::ScriptConverted]),
                ("Auth/Report".to_string(), vec![RequestIssue::FormDataFile]),
            ],
            &["a warning".to_string()],
        );
        assert!(report.contains("`Auth/Report`"), "{report}");
        assert!(!report.contains("Check"), "{report}");
        assert!(report.contains("- a warning"), "{report}");
        assert_eq!(
            report_text(
                "Acme",
                &[("C".to_string(), vec![RequestIssue::ScriptConverted])],
                &[]
            ),
            ""
        );
    }

    #[test]
    fn invalid_text_is_an_error() {
        assert!(load_text("not: a spec").is_err());
        assert!(load_url("ftp://x", &Client::new(Default::default())).is_err());
    }

    #[test]
    fn relative_references_join_urls() {
        let base = "https://api.acme.dev/specs/v2/openapi.yaml?ref=main";
        assert_eq!(
            join_url(base, "schemas/pet.yaml"),
            "https://api.acme.dev/specs/v2/schemas/pet.yaml"
        );
        assert_eq!(
            join_url(base, "../common.yaml"),
            "https://api.acme.dev/specs/common.yaml"
        );
        assert_eq!(
            join_url(base, "/root.yaml"),
            "https://api.acme.dev/root.yaml"
        );
        assert_eq!(
            join_url(base, "//cdn.test/x.yaml"),
            "https://cdn.test/x.yaml"
        );
        assert_eq!(join_url(base, "http://other.test/y"), "http://other.test/y");
    }

    #[test]
    fn files_load_with_their_relative_references() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("api.yaml"),
            "openapi: 3.0.0\ninfo: {title: T, version: '1'}\npaths:\n  /x:\n    $ref: 'paths/x.yaml'\n",
        )
        .expect("write");
        std::fs::create_dir(dir.path().join("paths")).expect("mkdir");
        std::fs::write(
            dir.path().join("paths/x.yaml"),
            "get: {summary: Get x, responses: {}}\n",
        )
        .expect("write");
        let loaded = load_path(
            &dir.path().join("api.yaml"),
            "api.yaml",
            &Client::new(Default::default()),
        )
        .expect("loads");
        let loaded = &loaded.specs[0];
        assert_eq!(loaded.source, "api.yaml");
        assert_eq!(loaded.spec.operations[0].summary, "Get x");
        assert!(
            loaded.spec.warnings.is_empty(),
            "{:?}",
            loaded.spec.warnings
        );
    }

    fn write(dir: &Path, path: &str, text: &str) {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, text).expect("write");
    }

    fn spec_text(title: &str, version: &str) -> String {
        format!(
            "openapi: 3.0.0\ninfo: {{title: {title}, version: '{version}'}}\npaths:\n  /x:\n    get: {{responses: {{}}}}\n"
        )
    }

    /// A repository like the design's: a catalog pointing to two specs, a spec it does not
    /// list, a fragment, a broken spec and folders a scan skips.
    fn sample_tree(dir: &Path) {
        write(
            dir,
            "catalog-info.yaml",
            "apiVersion: backstage.io/v1alpha1\nkind: API\nmetadata: {name: v1}\nspec: {type: openapi, definition: {$text: ./specs/payments-v1.yaml}}\n---\napiVersion: backstage.io/v1alpha1\nkind: API\nmetadata: {name: v2}\nspec: {type: openapi, definition: {$text: ./specs/payments-v2.yaml}}\n",
        );
        write(
            dir,
            "specs/payments-v1.yaml",
            &spec_text("Payments", "1.0.0"),
        );
        write(
            dir,
            "specs/payments-v2.yaml",
            &spec_text("Payments", "2.0.0"),
        );
        write(
            dir,
            "other/users.json",
            "{\"openapi\": \"3.1.0\", \"info\": {\"title\": \"Users\", \"version\": \"1\"}, \"paths\": {}}",
        );
        write(dir, "specs/schemas/pet.yaml", "type: object\n");
        write(dir, "broken.yaml", "openapi: 4.0.0\n");
        write(
            dir,
            "node_modules/x/openapi.yaml",
            &spec_text("Dependency", "1"),
        );
        write(dir, ".hidden/openapi.yaml", &spec_text("Hidden", "1"));
    }

    fn sources(loaded: &Loaded) -> Vec<&str> {
        loaded
            .specs
            .iter()
            .map(|spec| spec.source.as_str())
            .collect()
    }

    #[test]
    fn catalogs_read_the_specs_they_point_to() {
        let dir = tempfile::tempdir().expect("tempdir");
        sample_tree(dir.path());
        let loaded = load_path(
            &dir.path().join("catalog-info.yaml"),
            "catalog-info.yaml",
            &Client::new(Default::default()),
        )
        .expect("a catalog");
        assert_eq!(
            sources(&loaded),
            vec!["specs/payments-v1.yaml", "specs/payments-v2.yaml"]
        );
        assert_eq!(
            loaded.catalogs,
            vec![CatalogFile {
                source: "catalog-info.yaml".into(),
                apis: 2
            }]
        );
    }

    #[test]
    fn folder_scans_find_every_spec_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        sample_tree(dir.path());
        let loaded = scan_folder(dir.path(), &Client::new(Default::default())).expect("scans");
        assert_eq!(
            sources(&loaded),
            vec![
                "specs/payments-v1.yaml",
                "specs/payments-v2.yaml",
                "other/users.json"
            ]
        );
        assert_eq!(loaded.failures.len(), 1);
        assert_eq!(loaded.failures[0].source, "broken.yaml");
        assert_eq!(loaded.catalogs.len(), 1);
    }

    #[test]
    fn repositories_are_cloned_and_scanned() {
        let Ok(git) = telaio_git::detect(&telaio_git::Executable::Auto) else {
            eprintln!("skipping, no git");
            return;
        };
        let repo = tempfile::tempdir().expect("tempdir");
        sample_tree(repo.path());
        let run = |args: &[&str]| {
            let output = std::process::Command::new(git.path())
                .args(args)
                .current_dir(repo.path())
                .output()
                .expect("git runs");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        run(&["init", "--quiet", "--initial-branch=main"]);
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "specs",
        ]);
        let url = format!("file://{}", repo.path().display());
        let client = Client::new(Default::default());
        let (checkout, loaded) = clone_repository(&git, &url, "", &client).expect("clones");
        assert_eq!(loaded.specs.len(), 3);
        let path = checkout.path().to_path_buf();
        assert!(path.exists());
        drop(checkout);
        assert!(!path.exists(), "the checkout is deleted with its guard");
        assert!(clone_repository(&git, &url, "missing", &client).is_err());
    }

    /// A small Bruno collection on disk: `bruno.json`, a request in a folder and two
    /// environments, one with a color and a secret.
    fn write_bruno_collection(root: &Path) {
        let users = root.join("users");
        let environments = root.join("environments");
        std::fs::create_dir_all(&users).expect("users");
        std::fs::create_dir_all(&environments).expect("environments");
        for (path, text) in [
            (root.join("bruno.json"), r#"{"name": "Shop"}"#),
            (
                users.join("Get user.bru"),
                "meta {\n  name: Get user\n  seq: 1\n}\n\nget {\n  url: {{host}}/users/1\n}\n\nassert {\n  $res.status: eq 200\n}\n",
            ),
            (
                environments.join("Local.bru"),
                "vars {\n  host: http://localhost\n}\nvars:secret [\n  token\n]\ncolor: #4CAF50\n",
            ),
            (
                environments.join("Prod.bru"),
                "vars {\n  host: https://shop.test\n}\n",
            ),
        ] {
            std::fs::write(path, text).expect("write");
        }
    }

    #[test]
    fn a_bruno_folder_reads_as_a_bruno_collection() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_bruno_collection(dir.path());
        let client = Client::new(SendOptions::default());
        let loaded = scan_folder(dir.path(), &client).expect("a folder");
        assert_eq!(loaded.format(), Some(ImportFormat::Bruno));
        let bruno = loaded.bruno.as_ref().expect("bruno");
        assert_eq!(bruno.name, "Shop");
        assert_eq!(bruno.source, "bruno.json");
        assert_eq!(bruno.formats, ".bru");
        assert_eq!((bruno.requests, bruno.folders), (1, 1));
        assert_eq!(
            bruno.environments,
            vec![
                BrunoEnvironment {
                    name: "Local".to_string(),
                    variables: 2,
                    secrets: 1,
                    color: Some(EnvColor::Green),
                },
                BrunoEnvironment {
                    name: "Prod".to_string(),
                    variables: 1,
                    secrets: 0,
                    color: None,
                },
            ]
        );

        // The root file picked on its own stands for the folder, and so does a folder dropped
        // on the window.
        let picked = load_files(&[dir.path().join("bruno.json")], &client).expect("a file");
        assert_eq!(picked.bruno.as_ref().map(|bruno| bruno.requests), Some(1));
        let dropped = load_files(&[dir.path().to_path_buf()], &client).expect("a folder");
        assert_eq!(dropped.format(), Some(ImportFormat::Bruno));
    }

    #[test]
    fn specs_win_over_a_bruno_collection_further_down() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("openapi.yaml"), SPEC).expect("spec");
        write_bruno_collection(&dir.path().join("tools/bruno"));
        let client = Client::new(SendOptions::default());
        let loaded = scan_folder(dir.path(), &client).expect("a folder");
        assert_eq!(loaded.format(), Some(ImportFormat::OpenApi));
        assert!(loaded.bruno.is_none());

        let only_bruno = tempfile::tempdir().expect("tempdir");
        write_bruno_collection(&only_bruno.path().join("api/bruno"));
        let loaded = scan_folder(only_bruno.path(), &client).expect("a folder");
        assert_eq!(loaded.format(), Some(ImportFormat::Bruno));
    }

    #[test]
    fn a_bruno_zip_and_a_pasted_request() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("shop.zip");
        let mut archive = zip::ZipWriter::new(std::fs::File::create(&path).expect("create"));
        for (name, text) in [
            ("shop/bruno.json", r#"{"name": "Shop"}"#),
            ("shop/ping.bru", "get {\n  url: /ping\n}\n"),
            ("shop/notes.txt", "x"),
        ] {
            archive
                .start_file(name, zip::write::SimpleFileOptions::default())
                .expect("entry");
            archive.write_all(text.as_bytes()).expect("write");
        }
        archive.finish().expect("finish");
        let loaded = load_files(&[path], &Client::new(SendOptions::default())).expect("a zip");
        assert_eq!(loaded.format(), Some(ImportFormat::Bruno));
        assert_eq!(loaded.bruno.as_ref().map(|bruno| bruno.requests), Some(1));

        let pasted =
            load_pasted("meta {\n  name: Ping\n}\n\nget {\n  url: /ping\n}\n").expect("a request");
        let bruno = pasted.bruno.expect("bruno");
        assert_eq!((bruno.requests, bruno.has_root), (1, false));
        assert_eq!(pasted.specs.len(), 0);
    }

    #[test]
    fn a_bruno_plan_with_environments_and_colors() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_bruno_collection(dir.path());
        let loaded =
            scan_folder(dir.path(), &Client::new(SendOptions::default())).expect("a folder");
        let mut wizard = Wizard::new(ImportFormat::OpenApi, SourceKind::Folder);
        wizard.set_loaded(loaded);
        assert_eq!(wizard.format, ImportFormat::Bruno);
        assert!(wizard.has_collection());
        assert!(wizard.is_collection_tool());
        assert_eq!(wizard.suggested_name(), "Shop");
        assert!(wizard.bruno_options.docs);

        wizard.rebuild("");
        let plan = wizard.plan().expect("plan");
        let names: Vec<&str> = plan
            .environments
            .iter()
            .map(|environment| environment.name.as_str())
            .collect();
        assert_eq!(names, vec!["Local", "Prod"]);
        assert_eq!(wizard.total_count(), 1);
        assert_eq!(
            wizard.bruno_environment_colors(),
            vec![("Local".to_string(), EnvColor::Green)]
        );

        wizard.toggle_bruno_environment(0);
        wizard.rebuild("");
        assert_eq!(wizard.plan().expect("plan").environments.len(), 1);
        assert!(wizard.bruno_environment_colors().is_empty());
        wizard.toggle_bruno_environment(0);
        wizard.bruno_colors = false;
        assert!(wizard.bruno_environment_colors().is_empty());
    }

    #[test]
    fn a_bruno_collection_imported_before_is_found_and_diffed() {
        let source_dir = tempfile::tempdir().expect("tempdir");
        write_bruno_collection(source_dir.path());
        let client = Client::new(SendOptions::default());
        let workspace_dir = tempfile::tempdir().expect("tempdir");
        let mut workspace =
            brasa_workspace::Workspace::open(workspace_dir.path()).expect("workspace");

        // The first import.
        let mut first = Wizard::new(ImportFormat::Bruno, SourceKind::Folder);
        first.set_loaded(scan_folder(source_dir.path(), &client).expect("a folder"));
        let source = first.bruno_source();
        assert_eq!(
            (source.kind.as_str(), source.location.as_str()),
            ("bruno", "Shop")
        );
        first.rebuild("");
        let origin = brasa_workspace::ImportOrigin {
            source: source.clone(),
            date: "2026-11-07".to_string(),
            options: first.options,
        };
        let plan = first.plan().expect("plan").clone();
        workspace
            .import_openapi(&plan, &|_| true, &origin)
            .expect("import");

        // The collection gets a request, and is read again from a .zip elsewhere: still found.
        std::fs::write(
            source_dir.path().join("users/List users.bru"),
            "meta {\n  name: List users\n  seq: 2\n}\n\nget {\n  url: {{host}}/users\n}\n",
        )
        .expect("write");
        let mut again = Wizard::new(ImportFormat::Bruno, SourceKind::Folder);
        again.set_loaded(scan_folder(source_dir.path(), &client).expect("a folder"));
        let found = find_existing(workspace.openapi_collections(), &again.bruno_source())
            .expect("the same collection");
        again.set_existing(Some(found));
        again.rebuild("");
        assert!(again.is_update());
        let (added, changed, removed, unchanged) = again.change_counts();
        assert_eq!((added, changed, removed, unchanged), (1, 0, 0, 1));

        // Loose request files name no collection: never an update.
        let loose = load_pasted("get {\n  url: /ping\n}\n").expect("a request");
        let mut wizard = Wizard::new(ImportFormat::Bruno, SourceKind::Paste);
        wizard.set_loaded(loose);
        assert!(find_existing(workspace.openapi_collections(), &wizard.bruno_source()).is_none());
    }
}
