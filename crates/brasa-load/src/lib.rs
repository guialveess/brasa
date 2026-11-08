//! The load test engine: run a request or a collection with N virtual users, aggregate live
//! metrics, and keep a run history.
//!
//! [`LoadRun::start`] is the entry point: given a [`LoadConfig`], the active
//! [`brasa_core::Environment`], a [`brasa_runner::SessionEnv`] and a shared
//! [`brasa_runner::Runner`], it starts the run in the background and returns immediately. The
//! caller polls [`LoadRun::snapshot`] for live metrics (for example every 250 ms from the UI),
//! calls [`LoadRun::stop`] to end it early, and [`LoadRun::join`] to block for the final
//! [`LoadSummary`]. The [`history`] module then saves and lists runs, and [`compare`] turns two
//! [`LoadSnapshot`]s into the signed deltas the "Compare with" panel shows.
//!
//! Besides the aggregates of a [`LoadSnapshot`], a run keeps a [`SampleLog`], one compact record
//! per completed request. [`LoadRun::read_samples`] copies the new part of it while the run goes
//! on, [`LoadSummary::log`] holds all of it at the end, [`window::window_stats`] computes the
//! statistics of any time window of it, with the histogram, the statuses and the error groups,
//! [`latency::latency_view`] the latency per request (histogram, percentiles and curve),
//! [`history::save_samples`] stores it next to the run and [`export`] writes it as CSV or JSON.
//!
//! # Concurrency model
//!
//! A run is: one supervisor thread (started by [`LoadRun::start`]), one plain OS thread per
//! virtual user (`ureq`, used by [`brasa_http::send`] through [`brasa_runner::Runner`], is
//! blocking, so a thread per virtual user is the simplest way to run many of them at once), and
//! the caller's own thread reading snapshots and calling `stop`.
//!
//! Everything these threads share is behind an [`std::sync::atomic`] type or a plain
//! [`std::sync::Mutex`], no channels, no async runtime:
//!
//! - `stop: AtomicBool` is how [`LoadRun::stop`], the duration timeout and the
//!   [`LoadConfig::stop_on_error_rate`] threshold all signal "end the run" to every thread. Every
//!   wait (between virtual user starts, during `think_time`) is a loop of short sleeps checking
//!   this flag, not one long `thread::sleep`, so stopping takes effect within a fraction of a
//!   second instead of waiting out whatever was left.
//! - `active_vus: AtomicU32` is incremented when a virtual user thread starts and decremented
//!   when it ends, read by [`LoadRun::snapshot`] with no locking needed.
//! - `metrics: Mutex<Aggregator>` is the one piece of real shared, mutable state: every virtual
//!   user thread locks it briefly to record one sample: push a few numbers, update a few
//!   counters, append the request to the [`SampleLog`] that lives inside the aggregator (so the
//!   log needs no lock of its own and always agrees with the counters), then unlock.
//!   [`LoadRun::snapshot`] locks it too, to copy whatever has been recorded so far, and sorts
//!   the copies into a [`LoadSnapshot`] after unlocking, and so does [`LoadRun::read_samples`], to copy only the samples the
//!   caller's own log does not have yet. Nothing panics while holding this lock (see
//!   `src/run.rs`'s `lock_metrics`), so a poisoned lock is not expected; if it ever happened
//!   anyway, the lock is still recovered rather than propagating the panic to every virtual user
//!   thread, since a live metrics view being slightly stale is far less disruptive than the
//!   whole run dying.
//!
//! Each virtual user owns its own [`brasa_runner::SessionEnv`] (cloned once from the one
//! [`LoadRun::start`] was given), never shared with another virtual user: this is what makes "a
//! login token a post script sets flows to the next request of the same virtual user" work without
//! any locking at all, since only one thread ever touches it.
#![warn(missing_docs)]

mod compare;
mod config;
mod csv_text;
pub mod export;
pub mod history;
pub mod latency;
mod metrics;
pub mod progress;
mod run;
pub mod samples;
mod status;
mod threshold;
pub mod window;

pub use compare::{Delta, DeltaUnit, compare};
pub use config::{LoadConfig, LoadTarget};
pub use latency::{
    LatencyOptions, LatencyPercentiles, LatencyScale, LatencyView, StackedHistogram, latency_view,
    latency_view_on,
};
pub use metrics::{ErrorExample, LoadSnapshot, SecondPoint, TargetStats};
pub use progress::Progress;
pub use run::{LoadRun, LoadSummary, SnapshotReader};
pub use samples::{SampleLog, SampleRecord};
pub use status::StatusKey;
pub use threshold::{
    Threshold, ThresholdInputs, ThresholdMetric, ThresholdOutcome, evaluate as evaluate_thresholds,
};
pub use window::{ErrorGroup, LatencyHistogram, WindowStats, window_stats, window_stats_with};
