//! [`QuickJsEngine`], the real [`crate::ScriptEngine`] implementation, on top of QuickJS via
//! `rquickjs`.

use std::time::{Duration, Instant, UNIX_EPOCH};

use brasa_core::{ConsoleLevel, ConsoleLine, KeyValue, ScriptStage, TestResult};
use rquickjs::context::EvalOptions;
use rquickjs::{
    Coerced, Context, Ctx, Error as JsError, Exception, FromJs, Function, Object, Runtime, Value,
};
use serde::{Deserialize, Serialize};

use crate::engine::ScriptEngine;
use crate::error::ScriptError;
use crate::types::{EnvChange, PostContext, PostOutcome, PreContext, PreOutcome, ScriptRequest};

/// The embedded prelude that builds `req`, `res`, `vars`, `env`, `test`, `expect` and `console`
/// on top of a plain data global (`__input`) and the native `util` bindings. See
/// `src/prelude.js`.
const PRELUDE_JS: &str = include_str!("prelude.js");

/// The QuickJS memory limit for a single script run, 32 MiB (see `docs/scripting.md`).
const MEMORY_LIMIT_BYTES: usize = 32 * 1024 * 1024;

/// The wall-clock time limit for a single script run, 5 seconds (see `docs/scripting.md`).
const TIME_LIMIT: Duration = Duration::from_secs(5);

/// The real [`ScriptEngine`], backed by the QuickJS engine through `rquickjs`.
///
/// Every call to [`ScriptEngine::run_pre`] or [`ScriptEngine::run_post`] creates a brand new
/// QuickJS `Runtime` and `Context`, runs the script, and tears them down: no state leaks between
/// runs. The sandbox has no module loader, no `fetch`, no timers and no other host access; the
/// only functions available beyond standard ECMAScript are the ones `prelude.js` and the native
/// `util` bindings define.
#[derive(Debug, Clone, Copy, Default)]
pub struct QuickJsEngine;

impl ScriptEngine for QuickJsEngine {
    fn run_pre(&self, script: &str, ctx: PreContext) -> Result<PreOutcome, ScriptError> {
        if script.trim().is_empty() {
            return Ok(PreOutcome {
                request: ctx.request,
                vars: ctx.vars,
                env_changes: Vec::new(),
                console: Vec::new(),
                tests: Vec::new(),
            });
        }

        let input = WireInput {
            request: WireRequest::from(&ctx.request),
            vars: to_wire_kv(&ctx.vars),
            env: to_wire_kv(&ctx.env),
            response: None,
        };
        let run = run_script("pre-script", script, &input)?;
        if let Some(error) = run.script_error {
            // A pre script that throws aborts the whole run: the request is never sent.
            return Err(error);
        }
        let output = run.output;
        let request = output
            .request
            .map(ScriptRequest::from)
            .unwrap_or(ctx.request);
        Ok(PreOutcome {
            request,
            vars: from_wire_kv(output.vars),
            env_changes: from_wire_env_changes(output.env_changes),
            console: from_wire_console(output.console, ScriptStage::Pre),
            tests: from_wire_tests(output.tests),
        })
    }

    fn run_post(&self, script: &str, ctx: PostContext) -> Result<PostOutcome, ScriptError> {
        if script.trim().is_empty() {
            return Ok(PostOutcome {
                vars: ctx.vars,
                env_changes: Vec::new(),
                console: Vec::new(),
                tests: Vec::new(),
                script_error: None,
            });
        }

        let input = WireInput {
            request: WireRequest::from(&ctx.request),
            vars: to_wire_kv(&ctx.vars),
            env: to_wire_kv(&ctx.env),
            response: Some(WireResponse::from(&ctx.response)),
        };
        let run = run_script("post-script", script, &input)?;
        let output = run.output;
        Ok(PostOutcome {
            vars: from_wire_kv(output.vars),
            env_changes: from_wire_env_changes(output.env_changes),
            console: from_wire_console(output.console, ScriptStage::Post),
            tests: from_wire_tests(output.tests),
            script_error: run.script_error,
        })
    }
}

/// The outcome of [`run_script`]: the collected JS-side state, plus an uncaught exception from
/// the user script, if any. The caller decides what to do with `script_error` (abort, for
/// `run_pre`, or report it, for `run_post`); [`run_script`] itself only returns `Err` for an
/// engine-level failure (timeout, memory limit, or the engine failing to start).
struct ScriptRun {
    output: WireOutput,
    script_error: Option<ScriptError>,
}

/// Creates a fresh QuickJS runtime and context, installs the sandbox (native `util` bindings,
/// `prelude.js`, the given `input`), runs `script`, and reads back the collected state.
///
/// `filename` is only used to label the script in QuickJS's own error stacks.
fn run_script(filename: &str, script: &str, input: &WireInput) -> Result<ScriptRun, ScriptError> {
    let runtime = Runtime::new().map_err(|error| ScriptError::Engine(error.to_string()))?;
    runtime.set_memory_limit(MEMORY_LIMIT_BYTES);
    let deadline = Instant::now() + TIME_LIMIT;
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));

    let context =
        Context::full(&runtime).map_err(|error| ScriptError::Engine(error.to_string()))?;

    let result = context.with(|ctx| -> Result<ScriptRun, ScriptError> {
        install_util(&ctx).map_err(|error| engine_error(&ctx, error))?;

        let input_json = serde_json::to_string(input).map_err(|error| {
            ScriptError::Engine(format!("could not encode script input: {error}"))
        })?;
        let input_value = ctx
            .json_parse(input_json)
            .map_err(|error| engine_error(&ctx, error))?;
        ctx.globals()
            .set("__input", input_value)
            .map_err(|error| engine_error(&ctx, error))?;

        eval_prelude(&ctx)?;

        let script_error = eval_user_script(&ctx, filename, script, deadline)?;

        let output_json: String = ctx
            .eval("JSON.stringify(__collect())")
            .map_err(|error| engine_error(&ctx, error))?;
        let output: WireOutput = serde_json::from_str(&output_json).map_err(|error| {
            ScriptError::Engine(format!("could not decode script output: {error}"))
        })?;

        Ok(ScriptRun {
            output,
            script_error,
        })
    });

    // `Context::with` holds the runtime's internal lock for the whole call above, and
    // `Runtime::memory_usage` needs that same lock, so it can only be inspected once `with` has
    // returned. QuickJS does not raise a distinct error type for an allocation failure (see
    // `classify_exception`), so a run that failed for any reason while sitting at the memory
    // limit is reported as having hit it, overriding whatever else was caught. QuickJS refuses
    // an allocation that would cross the limit rather than letting `malloc_size` reach it
    // exactly, so this checks for at least 90% of the limit rather than requiring an exact
    // match.
    let usage = runtime.memory_usage();
    let hit_memory_limit = usage.malloc_limit > 0
        && usage.malloc_size.saturating_mul(10) >= usage.malloc_limit.saturating_mul(9);
    if !hit_memory_limit {
        return result;
    }
    match result {
        Ok(run) if run.script_error.is_none() => Ok(run),
        _ => Err(ScriptError::MemoryLimit),
    }
}

/// Evaluates the embedded `prelude.js`. A failure here means a bug in `prelude.js` itself, not
/// in the user's script, so it is always reported as [`ScriptError::Engine`].
fn eval_prelude(ctx: &Ctx<'_>) -> Result<(), ScriptError> {
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = true;
    options.filename = Some("prelude.js".to_string());
    ctx.eval_with_options::<(), _>(PRELUDE_JS, options)
        .map_err(|error| engine_error(ctx, error))
}

/// Evaluates the user's `pre`/`post` script directly (not wrapped in anything), so that a syntax
/// error's line number is relative to the script itself.
///
/// Returns `Ok(None)` if the script ran without raising an uncaught exception, `Ok(Some(_))` for
/// a plain uncaught exception (syntax or runtime), and `Err` only for the wall-time or memory
/// limit being hit.
fn eval_user_script(
    ctx: &Ctx<'_>,
    filename: &str,
    script: &str,
    deadline: Instant,
) -> Result<Option<ScriptError>, ScriptError> {
    let mut options = EvalOptions::default();
    options.global = true;
    options.strict = true;
    options.filename = Some(filename.to_string());
    match ctx.eval_with_options::<Value, _>(script, options) {
        Ok(_) => Ok(None),
        Err(JsError::Exception) => match classify_exception(ctx, deadline) {
            ClassifiedException::Fatal(error) => Err(error),
            ClassifiedException::Reported(error) => Ok(Some(error)),
        },
        Err(other) => Err(ScriptError::Engine(other.to_string())),
    }
}

/// A caught JavaScript exception, classified into either a script-level problem (reported back
/// to the caller, never a Rust-level `Err`) or a resource limit (always a Rust-level `Err`,
/// since the runtime should not be trusted to keep running past it).
enum ClassifiedException {
    /// A syntax or runtime error in the script itself.
    Reported(ScriptError),
    /// The wall-time or memory limit was hit.
    Fatal(ScriptError),
}

/// Reads the JavaScript exception QuickJS just raised (via `Ctx::catch`) and turns it into a
/// [`ClassifiedException`].
///
/// The interrupt handler makes QuickJS raise a plain `InternalError: interrupted` exception,
/// not some distinct Rust-level error, so the timeout is recognized first, ahead of the
/// exception's own content, purely by the deadline having passed. The memory limit is handled
/// separately by the caller ([`run_script`]): an allocation failure does not even leave behind a
/// proper exception object (QuickJS raises `null`), and inspecting the runtime's own memory
/// usage needs a lock this function does not hold.
fn classify_exception(ctx: &Ctx<'_>, deadline: Instant) -> ClassifiedException {
    if Instant::now() >= deadline {
        return ClassifiedException::Fatal(ScriptError::Timeout);
    }

    let caught = ctx.catch();

    if let Some(exception) = caught.as_exception() {
        let message = exception.message().unwrap_or_default();
        let stack = exception.stack();
        let name: Option<String> = exception
            .as_object()
            .get::<_, Option<String>>("name")
            .ok()
            .flatten();

        if name.as_deref() == Some("SyntaxError") {
            let line = stack.as_deref().and_then(extract_line_number);
            return ClassifiedException::Reported(ScriptError::Syntax { message, line });
        }
        return ClassifiedException::Reported(ScriptError::Runtime { message, stack });
    }

    // A non-Error value was thrown (`throw "boom"`, `throw 42`, ...): fall back to coercing it
    // to a string the way `String(value)` would in JavaScript. This also covers the `null`
    // QuickJS leaves behind for an out-of-memory condition that the check above did not catch.
    let message = coerce_to_string(ctx, &caught);
    ClassifiedException::Reported(ScriptError::Runtime {
        message,
        stack: None,
    })
}

/// Coerces any JavaScript value to a string, the way `String(value)` would, without ever
/// panicking: falls back to a fixed placeholder if even that fails.
fn coerce_to_string<'js>(ctx: &Ctx<'js>, value: &Value<'js>) -> String {
    Coerced::<String>::from_js(ctx, value.clone())
        .map(|coerced| coerced.0)
        .unwrap_or_else(|_| "<could not describe the thrown value>".to_string())
}

/// Scans a QuickJS stack trace for the first `:<line>:<column>` pair and returns the line
/// number. QuickJS stacks look like `    at <anonymous> (pre-script:3:5)`; this is a best-effort
/// heuristic, not a real stack trace parser, since `rquickjs` 0.14 does not expose a structured
/// line number for a caught exception.
fn extract_line_number(stack: &str) -> Option<u32> {
    for line in stack.lines() {
        let trimmed = line.trim_end_matches(')');
        let last_colon = trimmed.rfind(':')?;
        let before_last = &trimmed[..last_colon];
        let column_candidate = &trimmed[last_colon + 1..];
        if column_candidate.parse::<u32>().is_err() {
            continue;
        }
        if let Some(middle_colon) = before_last.rfind(':') {
            let line_candidate = &before_last[middle_colon + 1..];
            if let Ok(line_number) = line_candidate.parse::<u32>() {
                return Some(line_number);
            }
        }
    }
    None
}

/// Wraps any `rquickjs::Error` that is not itself a caught JS exception into a
/// [`ScriptError::Engine`], describing it as precisely as `rquickjs` allows. Used for failures
/// in our own glue code (setting up globals, evaluating `prelude.js`, decoding the collected
/// output), never for the user's script.
fn engine_error(ctx: &Ctx<'_>, error: JsError) -> ScriptError {
    if matches!(error, JsError::Exception) {
        let caught = ctx.catch();
        let message = caught
            .as_exception()
            .and_then(|exception| exception.message())
            .unwrap_or_else(|| coerce_to_string(ctx, &caught));
        return ScriptError::Engine(message);
    }
    ScriptError::Engine(error.to_string())
}

/// Installs the native `util` object as a global.
///
/// Every function forwards to [`brasa_core::functions::call`], the very same dispatcher used
/// by `{{ name(args) }}` template interpolation, so both levels always behave identically,
/// including the exact wording of their errors. `now()` and `randomInt()` are parsed back into
/// JavaScript numbers for ergonomics (so a script can do arithmetic with them); every other
/// function already returns the right kind of string.
fn install_util(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let util = Object::new(ctx.clone())?;

    util.set(
        "uuid",
        Function::new(ctx.clone(), |ctx: Ctx<'_>| call_util(&ctx, "uuid", &[]))?,
    )?;
    util.set(
        "now",
        Function::new(ctx.clone(), |ctx: Ctx<'_>| -> rquickjs::Result<f64> {
            parse_number(&ctx, call_util(&ctx, "now", &[])?)
        })?,
    )?;
    util.set(
        "isoDate",
        Function::new(ctx.clone(), |ctx: Ctx<'_>| call_util(&ctx, "isoDate", &[]))?,
    )?;
    util.set(
        "randomInt",
        Function::new(
            ctx.clone(),
            |ctx: Ctx<'_>, min: f64, max: f64| -> rquickjs::Result<f64> {
                let args = [
                    brasa_core::Arg::Int(min as i64),
                    brasa_core::Arg::Int(max as i64),
                ];
                parse_number(&ctx, call_util(&ctx, "randomInt", &args)?)
            },
        )?,
    )?;
    util.set(
        "randomString",
        Function::new(ctx.clone(), |ctx: Ctx<'_>, len: f64| {
            call_util(&ctx, "randomString", &[brasa_core::Arg::Int(len as i64)])
        })?,
    )?;
    util.set(
        "base64Encode",
        Function::new(ctx.clone(), |ctx: Ctx<'_>, value: String| {
            call_util(&ctx, "base64Encode", &[brasa_core::Arg::Str(value)])
        })?,
    )?;
    util.set(
        "base64Decode",
        Function::new(ctx.clone(), |ctx: Ctx<'_>, value: String| {
            call_util(&ctx, "base64Decode", &[brasa_core::Arg::Str(value)])
        })?,
    )?;
    util.set(
        "urlEncode",
        Function::new(ctx.clone(), |ctx: Ctx<'_>, value: String| {
            call_util(&ctx, "urlEncode", &[brasa_core::Arg::Str(value)])
        })?,
    )?;

    ctx.globals().set("util", util)
}

/// Calls the shared template function dispatcher, turning a [`brasa_core::FunctionError`]
/// into a thrown JavaScript exception with the exact same message.
fn call_util(ctx: &Ctx<'_>, name: &str, args: &[brasa_core::Arg]) -> rquickjs::Result<String> {
    brasa_core::functions::call(name, args)
        .map_err(|error| Exception::throw_message(ctx, &error.to_string()))
}

/// Parses a decimal string produced by [`brasa_core::functions::call`] back into a JavaScript
/// number. Only used for `util.now()` and `util.randomInt()`, whose values are always plain
/// integers, so this should never actually fail.
fn parse_number(ctx: &Ctx<'_>, text: String) -> rquickjs::Result<f64> {
    text.parse::<f64>()
        .map_err(|_| Exception::throw_message(ctx, "internal error: could not parse a number"))
}

// --- JSON wire types ----------------------------------------------------------------------
//
// `brasa_core::KeyValue`, `ConsoleLine` and `TestResult` are foreign types from this crate's
// point of view, so they cannot derive `serde::Serialize`/`Deserialize` here (and brasa-core
// itself must not gain a `serde` dependency just for this). These small mirror types carry the
// same data across the JSON boundary to and from `prelude.js`; converting between them and the
// public API types is a handful of plain field-by-field mappings below.

/// A `{ key, value }` pair, the wire form of [`KeyValue`] (minus `enabled`, which scripts never
/// see: only enabled entries are passed in, per [`ScriptRequest::headers`]'s doc comment).
#[derive(Debug, Serialize, Deserialize)]
struct WireKeyValue {
    key: String,
    value: String,
}

/// The wire form of [`ScriptRequest`].
#[derive(Debug, Serialize, Deserialize)]
struct WireRequest {
    method: String,
    url: String,
    headers: Vec<WireKeyValue>,
    body: String,
}

impl From<&ScriptRequest> for WireRequest {
    fn from(request: &ScriptRequest) -> Self {
        WireRequest {
            method: request.method.clone(),
            url: request.url.clone(),
            headers: to_wire_kv(&request.headers),
            body: request.body.clone(),
        }
    }
}

impl From<WireRequest> for ScriptRequest {
    fn from(wire: WireRequest) -> Self {
        ScriptRequest {
            method: wire.method,
            url: wire.url,
            headers: from_wire_kv(wire.headers),
            body: wire.body,
        }
    }
}

/// The wire form of [`crate::types::ScriptResponse`], only ever sent to JavaScript, never read
/// back (`res` is read-only in a post script).
#[derive(Debug, Serialize)]
struct WireResponse {
    status: u16,
    headers: Vec<WireKeyValue>,
    body: String,
    time_ms: f64,
    size: usize,
}

impl From<&crate::types::ScriptResponse> for WireResponse {
    fn from(response: &crate::types::ScriptResponse) -> Self {
        WireResponse {
            status: response.status,
            headers: to_wire_kv(&response.headers),
            body: response.body.clone(),
            // `time_ms` is a `u128` in the public API for headroom; every real request timing
            // fits comfortably in an `f64`, the only numeric type JavaScript has.
            time_ms: response.time_ms as f64,
            size: response.size,
        }
    }
}

/// The JSON object set as `globalThis.__input` before `prelude.js` runs.
#[derive(Debug, Serialize)]
struct WireInput {
    request: WireRequest,
    vars: Vec<WireKeyValue>,
    env: Vec<WireKeyValue>,
    response: Option<WireResponse>,
}

/// The wire form of an [`EnvChange`], tagged by `type` (`"set"` or `"unset"`).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum WireEnvChange {
    Set { key: String, value: String },
    Unset { key: String },
}

/// The wire form of a [`brasa_core::TestResult`].
#[derive(Debug, Deserialize)]
struct WireTestResult {
    name: String,
    passed: bool,
    message: Option<String>,
}

/// The wire form of a [`brasa_core::ConsoleLine`], `level` as the lowercase `console.*` method
/// name.
#[derive(Debug, Deserialize)]
struct WireConsoleLine {
    level: String,
    text: String,
    /// `Date.now()` when it was logged, milliseconds since the Unix epoch.
    #[serde(default)]
    time: Option<f64>,
    /// The script line that logged it, when the stack trace had one.
    #[serde(default)]
    line: Option<u32>,
}

/// The JSON object `JSON.stringify(__collect())` produces after a script run.
#[derive(Debug, Deserialize)]
struct WireOutput {
    /// `None` after a post script (read-only `req`); `Some` after a pre script.
    request: Option<WireRequest>,
    vars: Vec<WireKeyValue>,
    env_changes: Vec<WireEnvChange>,
    tests: Vec<WireTestResult>,
    console: Vec<WireConsoleLine>,
}

/// Converts enabled [`KeyValue`] entries to their wire form. Every entry passed into a script is
/// assumed to already be the one the caller wants visible, `enabled` itself is dropped since
/// `prelude.js` has no notion of a disabled entry.
fn to_wire_kv(entries: &[KeyValue]) -> Vec<WireKeyValue> {
    entries
        .iter()
        .map(|entry| WireKeyValue {
            key: entry.key.clone(),
            value: entry.value.clone(),
        })
        .collect()
}

/// Converts wire entries back to enabled [`KeyValue`] entries.
fn from_wire_kv(entries: Vec<WireKeyValue>) -> Vec<KeyValue> {
    entries
        .into_iter()
        .map(|entry| KeyValue::new(entry.key, entry.value))
        .collect()
}

/// Converts wire environment changes back to [`EnvChange`] values.
fn from_wire_env_changes(changes: Vec<WireEnvChange>) -> Vec<EnvChange> {
    changes
        .into_iter()
        .map(|change| match change {
            WireEnvChange::Set { key, value } => EnvChange::Set { key, value },
            WireEnvChange::Unset { key } => EnvChange::Unset { key },
        })
        .collect()
}

/// Converts wire test results back to [`TestResult`] values.
fn from_wire_tests(tests: Vec<WireTestResult>) -> Vec<TestResult> {
    tests
        .into_iter()
        .map(|test| TestResult {
            name: test.name,
            passed: test.passed,
            message: test.message,
        })
        .collect()
}

/// Converts wire console lines back to [`ConsoleLine`] values. An unrecognized level (which
/// `prelude.js` never actually produces) falls back to [`ConsoleLevel::Log`] rather than
/// panicking.
fn from_wire_console(lines: Vec<WireConsoleLine>, stage: ScriptStage) -> Vec<ConsoleLine> {
    lines
        .into_iter()
        .map(|line| ConsoleLine {
            stage,
            time: line
                .time
                .filter(|millis| millis.is_finite() && *millis >= 0.0)
                .map(|millis| UNIX_EPOCH + Duration::from_millis(millis as u64)),
            line: line.line,
            level: match line.level.as_str() {
                "info" => ConsoleLevel::Info,
                "warn" => ConsoleLevel::Warn,
                "error" => ConsoleLevel::Error,
                _ => ConsoleLevel::Log,
            },
            text: line.text,
        })
        .collect()
}

/// Runs `f` inside a sandbox set up exactly as for a real run (native `util` bindings, input
/// and `prelude.js`), for a post script (`post` true) or a pre script, without running any user
/// script. Lets the tests of `crate::api` inspect the globals the sandbox really defines.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) fn with_sandbox<R>(post: bool, f: impl FnOnce(&Ctx<'_>) -> R) -> R {
    let input = WireInput {
        request: WireRequest {
            method: "GET".to_string(),
            url: "http://localhost".to_string(),
            headers: Vec::new(),
            body: String::new(),
        },
        vars: Vec::new(),
        env: Vec::new(),
        response: post.then(|| WireResponse {
            status: 200,
            headers: Vec::new(),
            body: "{}".to_string(),
            time_ms: 1.0,
            size: 2,
        }),
    };
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        install_util(&ctx).unwrap();
        let value = ctx
            .json_parse(serde_json::to_string(&input).unwrap())
            .unwrap();
        ctx.globals().set("__input", value).unwrap();
        eval_prelude(&ctx).unwrap();
        f(&ctx)
    })
}

/// The own property names of `globalThis` in a plain QuickJS context with the same intrinsics as
/// the sandbox but none of its globals: the standard ECMAScript baseline.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) fn baseline_global_names() -> std::collections::BTreeSet<String> {
    let runtime = Runtime::new().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let json: String = ctx
            .eval("JSON.stringify(Object.getOwnPropertyNames(globalThis))")
            .unwrap();
        serde_json::from_str::<Vec<String>>(&json)
            .unwrap()
            .into_iter()
            .collect()
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::ScriptResponse;
    use brasa_core::KeyValue;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    fn engine() -> QuickJsEngine {
        QuickJsEngine
    }

    fn base_request() -> ScriptRequest {
        ScriptRequest {
            method: "GET".to_string(),
            url: "https://example.com".to_string(),
            headers: Vec::new(),
            body: String::new(),
        }
    }

    fn pre_ctx() -> PreContext {
        PreContext {
            request: base_request(),
            vars: Vec::new(),
            env: Vec::new(),
        }
    }

    fn find<'a>(tests: &'a [TestResult], name: &str) -> &'a TestResult {
        tests
            .iter()
            .find(|test| test.name == name)
            .unwrap_or_else(|| panic!("no test result named {name:?}, got {tests:?}"))
    }

    // --- expect matchers -----------------------------------------------------------------------

    #[test]
    fn every_expect_matcher_passes_fails_and_negates() {
        let script = r#"
            test("toBe pass", () => expect(1).toBe(1));
            test("toBe fail", () => expect(1).toBe(2));
            test("toBe not pass", () => expect(1).not.toBe(2));
            test("toBe not fail", () => expect(1).not.toBe(1));

            test("toEqual pass", () => expect({ a: 1, b: [1, 2] }).toEqual({ a: 1, b: [1, 2] }));
            test("toEqual fail", () => expect({ a: 1 }).toEqual({ a: 2 }));
            test("toEqual not pass", () => expect({ a: 1 }).not.toEqual({ a: 2 }));
            test("toEqual not fail", () => expect({ a: 1 }).not.toEqual({ a: 1 }));

            test("toBeTruthy pass", () => expect(1).toBeTruthy());
            test("toBeTruthy fail", () => expect(0).toBeTruthy());
            test("toBeTruthy not pass", () => expect(0).not.toBeTruthy());
            test("toBeTruthy not fail", () => expect(1).not.toBeTruthy());

            test("toBeFalsy pass", () => expect(0).toBeFalsy());
            test("toBeFalsy fail", () => expect(1).toBeFalsy());
            test("toBeFalsy not pass", () => expect(1).not.toBeFalsy());
            test("toBeFalsy not fail", () => expect(0).not.toBeFalsy());

            test("toContain pass string", () => expect("hello world").toContain("world"));
            test("toContain pass array", () => expect([1, 2, 3]).toContain(2));
            test("toContain fail", () => expect("hello").toContain("bye"));
            test("toContain not pass", () => expect("hello").not.toContain("bye"));
            test("toContain not fail", () => expect("hello").not.toContain("hell"));

            test("toMatch pass regex", () => expect("hello123").toMatch(/[0-9]+/));
            test("toMatch pass string", () => expect("hello123").toMatch("[0-9]+"));
            test("toMatch fail", () => expect("hello").toMatch(/[0-9]+/));
            test("toMatch not pass", () => expect("hello").not.toMatch(/[0-9]+/));
            test("toMatch not fail", () => expect("hello123").not.toMatch(/[0-9]+/));

            test("toBeGreaterThan pass", () => expect(5).toBeGreaterThan(3));
            test("toBeGreaterThan fail", () => expect(3).toBeGreaterThan(5));
            test("toBeGreaterThan not pass", () => expect(3).not.toBeGreaterThan(5));
            test("toBeGreaterThan not fail", () => expect(5).not.toBeGreaterThan(3));

            test("toBeLessThan pass", () => expect(3).toBeLessThan(5));
            test("toBeLessThan fail", () => expect(5).toBeLessThan(3));
            test("toBeLessThan not pass", () => expect(5).not.toBeLessThan(3));
            test("toBeLessThan not fail", () => expect(3).not.toBeLessThan(5));

            test("toHaveProperty pass", () => expect({ a: 1 }).toHaveProperty("a"));
            test("toHaveProperty fail", () => expect({ a: 1 }).toHaveProperty("b"));
            test("toHaveProperty not pass", () => expect({ a: 1 }).not.toHaveProperty("b"));
            test("toHaveProperty not fail", () => expect({ a: 1 }).not.toHaveProperty("a"));
        "#;
        let outcome = engine()
            .run_pre(script, pre_ctx())
            .expect("script runs fine");
        for name in [
            "toBe pass",
            "toBe not pass",
            "toEqual pass",
            "toEqual not pass",
            "toBeTruthy pass",
            "toBeTruthy not pass",
            "toBeFalsy pass",
            "toBeFalsy not pass",
            "toContain pass string",
            "toContain pass array",
            "toContain not pass",
            "toMatch pass regex",
            "toMatch pass string",
            "toMatch not pass",
            "toBeGreaterThan pass",
            "toBeGreaterThan not pass",
            "toBeLessThan pass",
            "toBeLessThan not pass",
            "toHaveProperty pass",
            "toHaveProperty not pass",
        ] {
            let result = find(&outcome.tests, name);
            assert!(result.passed, "expected {name:?} to pass, got {result:?}");
        }
        for name in [
            "toBe fail",
            "toBe not fail",
            "toEqual fail",
            "toEqual not fail",
            "toBeTruthy fail",
            "toBeTruthy not fail",
            "toBeFalsy fail",
            "toBeFalsy not fail",
            "toContain fail",
            "toContain not fail",
            "toMatch fail",
            "toMatch not fail",
            "toBeGreaterThan fail",
            "toBeGreaterThan not fail",
            "toBeLessThan fail",
            "toBeLessThan not fail",
            "toHaveProperty fail",
            "toHaveProperty not fail",
        ] {
            let result = find(&outcome.tests, name);
            assert!(!result.passed, "expected {name:?} to fail, got {result:?}");
            assert!(result.message.is_some(), "{name:?} should carry a message");
        }
    }

    // --- test isolation --------------------------------------------------------------------------

    #[test]
    fn a_failing_test_does_not_stop_the_script() {
        let script = r#"
            test("first throws", () => { throw new Error("boom"); });
            test("second runs", () => expect(1).toBe(1));
            vars.set("reached", "yes");
        "#;
        let outcome = engine()
            .run_pre(script, pre_ctx())
            .expect("script runs fine");
        let first = find(&outcome.tests, "first throws");
        assert!(!first.passed);
        assert_eq!(first.message.as_deref(), Some("boom"));
        assert!(find(&outcome.tests, "second runs").passed);
        assert_eq!(
            outcome
                .vars
                .iter()
                .find(|kv| kv.key == "reached")
                .map(|kv| kv.value.as_str()),
            Some("yes")
        );
    }

    // --- vars / env round-trip -----------------------------------------------------------------

    #[test]
    fn vars_round_trip_through_get_and_set() {
        let mut ctx = pre_ctx();
        ctx.vars.push(KeyValue::new("existing", "0"));
        let script = r#"
            vars.set("seen", vars.get("existing"));
            vars.set("a", "1");
            vars.set("a", "2");
        "#;
        let outcome = engine().run_pre(script, ctx).expect("script runs fine");
        let get = |key: &str| {
            outcome
                .vars
                .iter()
                .find(|kv| kv.key == key)
                .map(|kv| kv.value.clone())
        };
        assert_eq!(get("existing"), Some("0".to_string()));
        assert_eq!(get("seen"), Some("0".to_string()));
        assert_eq!(get("a"), Some("2".to_string()));
    }

    #[test]
    fn env_set_and_unset_are_recorded_in_order() {
        let mut ctx = pre_ctx();
        ctx.env.push(KeyValue::new("w", "present"));
        let script = r#"
            env.set("x", "y");
            env.set("x", "z");
            env.unset("w");
        "#;
        let outcome = engine().run_pre(script, ctx).expect("script runs fine");
        assert_eq!(
            outcome.env_changes,
            vec![
                EnvChange::Set {
                    key: "x".to_string(),
                    value: "y".to_string()
                },
                EnvChange::Set {
                    key: "x".to_string(),
                    value: "z".to_string()
                },
                EnvChange::Unset {
                    key: "w".to_string()
                },
            ]
        );
    }

    // --- request mutation in pre, read-only in post ---------------------------------------------

    #[test]
    fn pre_script_can_mutate_the_request() {
        let mut ctx = pre_ctx();
        ctx.request.headers.push(KeyValue::new("X-Old", "gone"));
        ctx.request.headers.push(KeyValue::new("X-Keep", "kept"));
        let script = r#"
            req.method = "POST";
            req.url = "https://example.com/changed";
            req.headers.set("X-New", "1");
            req.headers.remove("X-Old");
            req.body = "new body";
        "#;
        let outcome = engine().run_pre(script, ctx).expect("script runs fine");
        assert_eq!(outcome.request.method, "POST");
        assert_eq!(outcome.request.url, "https://example.com/changed");
        assert_eq!(outcome.request.body, "new body");
        assert_eq!(
            outcome.request.headers,
            vec![KeyValue::new("X-Keep", "kept"), KeyValue::new("X-New", "1")]
        );
    }

    #[test]
    fn request_header_names_are_case_insensitive_in_pre() {
        let mut ctx = pre_ctx();
        ctx.request.headers.push(KeyValue::new("X-Old", "gone"));
        ctx.request
            .headers
            .push(KeyValue::new("Content-Type", "text/plain"));
        let script = r#"
            req.headers.set("content-type", "application/json");
            req.headers.remove("X-OLD");
            vars.set("seen", req.headers.get("CONTENT-TYPE"));
        "#;
        let outcome = engine().run_pre(script, ctx).expect("script runs fine");
        // The existing entry keeps its spelling and gets the new value, with no duplicate.
        assert_eq!(
            outcome.request.headers,
            vec![KeyValue::new("Content-Type", "application/json")]
        );
        assert_eq!(
            outcome.vars,
            vec![KeyValue::new("seen", "application/json")]
        );
    }

    #[test]
    fn response_header_lookups_are_case_insensitive() {
        let mut ctx = post_ctx("{}");
        ctx.response
            .headers
            .push(KeyValue::new("content-type", "application/json"));
        let script = r#"
            test("mixed case", () => expect(res.headers.get("Content-Type")).toBe("application/json"));
            test("upper case", () => expect(res.headers.get("CONTENT-TYPE")).toBe("application/json"));
            test("missing", () => expect(res.headers.get("X-Missing")).toBeFalsy());
        "#;
        let outcome = engine()
            .run_post(script, ctx)
            .expect("run_post never fails for a plain script error");
        assert!(
            outcome.tests.iter().all(|test| test.passed),
            "{:?}",
            outcome.tests
        );
    }

    #[test]
    fn variable_and_env_names_stay_case_sensitive() {
        let script = r#"
            vars.set("Name", "a");
            env.set("Token", "b");
            test("vars", () => expect(vars.get("name")).toBeFalsy());
            test("env", () => expect(env.get("token")).toBeFalsy());
        "#;
        let outcome = engine()
            .run_post(script, post_ctx("{}"))
            .expect("run_post never fails for a plain script error");
        assert!(
            outcome.tests.iter().all(|test| test.passed),
            "{:?}",
            outcome.tests
        );
    }

    fn base_response(body: &str) -> ScriptResponse {
        ScriptResponse {
            status: 200,
            headers: Vec::new(),
            body: body.to_string(),
            time_ms: 12,
            size: body.len(),
        }
    }

    fn post_ctx(body: &str) -> PostContext {
        PostContext {
            request: base_request(),
            response: base_response(body),
            vars: Vec::new(),
            env: Vec::new(),
        }
    }

    #[test]
    fn req_and_res_headers_are_read_only_in_post() {
        let script = r#"
            let methodThrew = false;
            try { req.method = "PUT"; } catch (e) { methodThrew = true; }
            let headersThrew = false;
            try { req.headers.set("X-New", "1"); } catch (e) { headersThrew = true; }
            let resHeadersThrew = false;
            try { res.headers.set("X-New", "1"); } catch (e) { resHeadersThrew = true; }
            test("req.method is read-only", () => expect(methodThrew).toBeTruthy());
            test("req.headers is read-only", () => expect(headersThrew).toBeTruthy());
            test("res.headers is read-only", () => expect(resHeadersThrew).toBeTruthy());
        "#;
        let outcome = engine()
            .run_post(script, post_ctx("{}"))
            .expect("run_post never fails for a plain script error");
        assert!(outcome.script_error.is_none());
        assert!(find(&outcome.tests, "req.method is read-only").passed);
        assert!(find(&outcome.tests, "req.headers is read-only").passed);
        assert!(find(&outcome.tests, "res.headers is read-only").passed);
    }

    // --- res.json() ------------------------------------------------------------------------------

    #[test]
    fn res_json_parses_a_valid_body() {
        let script = r#"test("json body", () => expect(res.json().id).toBe(42));"#;
        let outcome = engine()
            .run_post(script, post_ctx(r#"{"id":42}"#))
            .expect("run_post never fails for a plain script error");
        assert!(find(&outcome.tests, "json body").passed);
    }

    #[test]
    fn res_json_throws_on_invalid_json() {
        let script = r#"
            let threw = false;
            try { res.json(); } catch (e) { threw = true; }
            test("invalid json throws", () => expect(threw).toBeTruthy());
        "#;
        let outcome = engine()
            .run_post(script, post_ctx("not json"))
            .expect("run_post never fails for a plain script error");
        assert!(find(&outcome.tests, "invalid json throws").passed);
    }

    // --- console capture ---------------------------------------------------------------------------

    #[test]
    fn console_calls_are_captured_in_order() {
        let script = r#"
            console.log("hello", 42);
            console.info("info line");
            console.warn("warn line");
            console.error("error line");
            console.log({ a: 1 });
        "#;
        let outcome = engine()
            .run_pre(script, pre_ctx())
            .expect("script runs fine");
        assert_eq!(outcome.console.len(), 5);
        assert_eq!(outcome.console[0].level, ConsoleLevel::Log);
        assert_eq!(outcome.console[0].text, "hello 42");
        assert_eq!(outcome.console[1].level, ConsoleLevel::Info);
        assert_eq!(outcome.console[2].level, ConsoleLevel::Warn);
        assert_eq!(outcome.console[3].level, ConsoleLevel::Error);
        assert_eq!(outcome.console[4].text, r#"{"a":1}"#);
    }

    #[test]
    fn console_lines_carry_their_stage_time_and_script_line() {
        let before = std::time::SystemTime::now() - Duration::from_secs(1);
        let script = "const a = 1;\n\nconsole.log(a);\nfunction f() { console.warn('x'); }\nf();";
        let pre = engine()
            .run_pre(script, pre_ctx())
            .expect("script runs fine");
        let post = engine()
            .run_post(script, post_ctx("{}"))
            .expect("script runs fine");

        assert_eq!(pre.console[0].stage, ScriptStage::Pre);
        assert_eq!(post.console[0].stage, ScriptStage::Post);
        assert_eq!(pre.console[0].line, Some(3));
        assert_eq!(pre.console[1].line, Some(4));
        assert_eq!(post.console[1].line, Some(4));
        let time = pre.console[0].time.expect("a time");
        assert!(time >= before && time <= std::time::SystemTime::now());
    }

    // --- util functions ----------------------------------------------------------------------------

    #[test]
    fn util_functions_produce_the_right_shapes() {
        let script = r#"
            test("uuid format", () => expect(util.uuid()).toMatch(/^[0-9a-f-]{36}$/));
            test("now is a number", () => expect(typeof util.now()).toBe("number"));
            test("isoDate format", () => expect(util.isoDate()).toMatch(/^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$/));
            test("randomInt stays in range", () => {
                const n = util.randomInt(5, 10);
                expect(n >= 5 && n <= 10).toBeTruthy();
            });
            test("randomString has the right length", () => expect(util.randomString(12).length).toBe(12));
            test("base64 round trips", () => expect(util.base64Decode(util.base64Encode("hi"))).toBe("hi"));
            test("urlEncode escapes spaces", () => expect(util.urlEncode("a b")).toBe("a%20b"));
        "#;
        let outcome = engine()
            .run_pre(script, pre_ctx())
            .expect("script runs fine");
        for test in &outcome.tests {
            assert!(test.passed, "{} failed: {:?}", test.name, test.message);
        }
        assert_eq!(outcome.tests.len(), 7);
    }

    #[test]
    fn util_base64_decode_reports_the_same_error_as_the_template_layer() {
        let script = r#"
            let message = null;
            try { util.base64Decode("not base64!!"); } catch (e) { message = e.message; }
            vars.set("message", message);
        "#;
        let outcome = engine()
            .run_pre(script, pre_ctx())
            .expect("script runs fine");
        let message = outcome
            .vars
            .iter()
            .find(|kv| kv.key == "message")
            .map(|kv| kv.value.clone())
            .expect("message var must be set");
        let expected = brasa_core::functions::call(
            "base64Decode",
            &[brasa_core::Arg::Str("not base64!!".to_string())],
        )
        .unwrap_err()
        .to_string();
        assert_eq!(message, expected);
    }

    // --- syntax and runtime errors ---------------------------------------------------------------

    #[test]
    fn a_syntax_error_is_reported_with_a_line_number() {
        let result = engine().run_pre("let x = ;", pre_ctx());
        match result {
            Err(ScriptError::Syntax { line, .. }) => assert_eq!(line, Some(1)),
            other => panic!("expected a syntax error, got {other:?}"),
        }
    }

    #[test]
    fn pre_uncaught_exception_aborts_and_is_returned_as_an_error() {
        let result = engine().run_pre("null.foo;", pre_ctx());
        assert!(
            matches!(result, Err(ScriptError::Runtime { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn post_uncaught_exception_is_reported_but_does_not_fail_the_call() {
        let script = r#"
            console.log("before");
            test("t1", () => expect(1).toBe(1));
            null.foo;
            console.log("after");
        "#;
        let outcome = engine()
            .run_post(script, post_ctx("{}"))
            .expect("run_post never fails for a plain script error");
        assert!(matches!(
            outcome.script_error,
            Some(ScriptError::Runtime { .. })
        ));
        assert_eq!(outcome.console.len(), 1);
        assert_eq!(outcome.console[0].text, "before");
        assert!(find(&outcome.tests, "t1").passed);
    }

    // --- resource limits -----------------------------------------------------------------------

    #[test]
    fn an_infinite_loop_is_killed_by_the_timeout() {
        let start = std::time::Instant::now();
        let result = engine().run_pre("while (true) {}", pre_ctx());
        let elapsed = start.elapsed();
        assert!(matches!(result, Err(ScriptError::Timeout)), "{result:?}");
        assert!(elapsed < Duration::from_secs(10), "took {elapsed:?}");
    }

    #[test]
    fn a_memory_bomb_is_stopped_by_the_memory_limit() {
        let script = "let arr = []; while (true) { arr.push('x'.repeat(1000)); }";
        let result = engine().run_pre(script, pre_ctx());
        assert!(
            matches!(result, Err(ScriptError::MemoryLimit)),
            "{result:?}"
        );
    }

    // --- sandboxing ----------------------------------------------------------------------------

    #[test]
    fn the_sandbox_has_no_fetch_require_import_or_settimeout() {
        for script in [
            "fetch('https://example.com');",
            "require('fs');",
            "import x from 'y';",
            "setTimeout(() => {}, 0);",
        ] {
            let result = engine().run_pre(script, pre_ctx());
            assert!(
                result.is_err(),
                "expected {script:?} to fail, got {result:?}"
            );
        }
    }

    // --- isolation between runs ------------------------------------------------------------------

    #[test]
    fn no_state_is_shared_between_two_runs() {
        let script = "globalThis.leaked = (globalThis.leaked || 0) + 1; vars.set('counter', globalThis.leaked);";
        let counter = |outcome: &PreOutcome| {
            outcome
                .vars
                .iter()
                .find(|kv| kv.key == "counter")
                .map(|kv| kv.value.clone())
                .expect("counter var must be set")
        };
        let first = engine()
            .run_pre(script, pre_ctx())
            .expect("script runs fine");
        let second = engine()
            .run_pre(script, pre_ctx())
            .expect("script runs fine");
        assert_eq!(counter(&first), "1");
        assert_eq!(counter(&second), "1");
    }

    // --- empty scripts are a pass-through -------------------------------------------------------

    #[test]
    fn an_empty_pre_script_passes_the_request_and_vars_through() {
        let mut ctx = pre_ctx();
        ctx.vars.push(KeyValue::new("a", "1"));
        let outcome = engine()
            .run_pre("", ctx.clone())
            .expect("empty script never fails");
        assert_eq!(outcome.request, ctx.request);
        assert_eq!(outcome.vars, ctx.vars);
    }
}
