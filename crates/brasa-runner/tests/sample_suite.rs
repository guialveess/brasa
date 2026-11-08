//! Runs the whole sample suite (`samples/workspace`) against the local sample server
//! (`samples/server/server.py`), through the real `Runner`, `QuickJsEngine` and `ureq`.
//! See `docs/sample-suite.md`.
//!
//! The server is started with `--ephemeral`, so every test gets its own free ports, and it only
//! listens on `127.0.0.1`: nothing here touches the internet. The tests need Python 3 (standard
//! library only) and are skipped, loudly, when it is not installed.
#![allow(clippy::expect_used)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use brasa_core::{Environment, KeyValue};
use brasa_http::{HttpError, InvalidCertificates, SendOptions};
use brasa_runner::{FailedStage, RunResult, Runner, SessionEnv};
use brasa_script::QuickJsEngine;
use brasa_workspace::{Node, RequestEntry, Workspace};

/// The default port of every listener, as written in the sample environments. The server is
/// started on ephemeral ports, so these are rewritten to the real ones before running.
const DEFAULT_PORTS: [(&str, u16); 6] = [
    ("http", 8080),
    ("self-signed", 8443),
    ("expired", 8444),
    ("not-yet-valid", 8445),
    ("wrong-host", 8446),
    ("private-ca", 8447),
];

fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// A running `server.py`, killed when dropped.
struct SampleServer {
    child: Child,
    ports: Vec<(String, u16)>,
}

impl SampleServer {
    /// Starts the server, or returns `None` when no Python interpreter is available.
    fn start() -> Option<SampleServer> {
        let script = samples_dir().join("server/server.py");
        let (mut child, name) = ["python3", "python"].iter().find_map(|name| {
            let child = Command::new(name)
                .arg("-u")
                .arg(&script)
                .arg("--ephemeral")
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .ok()?;
            Some((child, *name))
        })?;
        let stdout = child.stdout.take().expect("piped stdout");
        let mut ports = None;
        for line in BufReader::new(stdout).lines() {
            let line = line.expect("readable server output");
            if let Some(json) = line.strip_prefix("READY ") {
                let value: serde_json::Value = serde_json::from_str(json).expect("READY json");
                ports = Some(
                    value
                        .as_object()
                        .expect("READY object")
                        .iter()
                        .map(|(name, port)| {
                            let port = port.as_u64().expect("numeric port");
                            (name.clone(), u16::try_from(port).expect("port in range"))
                        })
                        .collect::<Vec<_>>(),
                );
                break;
            }
        }
        let ports = ports.unwrap_or_else(|| panic!("`{name}` exited before the server was ready"));
        Some(SampleServer { child, ports })
    }

    fn port(&self, profile: &str) -> u16 {
        self.ports
            .iter()
            .find(|(name, _)| name == profile)
            .map(|(_, port)| *port)
            .expect("known profile")
    }

    /// The sample environment `name`, with every default port replaced by the real one.
    fn environment(&self, workspace: &Workspace, name: &str) -> Environment {
        let mut environment = workspace
            .load_environment(name)
            .expect("sample environment");
        for KeyValue { value, .. } in &mut environment.variables {
            for (profile, default) in DEFAULT_PORTS {
                let suffix = format!(":{default}");
                if let Some(host) = value.strip_suffix(&suffix) {
                    *value = format!("{host}:{}", self.port(profile));
                }
            }
        }
        environment
    }
}

impl Drop for SampleServer {
    fn drop(&mut self) {
        // The process may have exited already, nothing to report in that case.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts the server or skips the calling test.
macro_rules! server_or_skip {
    () => {
        match SampleServer::start() {
            Some(server) => server,
            None => {
                eprintln!("SKIPPED: Python 3 is not installed, the sample suite was not run");
                return;
            }
        }
    };
}

fn runner(invalid_certificates: InvalidCertificates) -> Runner {
    Runner::new(
        Arc::new(QuickJsEngine),
        SendOptions {
            invalid_certificates,
            ..SendOptions::default()
        },
    )
    .with_base_dir(Some(samples_dir().join("workspace")))
}

/// Every request of the tree in sidebar order, folders first.
fn requests(nodes: &[Node]) -> Vec<&RequestEntry> {
    let mut found = Vec::new();
    for node in nodes {
        match node {
            Node::Folder(folder) => found.extend(requests(&folder.children)),
            Node::Request(request) => found.push(request),
        }
    }
    found
}

fn open_workspace() -> Workspace {
    Workspace::open(samples_dir().join("workspace")).expect("sample workspace")
}

/// What a request of the suite is expected to do.
enum Expect {
    /// Runs to the end, every test green, no unresolved variable.
    Pass,
    /// Runs to the end with exactly one unresolved variable.
    Unresolved,
    /// Runs to the end with `failed` red tests out of `total`.
    FailedTests { total: usize, failed: usize },
    /// The pre script throws, nothing is sent.
    PreFails,
    /// Sending fails.
    SendFails,
    /// The post script throws, the response is kept.
    PostFails,
}

fn expectation(id: &str) -> Expect {
    match id {
        "variables/unresolved.brasa" => Expect::Unresolved,
        "scripting/test-failure-does-not-stop.brasa" => Expect::FailedTests {
            total: 3,
            failed: 1,
        },
        "expected-failures/failing-test.brasa" => Expect::FailedTests {
            total: 1,
            failed: 1,
        },
        "expected-failures/pre-script-throws.brasa"
        | "expected-failures/pre-script-syntax-error.brasa" => Expect::PreFails,
        "expected-failures/connection-refused.brasa"
        | "expected-failures/dropped-connection.brasa"
        | "expected-failures/too-many-redirects.brasa"
        | "expected-failures/multipart-missing-file.brasa"
        | "expected-failures/invalid-url.brasa" => Expect::SendFails,
        "expected-failures/post-script-throws.brasa"
        | "expected-failures/post-json-parse.brasa"
        | "expected-failures/post-readonly-req.brasa" => Expect::PostFails,
        _ => Expect::Pass,
    }
}

/// Why `result` does not match `expect`, if it does not.
fn mismatch(expect: &Expect, result: &RunResult) -> Option<String> {
    let red: Vec<String> = result
        .tests
        .iter()
        .filter(|test| !test.passed)
        .map(|test| {
            format!(
                "{}: {}",
                test.name,
                test.message.clone().unwrap_or_default()
            )
        })
        .collect();
    match expect {
        Expect::Pass | Expect::Unresolved => {
            if let Some(stage) = &result.failed_stage {
                return Some(format!("failed: {stage}"));
            }
            if !red.is_empty() {
                return Some(format!("red tests: {red:?}"));
            }
            if result.tests.is_empty() {
                return Some("has no test, every request of the suite checks itself".into());
            }
            let wanted = usize::from(matches!(expect, Expect::Unresolved));
            (result.warnings.len() != wanted).then(|| format!("warnings: {:?}", result.warnings))
        }
        Expect::FailedTests { total, failed } => {
            if let Some(stage) = &result.failed_stage {
                return Some(format!("failed: {stage}"));
            }
            (result.tests.len() != *total || red.len() != *failed)
                .then(|| format!("expected {failed} red of {total}, got {red:?}"))
        }
        Expect::PreFails => (!matches!(result.failed_stage, Some(FailedStage::Pre(_))
            if result.response.is_none() && result.resolved_request.is_none()))
        .then(|| format!("expected a pre failure, got {:?}", result.failed_stage)),
        Expect::SendFails => (!matches!(result.failed_stage, Some(FailedStage::Send(_)))
            || result.response.is_some())
        .then(|| format!("expected a send failure, got {:?}", result.failed_stage)),
        Expect::PostFails => (!matches!(result.failed_stage, Some(FailedStage::Post(_)))
            || result.response.is_none())
        .then(|| format!("expected a post failure, got {:?}", result.failed_stage)),
    }
}

/// Whether `id` never goes through the HTTPS listener of the `secure` environment: the `tls/`
/// requests use their own URLs, and a URL without a scheme is sent over plain HTTP.
fn plain_http_only(id: &str) -> bool {
    id.starts_with("tls/") || id == "variables/host-without-scheme.brasa"
}

/// The text a certificate warning must contain, for the `tls/` requests.
fn tls_reason(id: &str) -> Option<&'static str> {
    match id {
        // The verifier checks the issuer before the host name, so a certificate for another
        // host that nobody trusts is reported as an unknown issuer.
        "tls/self-signed.brasa"
        | "tls/post-with-body.brasa"
        | "tls/private-ca.brasa"
        | "tls/wrong-host.brasa" => Some("unknown issuer"),
        "tls/expired.brasa" => Some("expired"),
        "tls/not-yet-valid.brasa" => Some("not valid yet"),
        _ => None,
    }
}

/// Runs the whole workspace once against the environment `name`, in sidebar order and with one
/// shared session, and returns every problem found.
fn run_suite(server: &SampleServer, name: &str, secure: bool) -> Vec<String> {
    let workspace = open_workspace();
    let environment = server.environment(&workspace, name);
    let runner = runner(InvalidCertificates::SendWithWarning);
    let mut session = SessionEnv::new();
    let mut problems = Vec::new();
    let entries = requests(workspace.tree());
    assert!(entries.len() > 100, "found only {} requests", entries.len());

    for entry in entries {
        let request = workspace.load_request(&entry.id).expect("loadable request");
        let result = runner.run(&request, &environment, &mut session);
        let expect = expectation(&entry.id);
        if let Some(problem) = mismatch(&expect, &result) {
            problems.push(format!("[{name}] {}: {problem}", entry.id));
            continue;
        }
        let Some(response) = &result.response else {
            continue;
        };
        let warning = response.tls_warning.as_deref();
        match (tls_reason(&entry.id), warning) {
            (Some(reason), Some(text)) if text.contains(reason) => {}
            (Some(reason), other) => problems.push(format!(
                "[{name}] {}: expected a certificate warning about \"{reason}\", got {other:?}",
                entry.id
            )),
            (None, Some(text)) if !secure => {
                problems.push(format!("[{name}] {}: unexpected warning {text}", entry.id));
            }
            (None, None) if secure && !plain_http_only(&entry.id) => {
                problems.push(format!(
                    "[{name}] {}: expected a certificate warning",
                    entry.id
                ));
            }
            _ => {}
        }
    }
    problems
}

#[test]
fn the_suite_passes_over_plain_http() {
    let server = server_or_skip!();
    let problems = run_suite(&server, "local", false);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_suite_passes_over_https_with_a_certificate_warning() {
    let server = server_or_skip!();
    let problems = run_suite(&server, "secure", true);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every request of the `tls/` folder, run with `invalid_certificates`.
fn run_tls_folder(
    server: &SampleServer,
    invalid_certificates: InvalidCertificates,
) -> Vec<(String, RunResult)> {
    let workspace = open_workspace();
    let environment = server.environment(&workspace, "local");
    let runner = runner(invalid_certificates);
    let mut session = SessionEnv::new();
    let results: Vec<_> = requests(workspace.tree())
        .into_iter()
        .filter(|entry| entry.id.starts_with("tls/"))
        .map(|entry| {
            let request = workspace.load_request(&entry.id).expect("loadable request");
            (
                entry.id.clone(),
                runner.run(&request, &environment, &mut session),
            )
        })
        .collect();
    assert_eq!(results.len(), 6);
    results
}

#[test]
fn rejecting_invalid_certificates_fails_every_tls_request() {
    let server = server_or_skip!();
    for (id, result) in run_tls_folder(&server, InvalidCertificates::Reject) {
        assert!(
            matches!(
                result.failed_stage,
                Some(FailedStage::Send(HttpError::Certificate(_)))
            ),
            "{id}: {:?}",
            result.failed_stage
        );
        assert!(result.response.is_none(), "{id}: nothing must be sent");
    }
}

#[test]
fn accepting_invalid_certificates_passes_without_a_warning() {
    let server = server_or_skip!();
    for (id, result) in run_tls_folder(&server, InvalidCertificates::Accept) {
        assert!(
            result.failed_stage.is_none(),
            "{id}: {:?}",
            result.failed_stage
        );
        assert!(
            result.tests.iter().all(|test| test.passed),
            "{id}: red test"
        );
        let response = result.response.expect("a response");
        assert_eq!(response.tls_warning, None, "{id}");
    }
}
