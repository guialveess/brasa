//! Importing Bruno collections, in either of Bruno's formats: `.bru` files (`bruno.json` at the
//! root) or OpenCollection YAML (`opencollection.yml` at the root, Bruno 3.0 and later). Both may
//! be mixed in one collection. See `docs/bruno-import.md`.
//!
//! Pure, like the rest of `brasa-format`: the caller reads the collection's files and passes
//! their paths and text. The result is the [`ImportPlan`] and [`ImportedEnvironment`]s a Postman
//! import produces, so `build_postman_plan`, the wizard and `brasa-workspace` are shared.

mod bru;
mod convert;
mod model;
mod script;
mod yaml;

use std::collections::BTreeMap;

use brasa_core::KeyValue;

use crate::postman::{ImportPlan, ImportedEnvironment, PostmanOptions};
use bru::parse_bru;
use convert::{Inherited, Mapping, OAUTH2_TOKEN_VARIABLE, map_folder, take_defaults};
use model::{Defaults, RawEnvironment, RawFolder};
use yaml::parse_yaml;

/// The file at the root of a `.bru` collection.
pub const BRUNO_JSON: &str = "bruno.json";
/// The file at the root of an OpenCollection YAML collection.
pub const OPENCOLLECTION_YML: &str = "opencollection.yml";

/// The `kind` of a manifest source written by a Bruno import (#223). Its `location` is the
/// collection's name, which identifies it wherever its folder or `.zip` is.
pub const MANIFEST_KIND: &str = "bruno";

/// The folder of a collection's environments.
const ENVIRONMENTS: &str = "environments";

/// One file of a Bruno collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrunoFile {
    /// Its path, relative to wherever the caller read the collection from, with `/` separators.
    pub path: String,
    /// Its text.
    pub text: String,
}

/// What a Bruno collection gives.
#[derive(Debug, Clone, PartialEq)]
pub struct BrunoImport {
    /// The requests and folders, the collection's variables and what to tell about them.
    pub plan: ImportPlan,
    /// The environments.
    pub environments: Vec<ImportedEnvironment>,
    /// The color of each environment that has one, as Bruno writes it (`#4caf50`).
    pub environment_colors: Vec<(String, String)>,
}

/// Why a set of files is not a Bruno collection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrunoError {
    /// No `bruno.json`, no `opencollection.yml` and no request.
    #[error("no Bruno collection: no bruno.json, opencollection.yml or request file")]
    NotACollection,
}

/// Whether `path` (with `/` separators) is the root file of a Bruno collection.
pub fn is_collection_root(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == BRUNO_JSON || name == OPENCOLLECTION_YML
}

/// Whether `text`, read from a file named `name`, is a Bruno request or collection file: a `.bru`
/// file with a method or meta block, or a YAML file that looks like OpenCollection.
pub fn is_bruno_file(name: &str, text: &str) -> bool {
    if is_collection_root(name) {
        return true;
    }
    if name.ends_with(".bru") {
        let file = parse_bru(text);
        return file.blocks.iter().any(|block| {
            matches!(
                block.name.as_str(),
                "meta"
                    | "vars"
                    | "get"
                    | "post"
                    | "put"
                    | "delete"
                    | "patch"
                    | "options"
                    | "head"
                    | "connect"
                    | "trace"
                    | "http"
            )
        });
    }
    if name.ends_with(".yml") || name.ends_with(".yaml") {
        return parse_yaml(text).is_some_and(|value| {
            value.get("opencollection").is_some()
                || value.get("info").is_some() && value.get("http").is_some()
        });
    }
    false
}

/// The directory part of `path`, empty at the root.
fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..at])
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Whether `path` is inside a folder Bruno and Brasa both ignore: `node_modules` or a hidden
/// one such as `.git`.
fn ignored(path: &str) -> bool {
    dir_of(path)
        .split('/')
        .any(|part| part == "node_modules" || (part.starts_with('.') && part.len() > 1))
}

/// The stem of a request or environment file name.
fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// Parses a Bruno collection from its `files`. `fallback_name` names the collection when
/// `bruno.json` or `opencollection.yml` does not.
pub fn parse_bruno_collection(
    files: &[BrunoFile],
    fallback_name: &str,
    options: &PostmanOptions,
) -> Result<BrunoImport, BrunoError> {
    let files: Vec<BrunoFile> = files
        .iter()
        .map(|file| BrunoFile {
            path: file.path.replace('\\', "/"),
            text: file.text.clone(),
        })
        .collect();
    // The root is the shallowest folder with a root file.
    let root = files
        .iter()
        .filter(|file| is_collection_root(&file.path))
        .map(|file| dir_of(&file.path).to_string())
        .min_by_key(|dir| (dir.matches('/').count(), dir.len()));
    let root_prefix = match &root {
        Some(dir) if dir.is_empty() => String::new(),
        Some(dir) => format!("{dir}/"),
        None => String::new(),
    };
    let relative: Vec<(String, &str)> = files
        .iter()
        .filter_map(|file| {
            let path = file.path.strip_prefix(&root_prefix)?;
            (!ignored(path)).then(|| (path.to_string(), file.text.as_str()))
        })
        .collect();
    let text_of = |path: &str| {
        relative
            .iter()
            .find(|(known, _)| known == path)
            .map(|(_, text)| *text)
    };

    let mut warnings = Vec::new();
    let mut collection_name = None;
    let mut defaults = Defaults::default();
    if let Some(text) = text_of(OPENCOLLECTION_YML)
        && let Some(value) = parse_yaml(text)
    {
        collection_name = yaml::yaml_info(&value).0;
        defaults = yaml::yaml_defaults(&value);
    }
    if let Some(text) = text_of(BRUNO_JSON)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(text)
    {
        collection_name = collection_name.or_else(|| {
            value
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    }
    if let Some(text) = text_of("collection.bru") {
        defaults = model::bru_defaults(&parse_bru(text));
    }
    let collection_name = collection_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| fallback_name.to_string());

    // Folders by path, created on demand, then nested.
    let mut folders: BTreeMap<String, RawFolder> = BTreeMap::new();
    let mut environments = Vec::new();
    let mut found_request = false;
    for (path, text) in &relative {
        let name = file_name(path);
        let dir = dir_of(path);
        let is_bru = name.ends_with(".bru");
        let is_yaml = name.ends_with(".yml") || name.ends_with(".yaml");
        if !is_bru && !is_yaml {
            if name == ".env" {
                warnings.push(
                    "the collection's .env file is not imported: set its values in an environment"
                        .to_string(),
                );
            }
            continue;
        }
        if dir == ENVIRONMENTS {
            let environment = if is_bru {
                model::bru_environment(&parse_bru(text), stem(name))
            } else {
                match parse_yaml(text) {
                    Some(value) => yaml::yaml_environment(&value, stem(name)),
                    None => continue,
                }
            };
            environments.push(environment);
            continue;
        }
        if dir.is_empty() && matches!(name, "collection.bru" | OPENCOLLECTION_YML) {
            continue;
        }
        let folder = folders.entry(dir.to_string()).or_default();
        if matches!(name, "folder.bru" | "folder.yml") {
            let (folder_name, seq) = if is_bru {
                let file = parse_bru(text);
                folder.defaults = model::bru_defaults(&file);
                model::bru_folder_meta(&file)
            } else {
                let Some(value) = parse_yaml(text) else {
                    continue;
                };
                folder.defaults = yaml::yaml_defaults(&value);
                yaml::yaml_info(&value)
            };
            if let Some(folder_name) = folder_name {
                folder.name = folder_name;
            }
            folder.seq = seq;
            continue;
        }
        let request = if is_bru {
            let file = parse_bru(text);
            if !file.blocks.iter().any(|block| {
                matches!(
                    block.name.as_str(),
                    "get"
                        | "post"
                        | "put"
                        | "delete"
                        | "patch"
                        | "options"
                        | "head"
                        | "connect"
                        | "trace"
                        | "http"
                        | "grpc"
                        | "ws"
                ) || block.name == "meta"
            }) {
                continue;
            }
            model::bru_request(&file, stem(name))
        } else {
            match parse_yaml(text) {
                Some(value) if value.get("info").is_some() || value.get("http").is_some() => {
                    yaml::yaml_request(&value, stem(name))
                }
                _ => continue,
            }
        };
        found_request = true;
        folder.requests.push(request);
    }
    if root.is_none() && !found_request && environments.is_empty() {
        return Err(BrunoError::NotACollection);
    }
    if files.iter().any(|file| file.text.contains("process.env.")) {
        warnings.push(
            "values read from process.env are not imported: set them in an environment".to_string(),
        );
    }

    let mut root_folder = nest(folders);
    root_folder.name = collection_name.clone();
    root_folder.defaults = defaults;

    let mut mapping = Mapping {
        options,
        warnings,
        variables: Vec::new(),
        oauth2: false,
    };
    take_defaults(&root_folder.defaults, "collection", &mut mapping);
    let root_import = map_folder(&root_folder, &Inherited::default(), "", &mut mapping);
    let mut secret_variables = Vec::new();
    if mapping.oauth2 {
        secret_variables.push(KeyValue::new(OAUTH2_TOKEN_VARIABLE, ""));
    }
    let environment_colors = environments
        .iter()
        .filter_map(|environment| {
            let color = environment.color.as_ref()?;
            Some((environment.name.clone(), color.clone()))
        })
        .collect();
    let environments = resolve_environments(&environments, options);

    Ok(BrunoImport {
        plan: ImportPlan {
            collection_name,
            postman_id: String::new(),
            schema_version: String::new(),
            root: root_import,
            variables: if options.variables {
                mapping.variables
            } else {
                Vec::new()
            },
            secret_variables,
            warnings: mapping.warnings,
        },
        environments,
        environment_colors,
    })
}

/// Nests folders keyed by path into the tree under the root (`""`). A folder without a
/// `folder.bru` is named after its directory.
fn nest(mut folders: BTreeMap<String, RawFolder>) -> RawFolder {
    // Every ancestor of a folder exists, so a request three levels down is not orphaned.
    let paths: Vec<String> = folders.keys().cloned().collect();
    for path in paths {
        let mut dir = path.as_str();
        while !dir.is_empty() {
            dir = dir_of(dir);
            folders.entry(dir.to_string()).or_default();
        }
    }
    folders.entry(String::new()).or_default();
    // Deepest first, so children are complete before moving into their parent.
    let mut paths: Vec<String> = folders.keys().cloned().collect();
    paths.sort_by_key(|path| {
        std::cmp::Reverse(path.matches('/').count() + usize::from(!path.is_empty()))
    });
    for path in paths {
        if path.is_empty() {
            continue;
        }
        let Some(mut folder) = folders.remove(&path) else {
            continue;
        };
        if folder.name.is_empty() {
            // No `folder.bru` or `folder.yml`: named after its directory, after the others.
            folder.name = file_name(&path).to_string();
            folder.seq = f64::MAX;
        }
        folders
            .entry(dir_of(&path).to_string())
            .or_default()
            .folders
            .push(folder);
    }
    folders.remove("").unwrap_or_default()
}

/// The environments as Brasa writes them: `extends` resolved (the parent's variables the child
/// does not set), disabled variables left out, secret ones as empty entries for `.local.env`.
fn resolve_environments(
    environments: &[RawEnvironment],
    options: &PostmanOptions,
) -> Vec<ImportedEnvironment> {
    let by_name = |name: &str| {
        environments
            .iter()
            .find(|environment| environment.name == name)
    };
    environments
        .iter()
        .map(|environment| {
            let mut base: Vec<KeyValue> = Vec::new();
            let mut secrets: Vec<String> = Vec::new();
            let mut chain = vec![environment];
            while let Some(parent) = chain
                .last()
                .and_then(|last| last.extends.as_deref())
                .and_then(by_name)
            {
                if chain.iter().any(|known| known.name == parent.name) || chain.len() > 8 {
                    break;
                }
                chain.push(parent);
            }
            // The farthest ancestor first, so a nearer value replaces it.
            for level in chain.iter().rev() {
                for variable in level.vars.iter().filter(|variable| variable.enabled) {
                    base.retain(|known| known.key != variable.key);
                    base.push(KeyValue::new(variable.key.clone(), variable.value.clone()));
                }
                for secret in &level.secrets {
                    if !secrets.contains(secret) {
                        secrets.push(secret.clone());
                    }
                }
            }
            base.retain(|variable| !secrets.contains(&variable.key));
            let mut warnings = Vec::new();
            if !secrets.is_empty() {
                warnings.push(format!(
                    "environment \"{}\": the values of {} are not in the collection, Bruno keeps secrets on each machine; set them in its .local.env",
                    environment.name,
                    secrets.join(", ")
                ));
            }
            ImportedEnvironment {
                name: environment.name.clone(),
                base,
                secret: if options.secrets {
                    secrets
                        .into_iter()
                        .map(|name| KeyValue::new(name, ""))
                        .collect()
                } else {
                    Vec::new()
                },
                warnings,
            }
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests;
