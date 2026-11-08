//! The [`ScriptEngine`] trait.

use crate::error::ScriptError;
use crate::types::{PostContext, PostOutcome, PreContext, PreOutcome};

/// Runs the `::: pre` and `::: post` JavaScript sections of a `.brasa` request, (see
/// `docs/scripting.md`).
///
/// Implementations must be sandboxed: no network, no filesystem, no timers, no process access,
/// only the request, the response, variables, the environment, tests and the `util` library.
/// [`crate::QuickJsEngine`] is the real implementation, on top of QuickJS. [`crate::NoopEngine`]
/// is a trivial pass-through implementation for tests of other crates that need a
/// `ScriptEngine` but do not want to run real scripts.
pub trait ScriptEngine: Send + Sync {
    /// Runs `script` as a `::: pre` section.
    ///
    /// An uncaught exception aborts the run: it is returned as `Err`, and the caller must not
    /// send the request.
    fn run_pre(&self, script: &str, ctx: PreContext) -> Result<PreOutcome, ScriptError>;

    /// Runs `script` as a `::: post` section.
    ///
    /// Unlike `run_pre`, a plain uncaught exception in the script does not abort anything: it is
    /// reported through [`PostOutcome::script_error`], the response has already been received
    /// and is still shown. `Err` is reserved for an engine-level failure (the memory or wall
    /// time limit was hit, or the engine itself could not start).
    fn run_post(&self, script: &str, ctx: PostContext) -> Result<PostOutcome, ScriptError>;
}
