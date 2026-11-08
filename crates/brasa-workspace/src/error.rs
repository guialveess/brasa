//! The error type returned by most [`crate::Workspace`] operations.

use std::path::PathBuf;

/// Everything that can go wrong while working with a [`crate::Workspace`].
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// The given root path does not exist or is not a directory.
    #[error("{0:?} does not exist or is not a directory")]
    RootNotFound(PathBuf),

    /// An id is empty, absolute, or would escape the workspace root, for example because it
    /// contains a `..` component. Ids must be plain relative paths made of normal components
    /// (paths that escape the workspace are rejected).
    #[error("{0:?} is not a valid workspace id")]
    InvalidId(String),

    /// No file or folder exists for the given id.
    #[error("no entry found for {0:?}")]
    NotFound(String),

    /// A file or folder already exists at the target path.
    #[error("{0:?} already exists")]
    AlreadyExists(String),

    /// [`crate::Workspace::move_entry`] refused to move a folder into itself or into one of its
    /// own subfolders.
    #[error("{0:?} cannot be moved into itself")]
    MoveIntoItself(String),

    /// [`crate::create_workspace`] refused a folder that already exists and has something inside,
    /// hidden files included.
    #[error("{0:?} already exists and is not empty")]
    NotEmpty(PathBuf),

    /// The named environment has neither a `.env` nor a `.local.env` file.
    #[error("environment {0:?} not found")]
    EnvironmentNotFound(String),

    /// The content of a `.brasa` file could not be parsed.
    #[error(transparent)]
    Parse(#[from] brasa_format::ParseError),

    /// The content of an environment file could not be parsed.
    #[error("environment {name:?}: {source}")]
    EnvParse {
        /// The environment name being loaded.
        name: String,
        /// The underlying parse error.
        #[source]
        source: brasa_format::env::EnvParseError,
    },

    /// [`crate::Workspace::open`] gave up on a folder with more than
    /// [`crate::MAX_WORKSPACE_ENTRIES`] files and folders, too many to be a workspace.
    #[error(
        "{path:?} has more than {limit} files and folders, too many for a workspace. Open the folder that holds your requests instead"
    )]
    TooLarge {
        /// The folder that was being opened.
        path: PathBuf,
        /// The limit it went over.
        limit: usize,
    },

    /// A filesystem operation failed.
    #[error("io error at {path:?}: {source}")]
    Io {
        /// The path the operation was attempted on.
        path: PathBuf,
        /// The underlying IO error.
        #[source]
        source: std::io::Error,
    },

    /// A Postman collection or environment export could not be parsed.
    #[error(transparent)]
    Postman(#[from] brasa_format::postman::PostmanError),

    /// The workspace configuration file (`.brasa/project.toml`) is not valid TOML.
    #[error(transparent)]
    Project(#[from] brasa_format::project::ProjectError),
}
