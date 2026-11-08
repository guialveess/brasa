//! End-to-end tests for `brasa_runner::Runner`, against a local `tiny_http` server and the
//! real `QuickJsEngine`. See `docs/architecture.md` for the pipeline.
//!
//! No test here ever touches the internet: every server is bound to `127.0.0.1` on an
//! OS-assigned port by `brasa_http::test_support::TestServer`.
// `expect()` is the normal way to fail a test with a clear message; allowed crate-wide since
// this whole file is test code.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use brasa_core::{Auth, Body, Environment, KeyValue, Method, Request, TemplateWarning};
use brasa_http::SendOptions;
use brasa_http::test_support::{CapturedRequest, StubResponse, TestServer};
use brasa_runner::{FailedStage, Runner, SessionEnv};
use brasa_script::QuickJsEngine;
use pretty_assertions::assert_eq;

/// Starts a server that answers every request with `respond_with`, and returns it together with
/// a channel that yields one [`CapturedRequest`] per request received, in order, and the
/// server's background thread handle.
fn start_server<F>(respond_with: F) -> (TestServer, mpsc::Receiver<CapturedRequest>, JoinHandle<()>)
where
    F: Fn(&CapturedRequest) -> StubResponse + Send + 'static,
{
    let server = TestServer::start();
    let (tx, rx) = mpsc::channel();
    let handle = server.serve(move |captured| {
        let response = respond_with(&captured);
        // A dropped receiver here (a test that stops listening early) is not a bug in the
        // server.
        let _ = tx.send(captured);
        response
    });
    (server, rx, handle)
}

/// A bare, enabled [`Runner`] backed by the real `QuickJsEngine`, sending with default
/// [`SendOptions`].
fn runner() -> Runner {
    Runner::new(Arc::new(QuickJsEngine), SendOptions::default())
}

/// Whether `value` has the shape of a UUID (`xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, hex digits
/// and dashes). Written by hand instead of pulling in the `uuid` crate just for a test.
fn looks_like_a_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    const DASH_POSITIONS: [usize; 4] = [8, 13, 18, 23];
    bytes.len() == 36
        && DASH_POSITIONS.iter().all(|&i| bytes[i] == b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(i, &b)| DASH_POSITIONS.contains(&i) || b.is_ascii_hexdigit())
}

#[test]
fn login_token_is_reused_in_the_next_request() {
    let (server, rx, handle) = start_server(|captured| {
        if captured.url == "/login" {
            StubResponse::new(200).with_body(r#"{"token":"abc123"}"#)
        } else {
            StubResponse::new(200).with_body("ok")
        }
    });
    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let login = Request {
        method: Method::Post,
        url: format!("{}/login", server.url()),
        post_script: r#"
            var body = res.json();
            env.set("token", body.token);
        "#
        .to_string(),
        ..Request::default()
    };
    let login_result = runner.run(&login, &environment, &mut session);
    assert!(
        login_result.failed_stage.is_none(),
        "{:?}",
        login_result.failed_stage
    );

    let me = Request {
        method: Method::Get,
        url: format!("{}/me", server.url()),
        headers: vec![KeyValue::new("Authorization", "Bearer {{token}}")],
        ..Request::default()
    };
    let me_result = runner.run(&me, &environment, &mut session);
    assert!(
        me_result.failed_stage.is_none(),
        "{:?}",
        me_result.failed_stage
    );
    assert!(me_result.warnings.is_empty());

    drop(server);
    handle.join().expect("test server thread panicked");

    let login_captured = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("login request");
    assert_eq!(login_captured.url, "/login");
    let me_captured = rx.recv_timeout(Duration::from_secs(1)).expect("me request");
    let authorization = me_captured
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_str());
    assert_eq!(authorization, Some("Bearer abc123"));
}

#[test]
fn auth_uses_variables_set_by_the_pre_script_and_the_post_script_sees_it() {
    let (server, rx, handle) = start_server(|_| StubResponse::new(200));
    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let request = Request {
        method: Method::Get,
        url: format!("{}/me", server.url()),
        auth: Auth::Bearer {
            token: "{{token}}".to_string(),
        },
        pre_script: r#"
            if (!req.headers.get("Authorization")) {
                vars.set("token", "from-pre");
            }
        "#
        .to_string(),
        post_script: r#"
            test("the post script sees the auth header", () => {
                expect(req.headers.get("Authorization")).toBe("Bearer from-pre");
            });
        "#
        .to_string(),
        ..Request::default()
    };
    let result = runner.run(&request, &environment, &mut session);
    assert!(result.failed_stage.is_none(), "{:?}", result.failed_stage);
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(result.tests.len(), 1);
    assert!(result.tests[0].passed, "{:?}", result.tests[0]);

    drop(server);
    handle.join().expect("test server thread panicked");
    let captured = rx.recv_timeout(Duration::from_secs(1)).expect("request");
    let authorization = captured
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        .map(|(_, value)| value.as_str());
    assert_eq!(authorization, Some("Bearer from-pre"));
}

#[test]
fn pre_script_can_add_a_header() {
    let (server, rx, handle) = start_server(|_| StubResponse::new(200));
    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let request = Request {
        method: Method::Get,
        url: server.url(),
        pre_script: r#"req.headers.set("X-Generated", "yes");"#.to_string(),
        ..Request::default()
    };
    let result = runner.run(&request, &environment, &mut session);
    assert!(result.failed_stage.is_none(), "{:?}", result.failed_stage);

    drop(server);
    handle.join().expect("test server thread panicked");
    let captured = rx.recv_timeout(Duration::from_secs(1)).expect("request");
    assert!(
        captured
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("x-generated") && value == "yes")
    );
}

#[test]
fn template_function_uuid_is_resolved() {
    let (server, rx, handle) = start_server(|_| StubResponse::new(200));
    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let request = Request {
        method: Method::Get,
        url: server.url(),
        headers: vec![KeyValue::new("X-Request-Id", "{{ uuid() }}")],
        ..Request::default()
    };
    let result = runner.run(&request, &environment, &mut session);
    assert!(result.failed_stage.is_none(), "{:?}", result.failed_stage);
    assert!(result.warnings.is_empty());

    drop(server);
    handle.join().expect("test server thread panicked");
    let captured = rx.recv_timeout(Duration::from_secs(1)).expect("request");
    let request_id = captured
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-request-id"))
        .map(|(_, value)| value.clone())
        .expect("X-Request-Id header");
    assert!(looks_like_a_uuid(&request_id), "not a uuid: {request_id}");
}

#[test]
fn failing_test_is_reported() {
    let (server, rx, handle) = start_server(|_| StubResponse::new(200));
    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let request = Request {
        method: Method::Get,
        url: server.url(),
        post_script: r#"
            test("status is a teapot", function () {
                expect(res.status).toBe(418);
            });
        "#
        .to_string(),
        ..Request::default()
    };
    let result = runner.run(&request, &environment, &mut session);
    assert!(result.failed_stage.is_none(), "{:?}", result.failed_stage);
    assert_eq!(result.tests.len(), 1);
    assert!(!result.tests[0].passed);
    assert!(result.tests[0].message.is_some());

    drop(server);
    handle.join().expect("test server thread panicked");
    rx.recv_timeout(Duration::from_secs(1)).expect("request");
}

#[test]
fn pre_script_exception_prevents_sending() {
    let (server, rx, handle) = start_server(|_| StubResponse::new(200));
    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let request = Request {
        method: Method::Get,
        url: server.url(),
        pre_script: r#"throw new Error("boom");"#.to_string(),
        ..Request::default()
    };
    let result = runner.run(&request, &environment, &mut session);
    assert!(matches!(result.failed_stage, Some(FailedStage::Pre(_))));
    assert!(result.resolved_request.is_none());
    assert!(result.response.is_none());

    drop(server);
    handle.join().expect("test server thread panicked");
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "the server should never have received a request"
    );
}

#[test]
fn send_failure_is_reported() {
    let server = TestServer::start();
    let url = server.url();
    // Closes the listening socket: connecting now fails deterministically, with nothing else
    // bound to this ephemeral port.
    drop(server);

    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let request = Request {
        method: Method::Get,
        url,
        ..Request::default()
    };
    let result = runner.run(&request, &environment, &mut session);
    assert!(matches!(result.failed_stage, Some(FailedStage::Send(_))));
    assert!(result.resolved_request.is_some());
    assert!(result.response.is_none());
}

#[test]
fn missing_variable_is_reported_as_a_warning() {
    let (server, rx, handle) = start_server(|_| StubResponse::new(200));
    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let request = Request {
        method: Method::Get,
        url: server.url(),
        headers: vec![KeyValue::new("X-Missing", "{{missing}}")],
        ..Request::default()
    };
    let result = runner.run(&request, &environment, &mut session);
    assert!(result.failed_stage.is_none(), "{:?}", result.failed_stage);
    assert_eq!(
        result.warnings,
        vec![TemplateWarning::UnknownVariable("missing".to_string())]
    );

    drop(server);
    handle.join().expect("test server thread panicked");
    rx.recv_timeout(Duration::from_secs(1)).expect("request");
}

#[test]
fn json_body_is_sent_with_a_default_content_type() {
    let (server, rx, handle) = start_server(|_| StubResponse::new(200));
    let runner = runner();
    let environment = Environment::default();
    let mut session = SessionEnv::new();

    let request = Request {
        method: Method::Post,
        url: server.url(),
        body: Body::Json(r#"{"name":"brasa"}"#.to_string()),
        ..Request::default()
    };
    let result = runner.run(&request, &environment, &mut session);
    assert!(result.failed_stage.is_none(), "{:?}", result.failed_stage);

    drop(server);
    handle.join().expect("test server thread panicked");
    let captured = rx.recv_timeout(Duration::from_secs(1)).expect("request");
    assert_eq!(captured.body, br#"{"name":"brasa"}"#);
    assert!(
        captured
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                && value == "application/json")
    );
}
