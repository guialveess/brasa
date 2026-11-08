//! Plain owned data types exchanged with a [`crate::ScriptEngine`].
//!
//! Every type here is ordinary Rust data (no `rquickjs` type appears in this module, or anywhere
//! else in this crate's public API), matching `docs/scripting.md`. Headers, request
//! variables and environment overrides all reuse [`brasa_core::KeyValue`], the same type used
//! for headers throughout `brasa-core`.

use brasa_core::{ConsoleLine, KeyValue, TestResult};

/// The request as seen by a script: the `req` global of `docs/scripting.md`.
///
/// This mirrors only the fields a script can see or change (`method`, `url`, `headers`,
/// `body`), not the full `brasa_core::Request` (which also carries `query`, the scripts
/// themselves and `docs`). Building this from a `Request` and merging it back is the job of
/// `brasa-runner`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScriptRequest {
    /// The HTTP method, for example `"GET"`.
    pub method: String,
    /// The request URL, possibly still containing `{{ }}` placeholders.
    pub url: String,
    /// The request headers, in order. Reused from `brasa-core`; `enabled` is not interpreted
    /// by this crate, callers are expected to only pass in the headers meant to be visible to
    /// the script.
    pub headers: Vec<KeyValue>,
    /// The request body as raw text.
    pub body: String,
}

/// The response as seen by a `post` script: the `res` global of `docs/scripting.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptResponse {
    /// The HTTP status code.
    pub status: u16,
    /// The response headers, in the order received.
    pub headers: Vec<KeyValue>,
    /// The response body, decoded as UTF-8 (lossily, if the body is not valid UTF-8) so it can
    /// be handed to the script as a plain JavaScript string.
    pub body: String,
    /// Total time spent sending the request and receiving the response, in milliseconds.
    pub time_ms: u128,
    /// The response body size in bytes.
    pub size: usize,
}

/// A single change a script made to the session environment via `env.set`/`env.unset`.
///
/// Kept as an ordered list of changes, rather than just the final state, so a caller
/// (`brasa-runner`) can replay them onto its own `SessionEnv` in the order they happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvChange {
    /// `env.set(key, value)`.
    Set {
        /// The variable name.
        key: String,
        /// The new value, already converted to a string as `env.set` does in JavaScript.
        value: String,
    },
    /// `env.unset(key)`.
    Unset {
        /// The variable name to remove.
        key: String,
    },
}

/// Input to [`crate::ScriptEngine::run_pre`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreContext {
    /// The request about to be sent, editable by the script.
    pub request: ScriptRequest,
    /// Request-scoped variables visible through `vars.get`/`vars.set`, valid only for this run.
    pub vars: Vec<KeyValue>,
    /// The active environment, merged with any session overrides, visible through `env.get`.
    pub env: Vec<KeyValue>,
}

/// The result of running a `::: pre` script.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreOutcome {
    /// The request, with whatever changes the script made to `req.method`, `req.url`,
    /// `req.headers` and `req.body`.
    pub request: ScriptRequest,
    /// The final state of the request-scoped variables.
    pub vars: Vec<KeyValue>,
    /// Every `env.set`/`env.unset` call the script made, in order.
    pub env_changes: Vec<EnvChange>,
    /// Lines captured from `console.log`/`info`/`warn`/`error`, in order.
    pub console: Vec<ConsoleLine>,
    /// The outcome of every `test(name, fn)` call, in order.
    pub tests: Vec<TestResult>,
}

/// Input to [`crate::ScriptEngine::run_post`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostContext {
    /// The request exactly as it was sent. Read-only from the script.
    pub request: ScriptRequest,
    /// The response received.
    pub response: ScriptResponse,
    /// Request-scoped variables, as left by the pre script (or empty, if there was none).
    pub vars: Vec<KeyValue>,
    /// The active environment, merged with any session overrides.
    pub env: Vec<KeyValue>,
}

/// The result of running a `::: post` script.
///
/// An uncaught exception in a post script does not abort the run (`docs/scripting.md`):
/// the response is still shown. So, unlike [`crate::ScriptEngine::run_pre`], `run_post` only
/// returns `Err` for an engine-level failure (the wall-time or memory limit was hit, or the
/// engine itself could not start). A plain uncaught exception from the script is instead reported
/// through [`PostOutcome::script_error`], alongside whatever console lines, test results and
/// variable changes happened before it was thrown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PostOutcome {
    /// The final state of the request-scoped variables.
    pub vars: Vec<KeyValue>,
    /// Every `env.set`/`env.unset` call the script made, in order.
    pub env_changes: Vec<EnvChange>,
    /// Lines captured from `console.log`/`info`/`warn`/`error`, in order.
    pub console: Vec<ConsoleLine>,
    /// The outcome of every `test(name, fn)` call, in order.
    pub tests: Vec<TestResult>,
    /// Set when the script raised an uncaught exception (a syntax error or a runtime error).
    /// `None` means the script ran to completion without one.
    pub script_error: Option<crate::error::ScriptError>,
}
