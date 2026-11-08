//! Drives the `git` executable installed on the machine: detection, repository discovery,
//! status, staging, discarding changes, diffs, applying patches, committing, pushing, pulling and
//! fetching, branches and checkout, the stash, the commit log, and the lane layout of the commit
//! graph.
//!
//! Everything is synchronous and blocking, the app calls it from a background executor. Git is
//! always run as a child process (never a library) with a fixed environment: English messages,
//! no terminal prompts, no optional locks and unquoted paths. Paths in the results are relative to
//! the repository root and separated by `/` on every platform.
//!
//! On Windows it can also run the git of a WSL distribution, through `wsl.exe`, for a folder
//! inside WSL (see [`detect_for`]).
#![warn(missing_docs)]

mod branch;
mod commit;
mod detect;
mod diff;
mod error;
mod file_diff;
mod graph;
mod log;
mod repo;
mod run;
mod stash;
mod status;
mod sync;
mod wsl;

pub use branch::{BranchInfo, RefKind};
pub use detect::{Executable, MIN_VERSION, Version, detect, detect_for};
pub use diff::{ApplyTarget, DiffSource, FileVersion, NumStat, hunk_patch};
pub use error::{DetectError, FailureKind, GitError};
pub use file_diff::{
    ChangeBlock, DiffLine, FileDiff, LineKind, SplitRow, block_patch, parse_file_diff,
};
pub use graph::{GraphLayout, GraphLine, GraphRow, LinePart};
pub use log::{CommitDetail, CommitFile, LogCommit, LogQuery, RefLabel, RefLabelKind};
pub use repo::{Author, Git, Repo};
pub use stash::{StashEntry, auto_stash_message};
pub use status::{Change, Entry, Head, Status};
pub use sync::{CommitSummary, PullMode, PushTarget};
pub use wsl::{WslLocation, wsl_location};
