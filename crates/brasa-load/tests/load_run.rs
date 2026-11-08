//! Integration tests for [`brasa_load::LoadRun`], against a local `tiny_http` server and the
//! real `QuickJsEngine`.
//!
//! No test here ever touches the internet: every server is bound to `127.0.0.1` on an
//! OS-assigned port by `brasa_http::test_support::TestServer`. Every test finishes in well
//! under 3 seconds.
// `expect()` is the normal way to fail a test with a clear message; allowed crate-wide since
// this whole file is test code.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use brasa_core::{Environment, KeyValue, Method, Request};
use brasa_http::SendOptions;
use brasa_http::test_support::{CapturedRequest, StubResponse, TestServer};
use brasa_load::{LoadConfig, LoadRun, LoadTarget, SampleLog, StatusKey};
use brasa_runner::{Runner, SessionEnv};
use brasa_script::QuickJsEngine;
use pretty_assertions::assert_eq;

/// A bare, enabled [`Runner`] backed by the real `QuickJsEngine`, sending with default
/// [`SendOptions`].
fn runner() -> Arc<Runner> {
    Arc::new(Runner::new(Arc::new(QuickJsEngine), SendOptions::default()))
}

/// A bare GET request against `url`, no headers, no body, no scripts.
fn get(url: impl Into<String>) -> Request {
    Request {
        method: Method::Get,
        url: url.into(),
        ..Request::default()
    }
}

#[test]
fn a_short_run_produces_samples_for_every_target_with_a_200_status_count() {
    let server = TestServer::start();
    let handle = server.serve(|_| StubResponse::new(200));

    let config = LoadConfig {
        targets: vec![
            LoadTarget::new("a", get(format!("{}/a", server.url()))),
            LoadTarget::new("b", get(format!("{}/b", server.url()))),
        ],
        vus: 4,
        duration: Duration::from_secs(1),
        ramp_up: Duration::ZERO,
        think_time: Duration::ZERO,
        stop_on_error_rate: None,
        reuse_connections: false,
    };

    let run = LoadRun::start(config, Environment::default(), SessionEnv::new(), runner());
    let summary = run.join();

    drop(server);
    handle.join().expect("test server thread panicked");

    assert!(!summary.stopped_early);
    assert!(summary.snapshot.total > 0, "expected at least one sample");
    assert_eq!(summary.snapshot.per_target.len(), 2);
    assert!(
        summary.snapshot.per_target[0].count > 0,
        "target 'a' got no samples"
    );
    assert!(
        summary.snapshot.per_target[1].count > 0,
        "target 'b' got no samples"
    );
    assert_eq!(
        summary.snapshot.status_counts,
        vec![(StatusKey::Code(200), summary.snapshot.total)]
    );
    assert!((summary.snapshot.error_rate - 0.0).abs() < f64::EPSILON);
}

#[test]
fn ramp_up_starts_fewer_virtual_users_early_than_late() {
    let server = TestServer::start();
    let handle = server.serve(|_| StubResponse::new(200));

    let config = LoadConfig {
        targets: vec![LoadTarget::new("a", get(server.url()))],
        vus: 10,
        duration: Duration::from_millis(600),
        ramp_up: Duration::from_millis(400),
        think_time: Duration::from_millis(5),
        stop_on_error_rate: None,
        reuse_connections: false,
    };

    let run = LoadRun::start(config, Environment::default(), SessionEnv::new(), runner());

    std::thread::sleep(Duration::from_millis(60));
    let early_active = run.snapshot().active_vus;

    std::thread::sleep(Duration::from_millis(450));
    let late_active = run.snapshot().active_vus;

    run.stop();
    let _ = run.join();
    drop(server);
    handle.join().expect("test server thread panicked");

    assert!(
        early_active < late_active,
        "expected fewer virtual users early ({early_active}) than late ({late_active})"
    );
    assert!(
        late_active >= 8,
        "expected ramp-up to have nearly finished by now, got {late_active}"
    );
}

#[test]
fn stop_ends_the_run_promptly() {
    let server = TestServer::start();
    let handle = server.serve(|_| StubResponse::new(200));

    let config = LoadConfig {
        targets: vec![LoadTarget::new("a", get(server.url()))],
        vus: 2,
        duration: Duration::from_secs(5),
        ramp_up: Duration::ZERO,
        think_time: Duration::ZERO,
        stop_on_error_rate: None,
        reuse_connections: false,
    };

    let run = LoadRun::start(config, Environment::default(), SessionEnv::new(), runner());
    std::thread::sleep(Duration::from_millis(50));

    let started_stopping = Instant::now();
    run.stop();
    let summary = run.join();
    let stop_latency = started_stopping.elapsed();

    drop(server);
    handle.join().expect("test server thread panicked");

    assert!(summary.stopped_early);
    assert!(
        stop_latency < Duration::from_millis(500),
        "stop() took {stop_latency:?} to take effect"
    );
}

#[test]
fn a_server_returning_500_with_a_stop_on_error_rate_stops_the_run_early() {
    let server = TestServer::start();
    let handle = server.serve(|_| StubResponse::new(500));

    let config = LoadConfig {
        targets: vec![LoadTarget::new("a", get(server.url()))],
        vus: 10,
        // A generous nominal duration: if the error-rate stop condition ever regressed, this
        // test would still finish within the 3 second budget instead of hanging, just less
        // conclusively.
        duration: Duration::from_secs(2),
        ramp_up: Duration::ZERO,
        think_time: Duration::ZERO,
        stop_on_error_rate: Some(0.1),
        reuse_connections: false,
    };

    let started = Instant::now();
    let run = LoadRun::start(config, Environment::default(), SessionEnv::new(), runner());
    let summary = run.join();
    let elapsed = started.elapsed();

    drop(server);
    handle.join().expect("test server thread panicked");

    assert!(summary.stopped_early);
    assert!((summary.snapshot.error_rate - 1.0).abs() < f64::EPSILON);
    assert!(summary.snapshot.total >= 50);
    assert!(
        elapsed < Duration::from_secs(1),
        "expected the error rate threshold to stop the run well before its 2 second duration, \
         took {elapsed:?}"
    );
}

#[test]
fn a_post_script_env_change_flows_to_the_next_target_of_the_same_virtual_user() {
    let server = TestServer::start();
    let request_count = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel();
    let counter = Arc::clone(&request_count);
    let handle = server.serve(move |captured: CapturedRequest| {
        counter.fetch_add(1, Ordering::SeqCst);
        let _ = tx.send(captured.clone());
        if captured.url == "/login" {
            StubResponse::new(200).with_body(r#"{"token":"abc123"}"#)
        } else {
            StubResponse::new(200)
        }
    });

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
    let me = Request {
        method: Method::Get,
        url: format!("{}/me", server.url()),
        headers: vec![KeyValue::new("Authorization", "Bearer {{token}}")],
        ..Request::default()
    };

    let config = LoadConfig {
        targets: vec![LoadTarget::new("login", login), LoadTarget::new("me", me)],
        vus: 2,
        duration: Duration::from_millis(300),
        ramp_up: Duration::ZERO,
        think_time: Duration::from_millis(20),
        stop_on_error_rate: None,
        reuse_connections: false,
    };

    let run = LoadRun::start(config, Environment::default(), SessionEnv::new(), runner());
    let summary = run.join();

    drop(server);
    handle.join().expect("test server thread panicked");

    assert!(summary.snapshot.total > 0);

    let mut me_requests_seen = 0;
    while let Ok(captured) = rx.recv_timeout(Duration::from_millis(100)) {
        if captured.url == "/me" {
            me_requests_seen += 1;
            let authorization = captured
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| value.as_str());
            assert_eq!(
                authorization,
                Some("Bearer abc123"),
                "a 'me' request did not see the token its own virtual user's login set"
            );
        }
    }
    assert!(me_requests_seen > 0, "expected at least one 'me' request");
}

/// Runs `vus` virtual users against a local server for a short while and returns how many
/// requests the server answered and over how many connections (distinct client ports).
fn requests_and_connections(vus: u32, reuse_connections: bool) -> (usize, usize) {
    let server = TestServer::start();
    let (tx, rx) = mpsc::channel();
    let handle = server.serve(move |captured: CapturedRequest| {
        let _ = tx.send(captured.remote_addr.map(|addr| addr.port()));
        StubResponse::new(200)
    });

    let config = LoadConfig {
        targets: vec![LoadTarget::new("a", get(server.url()))],
        vus,
        duration: Duration::from_millis(300),
        ramp_up: Duration::ZERO,
        think_time: Duration::from_millis(10),
        stop_on_error_rate: None,
        reuse_connections,
    };
    let summary =
        LoadRun::start(config, Environment::default(), SessionEnv::new(), runner()).join();

    drop(server);
    handle.join().expect("test server thread panicked");

    let ports: Vec<u16> = rx.try_iter().flatten().collect();
    assert_eq!(ports.len() as u64, summary.snapshot.total);
    let connections = ports.iter().collect::<std::collections::HashSet<_>>().len();
    (ports.len(), connections)
}

#[test]
fn reusing_connections_keeps_one_connection_per_virtual_user() {
    let (requests, connections) = requests_and_connections(3, true);
    assert!(
        requests > 6,
        "expected several iterations per virtual user, got {requests}"
    );
    assert_eq!(connections, 3);
}

#[test]
fn without_reuse_every_request_opens_a_connection() {
    let (requests, connections) = requests_and_connections(3, false);
    assert!(
        requests > 6,
        "expected several iterations per virtual user, got {requests}"
    );
    // One connection per request. Not compared exactly: the OS may hand out a closed
    // connection's port again.
    assert!(
        connections > requests / 2,
        "{requests} requests over only {connections} connections"
    );
}

#[test]
fn the_summary_log_has_one_sample_per_request_with_resolved_urls_and_sizes() {
    let server = TestServer::start();
    let handle = server.serve(|captured: CapturedRequest| {
        if captured.url == "/missing" {
            StubResponse::new(404).with_body("nope \n  nope")
        } else {
            StubResponse::new(200).with_body("hello")
        }
    });
    let mut environment = Environment::default();
    environment
        .variables
        .push(KeyValue::new("host", server.url()));

    let config = LoadConfig {
        targets: vec![
            LoadTarget::new("ok", get("{{host}}/ok")),
            LoadTarget::new("missing", get("{{host}}/missing")),
        ],
        vus: 3,
        duration: Duration::from_millis(600),
        ramp_up: Duration::ZERO,
        think_time: Duration::from_millis(5),
        stop_on_error_rate: None,
        reuse_connections: true,
    };
    let run = LoadRun::start(config, environment, SessionEnv::new(), runner());

    // Poll like the UI does: the copy only ever grows, and every call appends only what is new.
    let mut polled = SampleLog::new();
    let mut previous_len = 0;
    while !run.is_finished() {
        std::thread::sleep(Duration::from_millis(50));
        run.read_samples(&mut polled);
        assert!(polled.len() >= previous_len);
        previous_len = polled.len();
    }
    let summary = run.join();
    drop(server);
    handle.join().expect("test server thread panicked");

    let log = &summary.log;
    assert_eq!(log.len() as u64, summary.snapshot.total);
    assert!(
        log.len() > 4,
        "expected several requests, got {}",
        log.len()
    );
    assert_eq!(log.urls().len(), 2, "URLs are interned: {:?}", log.urls());

    for sample in log.samples() {
        let url = log.url(sample.url);
        match sample.target {
            0 => {
                assert!(url.ends_with("/ok") && url.starts_with("http://"), "{url}");
                assert_eq!(sample.status, StatusKey::Code(200));
                assert_eq!(sample.size, 5);
                assert!(!sample.is_error);
            }
            1 => {
                assert!(
                    url.ends_with("/missing") && url.starts_with("http://"),
                    "{url}"
                );
                assert_eq!(sample.status, StatusKey::Code(404));
                assert_eq!(sample.size, 12);
                assert!(sample.is_error);
            }
            other => panic!("unexpected target {other}"),
        }
        assert!(sample.vu < 3);
        assert!(sample.latency_us > 0);
        assert!(sample.elapsed_ms < 5_000);
    }
    assert_eq!(
        summary.snapshot.error_examples.len(),
        1,
        "{:?}",
        summary.snapshot.error_examples
    );
    assert_eq!(
        summary.snapshot.error_examples[0].text,
        "HTTP 404 \u{b7} nope nope"
    );

    // What the polling loop copied while the run was live is a prefix of the final log.
    assert!(polled.len() <= log.len());
    assert_eq!(polled.samples(), &log.samples()[..polled.len()]);
    assert_eq!(polled.urls(), &log.urls()[..polled.urls().len()]);
}

#[test]
fn read_samples_called_repeatedly_ends_with_the_same_log_as_the_summary() {
    let server = TestServer::start();
    let handle = server.serve(|_| StubResponse::new(200));
    let config = LoadConfig {
        targets: vec![LoadTarget::new("a", get(format!("{}/a", server.url())))],
        vus: 2,
        duration: Duration::from_secs(5),
        ramp_up: Duration::ZERO,
        think_time: Duration::from_millis(5),
        stop_on_error_rate: None,
        reuse_connections: true,
    };
    let run = LoadRun::start(config, Environment::default(), SessionEnv::new(), runner());

    // Poll until the log has grown twice, with a generous deadline for a loaded machine.
    let mut polled = SampleLog::new();
    let deadline = Instant::now() + Duration::from_secs(4);
    let mut early = 0;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        run.read_samples(&mut polled);
        if early == 0 {
            early = polled.len();
        } else if polled.len() > early {
            break;
        }
    }
    assert!(early > 0, "no sample was recorded");
    assert!(
        polled.len() > early,
        "the log did not grow while the run went on"
    );

    run.stop();
    // Wait for the supervisor to join every virtual user, then read the final state.
    while !run.is_finished() {
        std::thread::sleep(Duration::from_millis(10));
    }
    run.read_samples(&mut polled);
    run.read_samples(&mut polled);
    let summary = run.join();
    drop(server);
    handle.join().expect("test server thread panicked");

    assert_eq!(polled, summary.log);
}
