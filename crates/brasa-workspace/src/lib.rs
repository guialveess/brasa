//! Filesystem access to a Brasa workspace: scanning a folder into a collection tree, and
//! loading and saving requests and environments.
//!
//! A workspace is a plain folder on disk: subfolders are collections, `.brasa` files are
//! requests, and a top-level `environments/` folder holds `.env`/`.local.env` files instead of
//! being shown as a collection. This crate only deals with the filesystem: [`brasa_format`] does
//! the actual parsing and serialization of file content, this crate is responsible for finding the
//! files, turning them into a tree of [`Node`]s, and writing them back safely (atomic writes, every
//! id confined to the workspace root).
#![warn(missing_docs)]

mod create;
mod env_edit;
mod error;
mod export;
mod git;
mod ids;
mod openapi;
mod postman;
mod project;
mod sanitize;
mod scan;
mod tree;
mod workspace;

pub use create::{
    EXAMPLE_ENVIRONMENT, EXAMPLE_REQUEST_ID, GITIGNORE, NewWorkspaceContent, create_workspace,
};
pub use env_edit::{EnvChange, EnvLayer, EnvLayers};
pub use error::WorkspaceError;
pub use git::git_branch;
pub use openapi::{ImportOrigin, OpenApiCollection, UpdateReport, openapi_collections_in};
pub use postman::ImportReport;
pub use project::ProjectWrite;
pub use sanitize::sanitize_file_name;
pub use tree::{Folder, Node, RequestEntry, natural_cmp};
pub use workspace::Workspace;

/// The extension (without the leading dot) of a request file.
pub(crate) const REQUEST_EXTENSION: &str = "brasa";

/// The name of the top-level folder that holds environment files instead of a collection.
pub(crate) const ENVIRONMENTS_FOLDER: &str = "environments";

/// The most files and folders [`Workspace::open`] scans (hidden entries and skipped folders such
/// as `node_modules` do not count) before it gives up with [`WorkspaceError::TooLarge`]. Every
/// change to the workspace rescans the whole folder, so a folder this big, such as a home folder
/// or the root of a large repository, would make every save slow. Real workspaces hold a few
/// hundred entries.
pub const MAX_WORKSPACE_ENTRIES: usize = 20_000;
