//! A small local HTTP server, for tests that need a real socket without touching the internet.
//!
//! Only compiled when the `test-support` feature is on (the default). It is used by this
//! crate's own integration tests under `tests/`, and is exposed publicly so other crates in the
//! workspace (for example `brasa-runner`) can depend on `brasa-http` with
//! `features = ["test-support"]` and reuse the same helper instead of writing their own.
//!
//! # Example
//!
//! ```
//! use brasa_http::test_support::{StubResponse, TestServer};
//!
//! let server = TestServer::start();
//! let handle = server.serve(|_request| StubResponse::new(200).with_body("hello"));
//!
//! // ... send a request to `server.url()` here ...
//!
//! drop(server); // unblocks the server thread so `handle.join()` returns
//! handle.join().expect("server thread panicked");
//! ```

use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// An owned, thread-safe snapshot of a request received by [`TestServer`].
///
/// Unlike [`tiny_http::Request`], this can be inspected freely (including from assertions
/// inside the handler closure) since it does not borrow the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRequest {
    /// The HTTP method, for example `"GET"` or `"POST"`. Non-standard methods are kept as sent.
    pub method: String,
    /// The request target as sent by the client: path plus query string, no scheme or host.
    pub url: String,
    /// The request headers, in the order `tiny_http` received them.
    pub headers: Vec<(String, String)>,
    /// The raw request body. Empty when the request had none.
    pub body: Vec<u8>,
    /// The client's address. Two requests from the same port came over the same connection.
    pub remote_addr: Option<SocketAddr>,
}

/// The response a [`TestServer`] handler wants to send back for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubResponse {
    /// The HTTP status code to send.
    pub status: u16,
    /// Extra response headers, in the order they should be sent.
    pub headers: Vec<(String, String)>,
    /// The response body.
    pub body: Vec<u8>,
}

impl StubResponse {
    /// A response with the given status code, no extra headers and an empty body.
    #[must_use]
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// Adds one response header.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Sets the response body, replacing any previous one.
    #[must_use]
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }
}

/// A `tiny_http` server bound to an OS-assigned port on `127.0.0.1`, for use in tests.
///
/// Dropping the server unblocks any thread started with [`TestServer::serve`], so a test does
/// not need to shut it down explicitly.
pub struct TestServer {
    server: Arc<tiny_http::Server>,
}

impl Drop for TestServer {
    /// Unblocks a thread started with [`TestServer::serve`] that is waiting for the next
    /// request, so it can notice the server is gone and return.
    fn drop(&mut self) {
        self.server.unblock();
    }
}

impl TestServer {
    /// Starts a new server listening on `127.0.0.1` with an OS-assigned port.
    ///
    /// # Panics
    ///
    /// Panics if binding a loopback socket fails, which should not happen in a normal test
    /// environment.
    #[must_use]
    // Binding a loopback socket on an OS-assigned port is not expected to fail in a test
    // environment, and this helper exists purely for tests, so a panic here is the simplest
    // way to surface the (practically impossible) failure.
    #[allow(clippy::expect_used)]
    pub fn start() -> Self {
        let server =
            tiny_http::Server::http("127.0.0.1:0").expect("test server: failed to bind a socket");
        Self {
            server: Arc::new(server),
        }
    }

    /// The base URL of the server, for example `http://127.0.0.1:54321`, with no trailing
    /// slash.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}", self.server.server_addr())
    }

    /// Spawns a background thread that answers every request the server receives, until the
    /// `TestServer` is dropped, by calling `handler` and sending back the [`StubResponse`] it
    /// returns.
    ///
    /// A failure to write the response (for example because the client already gave up, as in
    /// a timeout test) is ignored: it is expected in some tests and is not a bug in the server
    /// itself. Panics inside `handler` (for example a failed `assert_eq!` on the captured
    /// request) propagate to the caller through the returned [`JoinHandle`] once joined.
    pub fn serve<F>(&self, handler: F) -> JoinHandle<()>
    where
        F: Fn(CapturedRequest) -> StubResponse + Send + 'static,
    {
        let server = Arc::clone(&self.server);
        thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let captured = capture(&mut request);
                let stub = handler(captured);
                let _ = respond(request, stub);
            }
        })
    }
}

/// Reads a `tiny_http::Request` into an owned [`CapturedRequest`].
fn capture(request: &mut tiny_http::Request) -> CapturedRequest {
    let method = request.method().as_str().to_string();
    let url = request.url().to_string();
    let headers = request
        .headers()
        .iter()
        .map(|header| {
            (
                header.field.as_str().as_str().to_string(),
                header.value.as_str().to_string(),
            )
        })
        .collect();

    let mut body = Vec::new();
    // A request without a body simply reads zero bytes here, so this is safe to call always.
    let _ = request.as_reader().read_to_end(&mut body);

    CapturedRequest {
        method,
        url,
        headers,
        body,
        remote_addr: request.remote_addr().copied(),
    }
}

/// Sends a [`StubResponse`] back for a received request.
fn respond(request: tiny_http::Request, stub: StubResponse) -> Result<(), std::io::Error> {
    let mut response = tiny_http::Response::from_data(stub.body).with_status_code(stub.status);

    for (name, value) in stub.headers {
        if let Ok(header) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response.add_header(header);
        }
    }

    request.respond(response)
}
