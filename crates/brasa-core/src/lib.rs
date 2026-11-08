//! Domain model, variable interpolation and template functions for Brasa.
//!
//! This crate has no filesystem or network access. It defines the plain data types shared by
//! every other crate (`Method`, `Request`, `Environment`, `Response`, ...), the `{{ }}` variable
//! interpolation rules and the built-in template functions (`uuid()`, `now()`, ...) described in
//! `docs/format.md`.
#![warn(missing_docs)]

mod auth;
mod body;
mod environment;
pub mod functions;
mod interpolate;
mod key_value;
pub mod log_safe;
mod method;
mod request;
mod response;

pub use auth::{ApiKeyLocation, Auth};
pub use body::{Body, MultipartValue, multipart_value};
pub use environment::Environment;
pub use functions::{Arg, FunctionError};
pub use interpolate::{
    Interpolated, TemplateWarning, VarScope, VariableKind, VariableSpan, interpolate,
    variable_spans,
};
pub use key_value::KeyValue;
pub use method::Method;
pub use request::Request;
pub use response::{
    ConsoleLevel, ConsoleLine, ResolvedBody, ResolvedField, ResolvedRequest, Response, ScriptStage,
    TestResult,
};
