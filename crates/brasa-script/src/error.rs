//! The error type returned by a [`crate::ScriptEngine`].

/// An error running a `::: pre` or `::: post` script.
///
/// This is plain owned data: no `rquickjs` type appears in it, so callers never need the
/// `rquickjs` crate themselves.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    /// The script did not parse as valid JavaScript.
    #[error("syntax error: {message}")]
    Syntax {
        /// The message QuickJS reported.
        message: String,
        /// The line number inside the script, when QuickJS's error stack lets us recover one.
        line: Option<u32>,
    },
    /// The script raised an exception nothing caught.
    #[error("script error: {message}")]
    Runtime {
        /// The exception's message (or, for a thrown non-`Error` value, its string form).
        message: String,
        /// The exception's stack trace, when available.
        stack: Option<String>,
    },
    /// The script ran for longer than the wall-clock time limit (5 seconds).
    #[error("script timed out")]
    Timeout,
    /// The script exceeded the memory limit (32 MiB).
    #[error("script exceeded its memory limit")]
    MemoryLimit,
    /// The script engine itself failed, independently of the script's content (for example, the
    /// QuickJS runtime could not be allocated, or our own `prelude.js` failed to evaluate). This
    /// should not happen in practice.
    #[error("script engine error: {0}")]
    Engine(String),
}
