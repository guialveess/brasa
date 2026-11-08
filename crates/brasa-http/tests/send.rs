//! Integration tests for `brasa_http::send`, against a local `tiny_http` server.
//!
//! No test here ever touches the internet: every server is bound to `127.0.0.1` on an
//! OS-assigned port by `brasa_http::test_support::TestServer`.
// `expect()` is the normal way to fail a test with a clear message; allowed crate-wide since
// this whole file is test code.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use brasa_core::{Method, ResolvedBody, ResolvedField, ResolvedRequest};
use brasa_http::test_support::{CapturedRequest, StubResponse, TestServer};
use brasa_http::{Client, HttpError, InvalidCertificates, SendOptions};

/// A bare request for `method` against `url`, no headers, no body.
fn request_to(method: Method, url: impl Into<String>) -> ResolvedRequest {
    ResolvedRequest {
        method,
        url: url.into(),
        ..ResolvedRequest::default()
    }
}

/// The outcome of a [`run`] call: the request the server captured (`None` if it never received
/// one, for example in the timeout test) and the result `brasa_http::send` returned.
struct TestRun {
    captured: Option<CapturedRequest>,
    result: Result<brasa_core::Response, HttpError>,
}

/// Starts a server, builds a request against it with `build_request`, has the server answer
/// every request with `respond_with`, and sends the request through `brasa_http::send`.
///
/// Only the first captured request is kept (some tests, like the redirect ones, cause more than
/// one). The server is shut down and its thread joined before returning, so a panic inside
/// `respond_with` (for example a failed assertion) fails the test.
fn run<B, F>(build_request: B, options: &SendOptions, respond_with: F) -> TestRun
where
    B: FnOnce(&str) -> ResolvedRequest,
    F: Fn(CapturedRequest) -> StubResponse + Send + 'static,
{
    let server = TestServer::start();
    let request = build_request(&server.url());

    let (tx, rx) = mpsc::channel();
    let handle = server.serve(move |captured| {
        let response = respond_with(captured.clone());
        // The receiver may already have what it needs in tests that trigger more than one
        // request (redirects); a dropped receiver here is not a bug in the server.
        let _ = tx.send(captured);
        response
    });

    let result = brasa_http::send(&request, options);
    let captured = rx.recv_timeout(Duration::from_secs(1)).ok();

    drop(server);
    handle.join().expect("test server thread panicked");

    TestRun { captured, result }
}

#[test]
fn sends_every_standard_method() {
    let methods = [
        (Method::Get, "GET"),
        (Method::Post, "POST"),
        (Method::Put, "PUT"),
        (Method::Patch, "PATCH"),
        (Method::Delete, "DELETE"),
        (Method::Head, "HEAD"),
        (Method::Options, "OPTIONS"),
        (Method::Custom("PURGE".to_string()), "PURGE"),
    ];

    for (method, expected) in methods {
        let run = run(
            move |url| request_to(method, url),
            &SendOptions::default(),
            |_| StubResponse::new(200),
        );

        let captured = run
            .captured
            .expect("server should have received the request");
        assert_eq!(captured.method, expected);
        let response = run.result.expect("request should succeed");
        assert_eq!(response.status, 200);
    }
}

#[test]
fn sends_request_headers() {
    let run = run(
        |url| {
            let mut request = request_to(Method::Get, url);
            request.headers.push(ResolvedField {
                name: "X-Test".to_string(),
                value: "hello".to_string(),
            });
            request
        },
        &SendOptions::default(),
        |_| StubResponse::new(200),
    );

    let captured = run
        .captured
        .expect("server should have received the request");
    assert!(
        captured
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("x-test") && value == "hello"),
        "expected an X-Test: hello header, got {:?}",
        captured.headers
    );
    run.result.expect("request should succeed");
}

#[test]
fn receives_response_headers() {
    let run = run(
        |url| request_to(Method::Get, url),
        &SendOptions::default(),
        |_| StubResponse::new(200).with_header("X-Reply", "world"),
    );

    let response = run.result.expect("request should succeed");
    assert!(
        response
            .headers
            .iter()
            .any(|field| field.name.eq_ignore_ascii_case("x-reply") && field.value == "world"),
        "expected an X-Reply: world header, got {:?}",
        response.headers
    );
}

#[test]
fn sends_json_body() {
    let run = run(
        |url| {
            let mut request = request_to(Method::Post, url);
            request.headers.push(ResolvedField {
                name: "Content-Type".to_string(),
                value: "application/json".to_string(),
            });
            request.body = ResolvedBody::Bytes(br#"{"name":"brasa"}"#.to_vec());
            request
        },
        &SendOptions::default(),
        |_| StubResponse::new(200),
    );

    let captured = run
        .captured
        .expect("server should have received the request");
    assert_eq!(captured.body, br#"{"name":"brasa"}"#);
    assert!(
        captured
            .headers
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                && value == "application/json")
    );
    run.result.expect("request should succeed");
}

#[test]
fn sends_text_body() {
    let run = run(
        |url| {
            let mut request = request_to(Method::Post, url);
            request.headers.push(ResolvedField {
                name: "Content-Type".to_string(),
                value: "text/plain".to_string(),
            });
            request.body = ResolvedBody::Bytes(b"hello, brasa".to_vec());
            request
        },
        &SendOptions::default(),
        |_| StubResponse::new(200),
    );

    let captured = run
        .captured
        .expect("server should have received the request");
    assert_eq!(captured.body, b"hello, brasa");
    run.result.expect("request should succeed");
}

#[test]
fn sends_form_body() {
    let run = run(
        |url| {
            let mut request = request_to(Method::Post, url);
            request.headers.push(ResolvedField {
                name: "Content-Type".to_string(),
                value: "application/x-www-form-urlencoded".to_string(),
            });
            request.body = ResolvedBody::Bytes(b"a=1&b=2".to_vec());
            request
        },
        &SendOptions::default(),
        |_| StubResponse::new(200),
    );

    let captured = run
        .captured
        .expect("server should have received the request");
    assert_eq!(captured.body, b"a=1&b=2");
    run.result.expect("request should succeed");
}

#[test]
fn returns_404_as_a_normal_response() {
    let run = run(
        |url| request_to(Method::Get, url),
        &SendOptions::default(),
        |_| StubResponse::new(404).with_body("missing"),
    );

    let response = run.result.expect("a 404 is a response, not an error");
    assert_eq!(response.status, 404);
    assert_eq!(response.body, b"missing");
}

#[test]
fn returns_500_as_a_normal_response() {
    let run = run(
        |url| request_to(Method::Get, url),
        &SendOptions::default(),
        |_| StubResponse::new(500).with_body("oops"),
    );

    let response = run.result.expect("a 500 is a response, not an error");
    assert_eq!(response.status, 500);
    assert_eq!(response.body, b"oops");
}

#[test]
fn follows_redirects_by_default() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let count_for_handler = Arc::clone(&request_count);

    let run = run(
        |url| request_to(Method::Get, url),
        &SendOptions::default(),
        move |_| {
            let seen = count_for_handler.fetch_add(1, Ordering::SeqCst);
            if seen == 0 {
                StubResponse::new(302).with_header("Location", "/target")
            } else {
                StubResponse::new(200).with_body("landed")
            }
        },
    );

    let response = run
        .result
        .expect("request should succeed after following the redirect");
    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"landed");
    assert_eq!(request_count.load(Ordering::SeqCst), 2);
}

#[test]
fn does_not_follow_redirects_when_disabled() {
    let options = SendOptions {
        follow_redirects: false,
        ..SendOptions::default()
    };

    let run = run(
        |url| request_to(Method::Get, url),
        &options,
        |_| StubResponse::new(302).with_header("Location", "/target"),
    );

    let response = run
        .result
        .expect("request should succeed, redirect included");
    assert_eq!(response.status, 302);
    assert!(
        response
            .headers
            .iter()
            .any(|field| field.name.eq_ignore_ascii_case("location"))
    );
}

#[test]
fn times_out_when_the_server_is_too_slow() {
    let options = SendOptions {
        timeout: Duration::from_millis(100),
        ..SendOptions::default()
    };

    let run = run(
        |url| request_to(Method::Get, url),
        &options,
        |_| {
            std::thread::sleep(Duration::from_millis(600));
            StubResponse::new(200)
        },
    );

    match run.result {
        Err(HttpError::Timeout) => {}
        other => panic!("expected a timeout error, got {other:?}"),
    }
}

#[test]
fn reads_a_body_above_the_old_10_mb_limit() {
    let size = 12 * 1024 * 1024;
    let run = run(
        |url| request_to(Method::Get, url),
        &SendOptions::default(),
        move |_| StubResponse::new(200).with_body(vec![b'x'; size]),
    );
    let response = run
        .result
        .expect("a 12 MiB body is below the default 20 MB limit");
    assert_eq!(response.size, size);
}

#[test]
fn fails_with_a_clear_error_when_the_body_exceeds_the_limit() {
    let options = SendOptions {
        max_response_size: 1000,
        ..SendOptions::default()
    };
    let run = run(
        |url| request_to(Method::Get, url),
        &options,
        |_| StubResponse::new(200).with_body(vec![b'x'; 2000]),
    );
    match run.result {
        Err(HttpError::BodyTooLarge(1000)) => {}
        other => panic!("expected BodyTooLarge, got {other:?}"),
    }
}

/// Sends three requests to one server through `send_three` and returns the client port of each,
/// as the server saw it.
fn client_ports(send_three: impl FnOnce(&str)) -> Vec<u16> {
    let server = TestServer::start();
    let (tx, rx) = mpsc::channel();
    let handle = server.serve(move |captured| {
        let _ = tx.send(captured.remote_addr.map(|addr| addr.port()));
        StubResponse::new(200).with_body("ok")
    });

    send_three(&server.url());
    let ports = (0..3)
        .map(|_| {
            rx.recv_timeout(Duration::from_secs(1))
                .expect("a request")
                .expect("a remote address")
        })
        .collect();

    drop(server);
    handle.join().expect("test server thread panicked");
    ports
}

#[test]
fn send_opens_a_new_connection_for_every_request() {
    let ports = client_ports(|url| {
        for _ in 0..3 {
            brasa_http::send(&request_to(Method::Get, url), &SendOptions::default())
                .expect("sent");
        }
    });

    assert_ne!(ports[0], ports[1]);
    assert_ne!(ports[1], ports[2]);
}

#[test]
fn a_client_reuses_its_connection() {
    let client = Client::new(SendOptions::default());
    let ports = client_ports(|url| {
        for path in ["/a", "/b", "/c"] {
            let response = client
                .send(&request_to(Method::Get, format!("{url}{path}")))
                .expect("sent");
            assert_eq!(response.body, b"ok");
        }
    });

    assert_eq!(ports[0], ports[1]);
    assert_eq!(ports[1], ports[2]);
}

/// A one-shot HTTPS server with a self-signed certificate for `127.0.0.1`, built on `rustls`
/// directly since `tiny_http` would need an extra TLS stack. Answers every connection with a 200
/// and the body `secure`, and records the request line of each request it actually reads.
fn start_tls_server() -> (String, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
    start_tls_server_for(1)
}

/// [`start_tls_server`] answering `responses` requests, one per connection, out of
/// `responses + 1` connections at most: one rejected handshake is all it allows.
fn start_tls_server_for(
    responses: usize,
) -> (String, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    let certs = CertificateDer::pem_slice_iter(include_bytes!("fixtures/self_signed.cert.pem"))
        .collect::<Result<Vec<_>, _>>()
        .expect("fixture certificate parses");
    let key = PrivateKeyDer::from_pem_slice(include_bytes!("fixtures/self_signed.key.pem"))
        .expect("fixture key parses");
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = Arc::new(
        rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("protocol versions")
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .expect("certificate and key match"),
    );

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("https://{}", listener.local_addr().expect("local addr"));
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        // The rejected handshake and one connection per response, at most.
        let mut answered = 0;
        for _ in 0..=responses {
            let Ok((tcp, _)) = listener.accept() else {
                return;
            };
            let Ok(connection) = rustls::ServerConnection::new(Arc::clone(&config)) else {
                return;
            };
            let mut stream = rustls::StreamOwned::new(connection, tcp);
            let mut buffer = [0_u8; 4096];
            // Fails for the client that rejects the certificate, which is expected.
            let Ok(read) = stream.read(&mut buffer) else {
                continue;
            };
            let text = String::from_utf8_lossy(&buffer[..read]);
            let _ = tx.send(text.lines().next().unwrap_or_default().to_string());
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nsecure",
            );
            stream.conn.send_close_notify();
            let _ = stream.flush();
            answered += 1;
            if answered == responses {
                return;
            }
        }
    });
    (url, rx, handle)
}

#[test]
fn untrusted_certificate_is_sent_anyway_with_a_warning() {
    let (url, requests, handle) = start_tls_server();
    let request = ResolvedRequest {
        method: Method::Post,
        body: ResolvedBody::Bytes(b"payload".to_vec()),
        ..request_to(Method::Post, format!("{url}/secure"))
    };

    let response = brasa_http::send(&request, &SendOptions::default()).expect("fallback works");

    assert_eq!(response.status, 200);
    assert_eq!(response.body, b"secure");
    let warning = response.tls_warning.expect("a warning is set");
    assert!(
        warning.starts_with("TLS certificate not verified: "),
        "{warning}"
    );
    assert!(warning.contains("unknown issuer"), "{warning}");
    // The server saw the request exactly once: the rejected handshake sent nothing.
    assert_eq!(
        requests.recv().expect("one request"),
        "POST /secure HTTP/1.1"
    );
    handle.join().expect("server thread");
}

#[test]
fn a_client_remembers_an_untrusted_certificate() {
    // Two responses out of three connections: a second rejected handshake would leave the
    // second request without an answer.
    let (url, requests, handle) = start_tls_server_for(2);
    let client = Client::new(SendOptions::default());

    for _ in 0..2 {
        let response = client
            .send(&request_to(Method::Get, format!("{url}/secure")))
            .expect("sent without verification");
        assert_eq!(response.body, b"secure");
        assert!(response.tls_warning.is_some());
    }

    assert_eq!(requests.iter().count(), 2);
    handle.join().expect("server thread");
}

#[test]
fn accept_sends_without_a_warning() {
    // `Accept` never verifies, so the request goes through and no warning is set.
    let (url, _requests, handle) = start_tls_server();
    let options = SendOptions {
        invalid_certificates: InvalidCertificates::Accept,
        ..SendOptions::default()
    };

    let response =
        brasa_http::send(&request_to(Method::Get, format!("{url}/")), &options).expect("ok");

    assert_eq!(response.tls_warning, None);
    handle.join().expect("server thread");
}

#[test]
fn reject_fails_on_an_untrusted_certificate() {
    let (url, _requests, handle) = start_tls_server();
    let options = SendOptions {
        invalid_certificates: InvalidCertificates::Reject,
        ..SendOptions::default()
    };

    let result = brasa_http::send(&request_to(Method::Get, format!("{url}/")), &options);

    match result {
        Err(HttpError::Certificate(reason)) => {
            assert!(reason.contains("unknown issuer"), "{reason}");
        }
        other => panic!("expected a certificate error, got {other:?}"),
    }
    let message = HttpError::Certificate("unknown issuer".to_string()).to_string();
    assert_eq!(message, "invalid TLS certificate: unknown issuer");
    // Nothing connected after the rejected handshake: stop the server thread's second accept.
    let _ = std::net::TcpStream::connect(url.trim_start_matches("https://"));
    handle.join().expect("server thread");
}

/// The certificate the TLS test server presents, which the child of
/// `system_certificates_trust_the_system_bundle` trusts as its system bundle.
#[cfg(all(unix, not(target_vendor = "apple")))]
const SELF_SIGNED_CERT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/self_signed.cert.pem"
);

#[cfg(all(unix, not(target_vendor = "apple")))]
#[test]
fn system_certificates_trust_the_system_bundle() {
    // The system bundle is read once per process and the test cannot change the environment of
    // its own, so it runs the test below again in a child process, with `SSL_CERT_FILE` pointing
    // at the server's certificate.
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "system_certificates_child",
            "--ignored",
            "--nocapture",
        ])
        .env("SSL_CERT_FILE", SELF_SIGNED_CERT)
        .env_remove("SSL_CERT_DIR")
        .output()
        .expect("child test runs");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

/// Run by `system_certificates_trust_the_system_bundle` only.
#[cfg(all(unix, not(target_vendor = "apple")))]
#[test]
#[ignore = "run by system_certificates_trust_the_system_bundle with SSL_CERT_FILE set"]
fn system_certificates_child() {
    assert_eq!(
        std::env::var("SSL_CERT_FILE").as_deref(),
        Ok(SELF_SIGNED_CERT)
    );
    // One rejected handshake, then one verified request, which stops the server.
    let (url, _requests, handle) = start_tls_server_for(1);
    let request = request_to(Method::Get, format!("{url}/secure"));
    let bundled = SendOptions {
        invalid_certificates: InvalidCertificates::Reject,
        system_certificates: false,
        ..SendOptions::default()
    };

    match brasa_http::send(&request, &bundled) {
        Err(HttpError::Certificate(reason)) => {
            assert!(reason.contains("unknown issuer"), "{reason}")
        }
        other => panic!("expected a certificate error, got {other:?}"),
    }

    let system = SendOptions {
        system_certificates: true,
        ..bundled
    };
    let response = brasa_http::send(&request, &system).expect("verified by the system bundle");
    assert_eq!(response.body, b"secure");
    assert_eq!(response.tls_warning, None);
    handle.join().expect("server thread");
}
