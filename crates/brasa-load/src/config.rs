//! Configuration for a load test run: the requests to send and how hard to hit them.

use std::time::Duration;

use brasa_core::Request;

/// One request a load test loops over.
///
/// A single request is one target; a collection is every request of a folder, in tree order.
/// Building that list from a workspace is the caller's job (`brasa-app`), this crate only runs
/// whatever [`LoadTarget`]s it is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadTarget {
    /// A short identifier for this target, used to label its samples in
    /// [`crate::LoadSnapshot::per_target`] and in the per-request table. Usually the request's
    /// file stem.
    pub id: String,
    /// The request to send. Interpolated fresh on every iteration by
    /// [`brasa_runner::Runner::run`], so `{{ }}` markers pick up whatever the environment and
    /// the virtual user's session hold at the time.
    pub request: Request,
}

impl LoadTarget {
    /// Creates a target named `id` for `request`.
    #[must_use]
    pub fn new(id: impl Into<String>, request: Request) -> Self {
        Self {
            id: id.into(),
            request,
        }
    }
}

/// The configuration of a load test run.
///
/// `PartialEq` only, not `Eq`: `stop_on_error_rate` is an `f64`.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadConfig {
    /// The requests to run. Every virtual user loops over these in order, so a login request
    /// followed by an authenticated one behaves as expected: the same virtual user sees the
    /// token its own post script set.
    pub targets: Vec<LoadTarget>,
    /// The number of virtual users to run at full ramp-up.
    ///
    /// The UI is expected to keep this in `1..=500`; [`crate::LoadRun::start`] does not enforce
    /// the range itself, it simply starts however many virtual users it is asked for (including
    /// `0`, which finishes immediately with no samples).
    pub vus: u32,
    /// How long the run lasts, once every virtual user has started. Measured from the moment
    /// [`crate::LoadRun::start`] is called, not from the moment the last virtual user starts.
    pub duration: Duration,
    /// How long it takes to go from `0` to `vus` virtual users, started at even intervals
    /// (linear ramp-up).
    pub ramp_up: Duration,
    /// How long a virtual user waits after finishing one iteration (one pass over `targets`)
    /// before starting the next.
    pub think_time: Duration,
    /// Stop the run early once the overall error rate exceeds this fraction (for example `0.05`
    /// for 5%), but only after at least 50 samples have been recorded, so a handful of early
    /// failures cannot stop a run before it has really started.
    pub stop_on_error_rate: Option<f64>,
    /// Whether each virtual user keeps its connections open and reuses them (keep-alive), like a
    /// real client: one [`brasa_runner::Client`] per virtual user, created when it starts and
    /// kept for the whole run. When `false`, every request opens a new connection (with its TLS
    /// handshake over HTTPS) and closes it, measuring clients that always connect from scratch.
    pub reuse_connections: bool,
}
