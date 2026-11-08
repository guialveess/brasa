//! [`NoopEngine`], a trivial [`crate::ScriptEngine`] for tests of other crates.

use crate::engine::ScriptEngine;
use crate::error::ScriptError;
use crate::types::{PostContext, PostOutcome, PreContext, PreOutcome};

/// A [`ScriptEngine`] that never actually runs JavaScript.
///
/// `run_pre` returns the request and variables unchanged, with no environment changes, console
/// lines or test results. `run_post` is the same, minus the request (post scripts cannot change
/// it anyway). Neither ever fails. This is meant for tests in `brasa-runner` and
/// `brasa-app` that need some `ScriptEngine` but do not want to depend on QuickJS or exercise
/// real scripts; it ignores the script text entirely.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopEngine;

impl ScriptEngine for NoopEngine {
    fn run_pre(&self, _script: &str, ctx: PreContext) -> Result<PreOutcome, ScriptError> {
        Ok(PreOutcome {
            request: ctx.request,
            vars: ctx.vars,
            env_changes: Vec::new(),
            console: Vec::new(),
            tests: Vec::new(),
        })
    }

    fn run_post(&self, _script: &str, ctx: PostContext) -> Result<PostOutcome, ScriptError> {
        Ok(PostOutcome {
            vars: ctx.vars,
            env_changes: Vec::new(),
            console: Vec::new(),
            tests: Vec::new(),
            script_error: None,
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use brasa_core::KeyValue;
    use pretty_assertions::assert_eq;

    #[test]
    fn run_pre_passes_the_request_and_vars_through_unchanged() {
        let engine = NoopEngine;
        let ctx = PreContext {
            request: crate::types::ScriptRequest {
                method: "GET".to_string(),
                url: "https://example.com".to_string(),
                headers: vec![KeyValue::new("Accept", "*/*")],
                body: String::new(),
            },
            vars: vec![KeyValue::new("a", "1")],
            env: Vec::new(),
        };
        let outcome = engine
            .run_pre("vars.set('a', '2');", ctx.clone())
            .expect("NoopEngine never fails");
        assert_eq!(outcome.request, ctx.request);
        assert_eq!(outcome.vars, ctx.vars);
        assert!(outcome.env_changes.is_empty());
        assert!(outcome.console.is_empty());
        assert!(outcome.tests.is_empty());
    }

    #[test]
    fn run_post_never_reports_a_script_error() {
        let engine = NoopEngine;
        let ctx = PostContext {
            request: crate::types::ScriptRequest::default(),
            response: crate::types::ScriptResponse {
                status: 200,
                headers: Vec::new(),
                body: String::new(),
                time_ms: 0,
                size: 0,
            },
            vars: Vec::new(),
            env: Vec::new(),
        };
        let outcome = engine
            .run_post("test('x', () => expect(1).toBe(2));", ctx)
            .expect("NoopEngine never fails");
        assert!(outcome.script_error.is_none());
        assert!(outcome.tests.is_empty());
    }
}
