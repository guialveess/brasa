//! The request execution pipeline: build the variable scope, run the pre script, interpolate,
//! send the request, run the post script, and collect the result.
//!
//! See `docs/architecture.md` for the pipeline. The single entry point is [`Runner::run`], which never fails:
//! every kind of failure is reported inside the returned [`RunResult`] instead of a top-level
//! `Result` (see [`RunResult::failed_stage`]).
#![warn(missing_docs)]

mod preview;
mod resolve;
mod result;
mod runner;
mod script_bridge;
mod session_env;

pub use brasa_http::{Client, HttpError, InvalidCertificates, SendOptions};
pub use brasa_script::ScriptEngine;
pub use preview::{Preview, UnknownVariable, VariableLocation, preview};
pub use result::{FailedStage, RunResult};
pub use runner::Runner;
pub use session_env::SessionEnv;
