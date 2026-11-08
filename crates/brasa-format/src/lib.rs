//! Parsing and serialization of the `.brasa` request file format and of `.env` environment
//! files. Pure, no filesystem or network access: every function takes and returns plain strings
//! or `brasa-core` types, and the caller (`brasa-workspace`) does the actual file IO.
//!
//! See `docs/format.md` for the full format specification.
//!
//! The `postman` module (Postman collection import, see `docs/postman-import.md`) maps Postman collection
//! and environment exports into the same `brasa-core` types, pure JSON in, plain data out.
//!
//! The `openapi` module (OpenAPI and Swagger import, see `docs/openapi-import.md`) reads OpenAPI
//! 3.1, 3.0 and Swagger 2.0 specs, JSON or YAML, into the same `brasa-core` types.
//!
//! The `snippet` module turns a resolved request into copyable code (curl, JavaScript `fetch`,
//! Python `requests`) for the UI's "Code" action.
//!
//! The `semantic_diff` module compares two versions of a `.brasa` file part by part (method and
//! URL, headers, query, auth, body, scripts, docs) for the diff tab's "Request" mode.
//! The `bruno` module imports Bruno collections, `.bru` or OpenCollection YAML, into the same plan
//! as a Postman import (see `docs/bruno-import.md`).
//!
//! The `export` module writes a folder of requests out to another tool's collection format, the
//! reverse direction of `postman` and `bruno`'s import.
//!
//! The `project` module reads and writes the workspace configuration file, `.brasa/project.toml`.
//! The `highlight` module colors `.brasa`, `.env`, JSON and JavaScript text a line at a time for
//! the diff tab.
#![warn(missing_docs)]

mod format;
mod script_convert;
mod semantic_diff;

pub mod bruno;
pub mod env;
pub mod export;
pub mod highlight;
pub mod openapi;
pub mod postman;
pub mod project;
pub mod snippet;

pub use format::{ParseError, ParseErrorKind, parse, serialize};
pub use semantic_diff::{
    DiffRow, DiffRowKind, PartDiff, PartSummary, RequestDiff, RequestDiffError, RequestPart,
    RowNote, diff_requests,
};
