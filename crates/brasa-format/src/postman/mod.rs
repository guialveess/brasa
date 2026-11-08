//! Importing Postman collections (v2.1, v2.0 accepted when it parses the same), environment and
//! globals exports (see `docs/postman-import.md`).
//!
//! This module is pure, like the rest of `brasa-format`: it only turns JSON text into plain
//! in-memory data, it never touches the filesystem. [`build_postman_plan`] gives the same plan an
//! OpenAPI import writes, so the wizard's preview, `brasa-workspace` and the update of a
//! collection work on one shape.

mod collection;
mod environment;
mod error;
mod model;
mod plan;
mod script;
mod value;

pub use collection::{
    ImportFolder, ImportPlan, IssueLevel, PostmanOptions, RequestIssue, parse_collection,
};
pub use environment::{ImportedEnvironment, parse_environment};
pub use error::PostmanError;
pub use plan::{MANIFEST_KIND, PostmanFormat, PostmanPlan, build_postman_plan, detect_format};
