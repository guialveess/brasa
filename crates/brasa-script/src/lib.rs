//! The `ScriptEngine` trait and its QuickJS implementation, used to run the `::: pre` and
//! `::: post` JavaScript sections of a `.brasa` request in a sandboxed runtime.
//!
//! See `docs/scripting.md` for the full script API and sandboxing rules. In short: a
//! fresh QuickJS runtime and context are created for every single run, with a 32 MiB memory
//! limit and a 5 second wall-time limit, no module loader, no `fetch`, no timers and no other
//! host access beyond the `req`, `res`, `vars`, `env`, `test`, `expect`, `console` and `util`
//! globals `prelude.js` defines. Every public type in this crate is plain owned data: no
//! `rquickjs` type is ever exposed.
#![warn(missing_docs)]

pub mod api;
mod engine;
mod error;
mod noop;
mod quickjs_engine;
mod types;

pub use engine::ScriptEngine;
pub use error::ScriptError;
pub use noop::NoopEngine;
pub use quickjs_engine::QuickJsEngine;
pub use types::{
    EnvChange, PostContext, PostOutcome, PreContext, PreOutcome, ScriptRequest, ScriptResponse,
};
