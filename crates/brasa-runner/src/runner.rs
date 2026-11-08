//! [`Runner`], the request execution pipeline (see `docs/architecture.md`): build the variable
//! scope, run the pre script, interpolate, send the request, run the post script, and collect a
//! complete [`RunResult`] for the UI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use brasa_core::{Environment, KeyValue, Request, ResolvedRequest, Response, VarScope};
use brasa_http::{Client, HttpError, SendOptions};
use brasa_script::{PostContext, PreContext, ScriptEngine};

use crate::resolve::{MultipartFiles, resolve};
use crate::result::{FailedStage, RunResult};
use crate::script_bridge::{
    apply_script_request, resolved_to_script_request, to_script_request, to_script_response,
};
use crate::session_env::SessionEnv;

/// Runs a [`Request`] end to end: pre script, interpolation, send, post script.
///
/// A `Runner` is cheap to keep around: it only holds the [`ScriptEngine`] implementation and the
/// [`SendOptions`] every request is sent with. Create one per application (or one per test) and
/// call [`Runner::run`] for every request.
pub struct Runner {
    engine: Arc<dyn ScriptEngine>,
    options: SendOptions,
    /// Where the relative paths of multipart files start: the workspace.
    base_dir: Option<PathBuf>,
}

impl Runner {
    /// Creates a runner that uses `engine` for `::: pre`/`::: post` scripts and sends requests
    /// with `options`.
    #[must_use]
    pub fn new(engine: Arc<dyn ScriptEngine>, options: SendOptions) -> Self {
        Self {
            engine,
            options,
            base_dir: None,
        }
    }

    /// Makes the relative paths of multipart files start at `base_dir`, the workspace folder.
    #[must_use]
    pub fn with_base_dir(mut self, base_dir: Option<PathBuf>) -> Self {
        self.base_dir = base_dir;
        self
    }

    /// Runs `request` against `environment`, applying any environment changes the scripts make
    /// to `session_env`. The request is sent on a new connection, closed afterwards.
    ///
    /// This never fails: every kind of failure (a pre script exception, a send error, a post
    /// script exception) is reported inside the returned [`RunResult`] instead of a top-level
    /// `Result` (see [`RunResult::failed_stage`]), alongside whatever was collected before the
    /// failure.
    pub fn run(
        &self,
        request: &Request,
        environment: &Environment,
        session_env: &mut SessionEnv,
    ) -> RunResult {
        self.execute(request, environment, session_env, |resolved| {
            brasa_http::send(resolved, &self.options)
        })
    }

    /// A [`Client`] that sends with this runner's options, for [`Self::run_with`].
    #[must_use]
    pub fn client(&self) -> Client {
        Client::new(self.options.clone())
    }

    /// [`Self::run`], sending the request through `client`, which reuses its open connections
    /// (keep-alive). `client` should come from [`Self::client`], so it has the same options.
    pub fn run_with(
        &self,
        request: &Request,
        environment: &Environment,
        session_env: &mut SessionEnv,
        client: &Client,
    ) -> RunResult {
        self.execute(request, environment, session_env, |resolved| {
            client.send(resolved)
        })
    }

    /// The pipeline behind [`Self::run`] and [`Self::run_with`], which only differ in `send`.
    fn execute(
        &self,
        request: &Request,
        environment: &Environment,
        session_env: &mut SessionEnv,
        send: impl FnOnce(&ResolvedRequest) -> Result<Response, HttpError>,
    ) -> RunResult {
        let started_at = SystemTime::now();
        let mut console = Vec::new();
        let mut tests = Vec::new();

        // 1. Pre script, skipped if empty or turned off.
        let mut working_request = request.clone();
        let mut request_vars: Vec<KeyValue> = Vec::new();
        if request.pre_enabled && !request.pre_script.trim().is_empty() {
            let ctx = PreContext {
                request: to_script_request(&working_request),
                vars: Vec::new(),
                env: session_env.merged_with(environment),
            };
            let started = Instant::now();
            let outcome = self.engine.run_pre(&request.pre_script, ctx);
            log::trace!(
                "pre-request script ran in {} us",
                started.elapsed().as_micros()
            );
            match outcome {
                Err(error) => {
                    // An uncaught exception in a pre script aborts the run: nothing is sent.
                    return RunResult {
                        resolved_request: None,
                        response: None,
                        tests,
                        console,
                        warnings: Vec::new(),
                        failed_stage: Some(FailedStage::Pre(error)),
                        started_at,
                        sent_at: None,
                        finished_at: SystemTime::now(),
                    };
                }
                Ok(outcome) => {
                    working_request = apply_script_request(&working_request, outcome.request);
                    request_vars = outcome.vars;
                    session_env.apply(&outcome.env_changes);
                    console.extend(outcome.console);
                    tests.extend(outcome.tests);
                }
            }
        }

        // 2. Interpolate into a ResolvedRequest. Never fatal: unresolved markers become warnings.
        let scope = VarScope {
            request_vars: &request_vars,
            session_env: session_env.as_slice(),
            environment: &environment.variables,
        };
        let started = Instant::now();
        let (resolved, warnings, file_error) = resolve(
            &working_request,
            &scope,
            MultipartFiles::Read(self.base_dir.as_deref()),
        );
        log::trace!(
            "interpolated in {} us, {} warnings",
            started.elapsed().as_micros(),
            warnings.len()
        );

        // 3. Send.
        let sent_at = SystemTime::now();
        let sent = match file_error {
            Some(error) => Err(HttpError::InvalidRequest(error)),
            None => send(&resolved),
        };
        let response = match sent {
            Ok(response) => response,
            Err(error) => {
                return RunResult {
                    resolved_request: Some(resolved),
                    response: None,
                    tests,
                    console,
                    warnings,
                    failed_stage: Some(FailedStage::Send(error)),
                    started_at,
                    sent_at: Some(sent_at),
                    finished_at: SystemTime::now(),
                };
            }
        };

        // 4. Post script, skipped if empty or turned off.
        if request.post_enabled && !request.post_script.trim().is_empty() {
            let ctx = PostContext {
                request: resolved_to_script_request(&resolved),
                response: to_script_response(&response),
                vars: request_vars,
                env: session_env.merged_with(environment),
            };
            let started = Instant::now();
            let outcome = self.engine.run_post(&request.post_script, ctx);
            log::trace!(
                "post-response script ran in {} us",
                started.elapsed().as_micros()
            );
            match outcome {
                Err(error) => {
                    // An engine-level failure (timeout, memory limit, ...): unlike a plain
                    // script exception, there is no PostOutcome to read console lines or tests
                    // from.
                    return RunResult {
                        resolved_request: Some(resolved),
                        response: Some(response),
                        tests,
                        console,
                        warnings,
                        failed_stage: Some(FailedStage::Post(error)),
                        started_at,
                        sent_at: Some(sent_at),
                        finished_at: SystemTime::now(),
                    };
                }
                Ok(outcome) => {
                    session_env.apply(&outcome.env_changes);
                    console.extend(outcome.console);
                    tests.extend(outcome.tests);
                    if let Some(script_error) = outcome.script_error {
                        // A plain uncaught exception: the response is still shown, but the
                        // failure is reported alongside whatever ran before it was thrown.
                        return RunResult {
                            resolved_request: Some(resolved),
                            response: Some(response),
                            tests,
                            console,
                            warnings,
                            failed_stage: Some(FailedStage::Post(script_error)),
                            started_at,
                            sent_at: Some(sent_at),
                            finished_at: SystemTime::now(),
                        };
                    }
                }
            }
        }

        RunResult {
            resolved_request: Some(resolved),
            response: Some(response),
            tests,
            console,
            warnings,
            failed_stage: None,
            started_at,
            sent_at: Some(sent_at),
            finished_at: SystemTime::now(),
        }
    }
}
