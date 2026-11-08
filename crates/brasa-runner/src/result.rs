//! [`RunResult`], everything the UI needs to render the outcome of a [`crate::Runner::run`]
//! call, and [`FailedStage`], which pipeline stage failed, if any.

use std::time::SystemTime;

use brasa_core::{ConsoleLine, ResolvedRequest, Response, TemplateWarning, TestResult};
use brasa_http::HttpError;
use brasa_script::ScriptError;

/// The complete outcome of running a request through [`crate::Runner::run`].
///
/// This is plain owned data: everything the UI needs to show is here, with no further calls
/// needed. `resolved_request` and `response` are both `None` only when a pre script exception
/// aborted the run before anything was sent; `response` alone is `None` when sending itself
/// failed.
#[derive(Debug)]
pub struct RunResult {
    /// The request actually sent, after variable interpolation. `None` if a pre script exception
    /// prevented the request from ever being built.
    pub resolved_request: Option<ResolvedRequest>,
    /// The response received. `None` if the request was never sent, or if sending it failed.
    pub response: Option<Response>,
    /// Every `test(name, fn)` result from the pre and post scripts, in order.
    pub tests: Vec<TestResult>,
    /// Every `console.log`/`info`/`warn`/`error` line from the pre and post scripts, in order.
    pub console: Vec<ConsoleLine>,
    /// Every unresolved `{{ }}` marker found while interpolating the request, in order.
    pub warnings: Vec<TemplateWarning>,
    /// Which pipeline stage failed, if any. `None` means the request ran to completion: it was
    /// sent and, if there was a post script, it ran without raising an uncaught exception.
    pub failed_stage: Option<FailedStage>,
    /// When the run started, before the pre script.
    pub started_at: SystemTime,
    /// When the request was handed to the HTTP client. `None` if it was never sent.
    pub sent_at: Option<SystemTime>,
    /// When the run ended, after the post script or the stage that failed.
    pub finished_at: SystemTime,
}

/// The pipeline stage that stopped a run from completing normally.
///
/// Interpolation itself is never a failing stage: an unresolved marker is reported as a warning
/// on [`RunResult::warnings`] instead, and the run continues.
#[derive(Debug, thiserror::Error)]
pub enum FailedStage {
    /// The `::: pre` script raised an uncaught exception, or the script engine itself failed.
    /// The request was never sent.
    #[error("pre-request script failed: {0}")]
    Pre(#[source] ScriptError),
    /// Sending the resolved request failed. No post script ran.
    #[error("sending the request failed: {0}")]
    Send(#[source] HttpError),
    /// The `::: post` script raised an uncaught exception, or the script engine itself failed.
    /// The response was already received and is still reported on [`RunResult::response`].
    #[error("post-response script failed: {0}")]
    Post(#[source] ScriptError),
}
