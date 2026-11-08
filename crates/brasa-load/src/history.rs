//! Run history: one JSON file per load test run, saved under `<workspace>/.brasa/runs/`.
//!
//! This is a documented exception to "`brasa-workspace` owns the filesystem": `brasa-load` does
//! its own IO here to avoid a `brasa-workspace -> brasa-load -> brasa-runner` dependency
//! edge, which would break the acyclic dependency graph of `AGENTS.md`.
//!
//! A run is stored as `NNNN.json` (the [`RunRecord`]) and, for runs saved by newer versions,
//! `NNNN.samples.csv.gz` next to it (the [`SampleLog`], see [`save_samples`]). The log is a
//! separate file because it grows with the number of requests while the record stays small, and
//! [`list`] must keep reading only the small files.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};

use crate::config::LoadConfig;
use crate::csv_text;
use crate::metrics::LoadSnapshot;
use crate::samples::{SampleLog, SampleRecord};
use crate::status::StatusKey;
use crate::threshold::Threshold;

/// The subfolder run history is stored in, relative to a workspace root. The workspace scanner
/// already skips hidden folders, so this never shows up as a collection.
const RUNS_DIR: &str = ".brasa/runs";

/// Everything that can go wrong reading or writing run history.
#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// A filesystem operation failed: creating `.brasa/runs/`, or reading or writing a run
    /// file.
    #[error("io error at {path:?}: {source}")]
    Io {
        /// The path the operation was attempted on.
        path: PathBuf,
        /// The underlying IO error.
        #[source]
        source: std::io::Error,
    },
    /// A run record could not be serialized to JSON. Should not happen for the plain data this
    /// crate produces; kept as a typed error instead of a panic, matching every other fallible
    /// step in this crate.
    #[error("could not encode {path:?}: {source}")]
    Encode {
        /// The file that was being written.
        path: PathBuf,
        /// The underlying serialization error.
        #[source]
        source: serde_json::Error,
    },
    /// A run file's content is not valid JSON, or not shaped like a [`RunRecord`].
    #[error("could not parse {path:?}: {source}")]
    Decode {
        /// The file that was being read.
        path: PathBuf,
        /// The underlying parse error.
        #[source]
        source: serde_json::Error,
    },
    /// A sample log file is not a gzip of the CSV [`save_samples`] writes.
    #[error("could not parse {path:?} at line {line}: {reason}")]
    Samples {
        /// The file that was being read.
        path: PathBuf,
        /// The 1-based line of the CSV where the problem is.
        line: usize,
        /// What is wrong with it.
        reason: String,
    },
}

/// A [`LoadConfig`] without the actual [`brasa_core::Request`] values, for storing alongside a
/// [`RunRecord`] (a config summary). This avoids
/// needing `serde` support on `brasa-core`'s domain types just to persist history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadConfigSummary {
    /// The number of virtual users the run was configured with.
    pub vus: u32,
    /// The configured run duration, in seconds.
    pub duration_secs: u64,
    /// The configured ramp-up time, in seconds.
    pub ramp_up_secs: u64,
    /// The configured think time, in milliseconds.
    pub think_time_ms: u64,
    /// The configured error rate threshold, if any (a fraction, for example `0.05` for 5%).
    pub stop_on_error_rate: Option<f64>,
    /// How many targets (requests) the run looped over.
    pub target_count: usize,
    /// Whether each virtual user reused its connections ([`LoadConfig::reuse_connections`]).
    /// `#[serde(default)]` (`false`) for a run saved before this field existed: those runs
    /// always opened a new connection per request.
    #[serde(default)]
    pub reuse_connections: bool,
}

impl From<&LoadConfig> for LoadConfigSummary {
    /// Builds a summary from the config a run was started with.
    fn from(config: &LoadConfig) -> Self {
        Self {
            vus: config.vus,
            duration_secs: config.duration.as_secs(),
            ramp_up_secs: config.ramp_up.as_secs(),
            think_time_ms: u64::try_from(config.think_time.as_millis()).unwrap_or(u64::MAX),
            stop_on_error_rate: config.stop_on_error_rate,
            target_count: config.targets.len(),
            reuse_connections: config.reuse_connections,
        }
    }
}

/// A single run's saved history entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    /// The run number, matching its file name (4
    /// digits, zero padded).
    pub number: u32,
    /// When the run started, as Unix seconds.
    pub started_at_unix: u64,
    /// A human label for what the run targeted, for example a request's name, or "3 requests"
    /// for a collection.
    pub target_label: String,
    /// The configuration the run used, without the request bodies (see [`LoadConfigSummary`]).
    pub config: LoadConfigSummary,
    /// The metrics at the moment the run ended.
    pub snapshot: LoadSnapshot,
    /// Whether the run ended before its configured duration fully elapsed (stopped by hand, or
    /// by [`crate::LoadConfig::stop_on_error_rate`]), the same meaning as
    /// [`crate::LoadSummary::stopped_early`]: the app shows this run as "Stopped" when `true`,
    /// "Finished" when `false`. `#[serde(default)]` (`false`) for a run file saved before this
    /// field existed, which falls back to always reading as "Finished", the only status those
    /// older records could show anyway.
    #[serde(default)]
    pub stopped_early: bool,
    /// Each target's HTTP method (as its plain token, e.g. `"GET"`, `"POST"`, or a custom
    /// method exactly as written) and display name, in the same order as
    /// [`crate::LoadSnapshot::per_target`], for the per-request table of a run viewed from
    /// history. A plain `(String, String)` pair rather than [`brasa_core::Method`] directly:
    /// `brasa-core` intentionally has no `serde` dependency (see [`LoadConfigSummary`]'s own
    /// doc comment), and `Method`'s `Display`/`FromStr` round-trip a token losslessly (`FromStr`
    /// is infallible, an unrecognized token becomes `Method::Custom`). `#[serde(default)]`
    /// (empty) for a run file saved before this field existed, which falls back to the generic
    /// "Target N" label those older records already showed.
    #[serde(default)]
    pub target_labels: Vec<(String, String)>,
    /// The goals the run was started with, to check the results against them when it is viewed
    /// from history. `#[serde(default)]` (empty) for a run file saved before this field existed,
    /// which shows no goals, the only thing those older records could show anyway.
    #[serde(default)]
    pub thresholds: Vec<Threshold>,
}

/// A lightweight preview of a [`RunRecord`], for a run list or a "Compare with" dropdown, without
/// reading the heavier fields of the full record (the histogram, the series and the per-target
/// table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecordHeader {
    /// The run number.
    pub number: u32,
    /// When the run started, as Unix seconds.
    pub started_at_unix: u64,
    /// The `target_label` of the underlying [`RunRecord`].
    pub target_label: String,
    /// The total sample count of the underlying [`RunRecord`].
    pub total: u64,
    /// The requests per second of the underlying [`RunRecord`].
    pub rps: f64,
    /// The p95 latency of the underlying [`RunRecord`], in microseconds.
    pub p95: u32,
    /// The error rate of the underlying [`RunRecord`] (`0.0` to `1.0`).
    pub error_rate: f64,
    /// Whether the underlying run reused its connections, see
    /// [`LoadConfigSummary::reuse_connections`].
    #[serde(default)]
    pub reuse_connections: bool,
}

impl From<&RunRecord> for RunRecordHeader {
    /// Extracts the header fields from a full record.
    fn from(record: &RunRecord) -> Self {
        Self {
            number: record.number,
            started_at_unix: record.started_at_unix,
            target_label: record.target_label.clone(),
            total: record.snapshot.total,
            rps: record.snapshot.rps,
            p95: record.snapshot.p95,
            error_rate: record.snapshot.error_rate,
            reuse_connections: record.config.reuse_connections,
        }
    }
}

/// The path a run file for `number` would have under `root`.
fn run_path(root: &Path, number: u32) -> PathBuf {
    root.join(RUNS_DIR).join(format!("{number:04}.json"))
}

/// The path the sample log for `number` would have under `root`.
fn samples_path(root: &Path, number: u32) -> PathBuf {
    root.join(RUNS_DIR)
        .join(format!("{number:04}.samples.csv.gz"))
}

/// The header line of a sample log file.
const SAMPLES_HEADER: &str = "elapsed_ms,vu,target,status,latency_us,size,error,url";

/// Saves `record` to `<root>/.brasa/runs/<NNNN>.json`, creating the folder if it does not
/// exist yet. Overwrites any existing file for the same run number.
pub fn save(root: &Path, record: &RunRecord) -> Result<(), HistoryError> {
    let dir = root.join(RUNS_DIR);
    fs::create_dir_all(&dir).map_err(|source| HistoryError::Io { path: dir, source })?;

    let path = run_path(root, record.number);
    let json = serde_json::to_string_pretty(record).map_err(|source| HistoryError::Encode {
        path: path.clone(),
        source,
    })?;
    fs::write(&path, json).map_err(|source| HistoryError::Io { path, source })
}

/// Loads the run numbered `number` from `<root>/.brasa/runs/`.
pub fn load(root: &Path, number: u32) -> Result<RunRecord, HistoryError> {
    let path = run_path(root, number);
    let text = fs::read_to_string(&path).map_err(|source| HistoryError::Io {
        path: path.clone(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| HistoryError::Decode { path, source })
}

/// Saves the sample log of run `number` to `<root>/.brasa/runs/<NNNN>.samples.csv.gz`, creating
/// the folder if it does not exist yet and overwriting any previous file for that number.
///
/// The file is a gzip of a CSV with the header
/// `elapsed_ms,vu,target,status,latency_us,size,error,url`: `status` is the HTTP code, `timeout`
/// or `failed`, `error` is `0` or `1`, and `url` (the last field, so the rest can be split
/// blindly) is quoted when it contains a comma, a quote or a line break.
pub fn save_samples(root: &Path, number: u32, log: &SampleLog) -> Result<(), HistoryError> {
    let dir = root.join(RUNS_DIR);
    fs::create_dir_all(&dir).map_err(|source| HistoryError::Io { path: dir, source })?;

    let path = samples_path(root, number);
    let io_error = |source| HistoryError::Io {
        path: path.clone(),
        source,
    };
    let file = File::create(&path).map_err(io_error)?;
    let mut encoder = GzEncoder::new(BufWriter::new(file), Compression::default());

    let mut line = String::with_capacity(128);
    encoder
        .write_all(format!("{SAMPLES_HEADER}\n").as_bytes())
        .map_err(io_error)?;
    for sample in log.samples() {
        line.clear();
        push_sample_line(&mut line, sample, log.url(sample.url));
        encoder.write_all(line.as_bytes()).map_err(io_error)?;
    }
    let mut writer = encoder.finish().map_err(io_error)?;
    writer.flush().map_err(io_error)
}

/// Appends the CSV line of `sample` (ending in a line break) to `out`.
fn push_sample_line(out: &mut String, sample: &SampleRecord, url: &str) {
    use std::fmt::Write as _;

    // Writing to a `String` cannot fail.
    let _ = write!(
        out,
        "{},{},{},{},{},{},{},",
        sample.elapsed_ms,
        sample.vu,
        sample.target,
        status_text(sample.status),
        sample.latency_us,
        sample.size,
        u8::from(sample.is_error),
    );
    csv_text::push_field(out, url);
    out.push('\n');
}

/// The `status` column of a sample: the code, `timeout` or `failed`.
pub(crate) fn status_text(status: StatusKey) -> String {
    match status {
        StatusKey::Code(code) => code.to_string(),
        StatusKey::Timeout => "timeout".to_string(),
        StatusKey::Failed => "failed".to_string(),
    }
}

/// Loads the sample log of run `number` saved by [`save_samples`]. `Ok(None)` when there is no
/// such file, which is the case for runs saved by versions that did not keep the samples.
pub fn load_samples(root: &Path, number: u32) -> Result<Option<SampleLog>, HistoryError> {
    let path = samples_path(root, number);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(HistoryError::Io { path, source }),
    };

    let mut lines = BufReader::new(GzDecoder::new(BufReader::new(file)));
    let mut log = SampleLog::new();
    let mut text = String::new();
    let mut line_number = 0;
    loop {
        text.clear();
        // A quoted URL can contain a line break: keep reading until the quotes balance.
        loop {
            let read = lines
                .read_line(&mut text)
                .map_err(|source| HistoryError::Io {
                    path: path.clone(),
                    source,
                })?;
            line_number += 1;
            if read == 0 || !csv_text::is_unterminated(&text) {
                break;
            }
        }
        if text.is_empty() {
            break;
        }
        let record = text.trim_end_matches(['\n', '\r']);
        if line_number == 1 {
            if record != SAMPLES_HEADER {
                return Err(samples_error(&path, 1, "unexpected header"));
            }
            continue;
        }
        let (sample, url) = parse_sample_line(record)
            .ok_or_else(|| samples_error(&path, line_number, "malformed sample"))?;
        let url = log.intern(&url);
        log.push(SampleRecord { url, ..sample });
    }
    Ok(Some(log))
}

/// A [`HistoryError::Samples`] for `path`.
fn samples_error(path: &Path, line: usize, reason: &str) -> HistoryError {
    HistoryError::Samples {
        path: path.to_path_buf(),
        line,
        reason: reason.to_string(),
    }
}

/// Parses one CSV line (without its line break) into a sample whose `url` is still `0`, and the
/// URL text.
fn parse_sample_line(line: &str) -> Option<(SampleRecord, String)> {
    let mut fields = line.splitn(8, ',');
    let elapsed_ms = fields.next()?.parse().ok()?;
    let vu = fields.next()?.parse().ok()?;
    let target = fields.next()?.parse().ok()?;
    let status = match fields.next()? {
        "timeout" => StatusKey::Timeout,
        "failed" => StatusKey::Failed,
        code => StatusKey::Code(code.parse().ok()?),
    };
    let latency_us = fields.next()?.parse().ok()?;
    let size = fields.next()?.parse().ok()?;
    let is_error = match fields.next()? {
        "0" => false,
        "1" => true,
        _ => return None,
    };
    let url = csv_text::unquote(fields.next()?);
    Some((
        SampleRecord {
            elapsed_ms,
            vu,
            target,
            url: 0,
            status,
            latency_us,
            size,
            is_error,
        },
        url,
    ))
}

/// Lists every run saved under `<root>/.brasa/runs/`, newest first (highest run number
/// first). A file that fails to read or parse is skipped rather than failing the whole listing:
/// one corrupt run should not hide every other one. Returns an empty list if the folder does not
/// exist yet. Only the `.json` files are read: the sample logs next to them are not runs.
#[must_use]
pub fn list(root: &Path) -> Vec<RunRecordHeader> {
    let dir = root.join(RUNS_DIR);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut headers: Vec<RunRecordHeader> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            let text = fs::read_to_string(entry.path()).ok()?;
            let record: RunRecord = serde_json::from_str(&text).ok()?;
            Some(RunRecordHeader::from(&record))
        })
        .collect();

    headers.sort_by_key(|header| std::cmp::Reverse(header.number));
    headers
}

/// The next run number to use under `<root>/.brasa/runs/`: one more than the highest existing
/// run file's number, or `1` if the folder does not exist or has none.
///
/// Based only on file names, not their content, so a corrupt file still reserves its number and
/// is never silently overwritten by [`save`]. A sample log (`NNNN.samples.csv.gz`) reserves its
/// number too.
#[must_use]
pub fn next_number(root: &Path) -> u32 {
    let dir = root.join(RUNS_DIR);
    let Ok(entries) = fs::read_dir(&dir) else {
        return 1;
    };

    let highest = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            path.file_name()?
                .to_str()?
                .split('.')
                .next()?
                .parse::<u32>()
                .ok()
        })
        .max();

    highest.map_or(1, |number| number + 1)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use super::*;
    use crate::config::LoadTarget;
    use crate::metrics::{ErrorExample, LoadSnapshot, SecondPoint, TargetStats};
    use crate::status::StatusKey;
    use crate::threshold::ThresholdMetric;

    /// A minimal but structurally complete snapshot, for tests that only care about history
    /// plumbing, not the metrics themselves.
    fn sample_snapshot() -> LoadSnapshot {
        LoadSnapshot {
            elapsed: Duration::from_secs(1),
            active_vus: 4,
            total: 10,
            rps: 10.0,
            p50: 1_000,
            p95: 2_000,
            p99: 3_000,
            error_rate: 0.1,
            series: vec![SecondPoint {
                rps: 10.0,
                p95: 2_000,
                p50: 1_000,
                p90: 1_800,
                p99: 3_000,
                vus: 4,
            }],
            histogram: [0; 26],
            status_counts: vec![(StatusKey::Code(200), 9), (StatusKey::Code(500), 1)],
            per_target: vec![TargetStats {
                count: 10,
                p50: 1_000,
                p95: 2_000,
                p99: 3_000,
                error_rate: 0.1,
            }],
            error_examples: vec![ErrorExample {
                status: StatusKey::Code(500),
                text: "HTTP 500".to_string(),
            }],
        }
    }

    fn sample_record(number: u32) -> RunRecord {
        let config = LoadConfig {
            targets: vec![LoadTarget::new("login", brasa_core::Request::default())],
            vus: 4,
            duration: Duration::from_secs(60),
            ramp_up: Duration::from_secs(10),
            think_time: Duration::from_millis(500),
            stop_on_error_rate: Some(0.05),
            reuse_connections: true,
        };
        RunRecord {
            number,
            started_at_unix: 1_700_000_000,
            target_label: "login".to_string(),
            config: LoadConfigSummary::from(&config),
            snapshot: sample_snapshot(),
            stopped_early: false,
            target_labels: vec![("POST".to_string(), "login".to_string())],
            thresholds: vec![Threshold {
                metric: ThresholdMetric::P99,
                limit: 50.0,
            }],
        }
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempdir().expect("temp dir");
        let record = sample_record(1);

        save(dir.path(), &record).expect("save should succeed");
        let loaded = load(dir.path(), 1).expect("load should succeed");
        assert_eq!(loaded, record);
    }

    #[test]
    fn save_writes_a_four_digit_zero_padded_file_name() {
        let dir = tempdir().expect("temp dir");
        save(dir.path(), &sample_record(7)).expect("save should succeed");
        assert!(dir.path().join(".brasa/runs/0007.json").is_file());
    }

    #[test]
    fn next_number_is_one_for_an_empty_or_missing_history() {
        let dir = tempdir().expect("temp dir");
        assert_eq!(next_number(dir.path()), 1);
    }

    #[test]
    fn next_number_is_one_more_than_the_highest_existing_run() {
        let dir = tempdir().expect("temp dir");
        save(dir.path(), &sample_record(1)).expect("save should succeed");
        save(dir.path(), &sample_record(3)).expect("save should succeed");
        assert_eq!(next_number(dir.path()), 4);
    }

    #[test]
    fn list_is_newest_first() {
        let dir = tempdir().expect("temp dir");
        save(dir.path(), &sample_record(1)).expect("save should succeed");
        save(dir.path(), &sample_record(2)).expect("save should succeed");
        save(dir.path(), &sample_record(3)).expect("save should succeed");

        let headers = list(dir.path());
        let numbers: Vec<u32> = headers.iter().map(|header| header.number).collect();
        assert_eq!(numbers, vec![3, 2, 1]);
    }

    #[test]
    fn list_skips_a_corrupt_file_but_keeps_the_others() {
        let dir = tempdir().expect("temp dir");
        save(dir.path(), &sample_record(1)).expect("save should succeed");
        save(dir.path(), &sample_record(2)).expect("save should succeed");
        fs::write(dir.path().join(".brasa/runs/0003.json"), "not json").expect("write junk");

        let headers = list(dir.path());
        let numbers: Vec<u32> = headers.iter().map(|header| header.number).collect();
        assert_eq!(numbers, vec![2, 1]);
    }

    #[test]
    fn next_number_still_counts_a_corrupt_file_by_its_name() {
        let dir = tempdir().expect("temp dir");
        save(dir.path(), &sample_record(1)).expect("save should succeed");
        fs::create_dir_all(dir.path().join(".brasa/runs")).expect("create runs dir");
        fs::write(dir.path().join(".brasa/runs/0002.json"), "not json").expect("write junk");

        assert_eq!(next_number(dir.path()), 3);
    }

    #[test]
    fn load_of_a_missing_run_is_an_error() {
        let dir = tempdir().expect("temp dir");
        assert!(matches!(load(dir.path(), 99), Err(HistoryError::Io { .. })));
    }

    #[test]
    fn list_of_a_workspace_with_no_runs_folder_is_empty() {
        let dir = tempdir().expect("temp dir");
        assert!(list(dir.path()).is_empty());
    }

    #[test]
    fn round_trip_preserves_stopped_early_and_target_labels() {
        let dir = tempdir().expect("temp dir");
        let mut record = sample_record(1);
        record.stopped_early = true;
        record.target_labels = vec![
            ("POST".to_string(), "create".to_string()),
            ("GET".to_string(), "list".to_string()),
        ];

        save(dir.path(), &record).expect("save should succeed");
        let loaded = load(dir.path(), 1).expect("load should succeed");

        assert!(loaded.stopped_early);
        assert_eq!(
            loaded.target_labels,
            vec![
                ("POST".to_string(), "create".to_string()),
                ("GET".to_string(), "list".to_string()),
            ]
        );
    }

    #[test]
    fn a_run_saved_before_reuse_connections_existed_did_not_reuse_them() {
        let dir = tempdir().expect("temp dir");
        let mut value = serde_json::to_value(sample_record(1)).expect("serialize");
        value["config"]
            .as_object_mut()
            .expect("config serializes as an object")
            .remove("reuse_connections");
        fs::create_dir_all(dir.path().join(".brasa/runs")).expect("create runs dir");
        fs::write(
            dir.path().join(".brasa/runs/0001.json"),
            serde_json::to_string_pretty(&value).expect("serialize back to text"),
        )
        .expect("write old record");

        assert!(!load(dir.path(), 1).expect("loads").config.reuse_connections);
        assert!(!list(dir.path())[0].reuse_connections);
        // A current record keeps it.
        save(dir.path(), &sample_record(2)).expect("save should succeed");
        assert!(list(dir.path())[0].reuse_connections);
    }

    #[test]
    fn a_run_file_saved_before_stopped_early_and_target_labels_existed_still_loads() {
        let dir = tempdir().expect("temp dir");
        // The exact shape `save` wrote before its newer fields existed: everything
        // `RunRecord` still has, nothing it gained since.
        let mut value = serde_json::to_value(sample_record(1)).expect("serialize");
        let object = value
            .as_object_mut()
            .expect("record serializes as an object");
        object.remove("stopped_early");
        object.remove("target_labels");
        let old_json = serde_json::to_string_pretty(&value).expect("serialize back to text");

        fs::create_dir_all(dir.path().join(".brasa/runs")).expect("create runs dir");
        fs::write(dir.path().join(".brasa/runs/0001.json"), old_json).expect("write old record");

        let loaded = load(dir.path(), 1).expect("an old record without these fields still loads");
        assert!(
            !loaded.stopped_early,
            "should default to false (\"Finished\")"
        );
        assert!(
            loaded.target_labels.is_empty(),
            "should default to empty (falls back to the generic \"Target N\" label)"
        );
    }

    /// A log with one sample of every status kind and URLs that need CSV quoting.
    fn sample_log() -> SampleLog {
        let mut log = SampleLog::new();
        let plain = log.intern("http://t/users?page=1");
        let tricky = log.intern("http://t/search?q=a,b&name=\"x\"");
        let multiline = log.intern("http://t/a\nb");
        let rows = [
            (0, 0, 0, plain, StatusKey::Code(200), 1_500, 312),
            (15, 1, 1, tricky, StatusKey::Code(500), 40_000, 64),
            (1_020, 2, 0, tricky, StatusKey::Timeout, 5_000_000, 0),
            (1_900, 2, 1, multiline, StatusKey::Failed, 12, 0),
            (2_000, 0, 0, plain, StatusKey::Code(201), 900, 0),
        ];
        for (elapsed_ms, vu, target, url, status, latency_us, size) in rows {
            log.push(SampleRecord {
                elapsed_ms,
                vu,
                target,
                url,
                status,
                latency_us,
                size,
                is_error: status.is_error(),
            });
        }
        log
    }

    #[test]
    fn samples_round_trip_with_quoted_urls() {
        let dir = tempdir().expect("temp dir");
        let log = sample_log();

        save_samples(dir.path(), 3, &log).expect("save should succeed");
        assert!(
            dir.path()
                .join(".brasa/runs/0003.samples.csv.gz")
                .is_file()
        );
        let loaded = load_samples(dir.path(), 3)
            .expect("load should succeed")
            .expect("the file exists");
        assert_eq!(loaded, log);
        assert_eq!(
            loaded.url(loaded.samples()[1].url),
            "http://t/search?q=a,b&name=\"x\""
        );
        assert_eq!(loaded.url(loaded.samples()[3].url), "http://t/a\nb");
    }

    #[test]
    fn the_saved_csv_has_the_documented_header_and_columns() {
        use std::io::Read;

        let dir = tempdir().expect("temp dir");
        save_samples(dir.path(), 1, &sample_log()).expect("save should succeed");
        let file = File::open(dir.path().join(".brasa/runs/0001.samples.csv.gz")).expect("file");
        let mut text = String::new();
        GzDecoder::new(file)
            .read_to_string(&mut text)
            .expect("valid gzip");

        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some("elapsed_ms,vu,target,status,latency_us,size,error,url")
        );
        assert_eq!(
            lines.next(),
            Some("0,0,0,200,1500,312,0,http://t/users?page=1")
        );
        assert_eq!(
            lines.next(),
            Some("15,1,1,500,40000,64,1,\"http://t/search?q=a,b&name=\"\"x\"\"\"")
        );
        assert_eq!(
            lines.next(),
            Some("1020,2,0,timeout,5000000,0,1,\"http://t/search?q=a,b&name=\"\"x\"\"\"")
        );
    }

    #[test]
    fn an_empty_log_round_trips() {
        let dir = tempdir().expect("temp dir");
        save_samples(dir.path(), 1, &SampleLog::new()).expect("save should succeed");
        let loaded = load_samples(dir.path(), 1).expect("load").expect("exists");
        assert!(loaded.is_empty());
    }

    #[test]
    fn a_run_without_a_sample_file_loads_as_none() {
        let dir = tempdir().expect("temp dir");
        assert!(load_samples(dir.path(), 1).expect("no error").is_none());
        save(dir.path(), &sample_record(1)).expect("save should succeed");
        assert!(load_samples(dir.path(), 1).expect("no error").is_none());
    }

    #[test]
    fn a_corrupt_sample_file_is_a_decode_error_with_its_line() {
        let dir = tempdir().expect("temp dir");
        fs::create_dir_all(dir.path().join(".brasa/runs")).expect("create runs dir");
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(
                format!("{SAMPLES_HEADER}\n0,0,0,200,1,1,0,http://t/\nnot,a,sample\n").as_bytes(),
            )
            .expect("write");
        let bytes = encoder.finish().expect("finish");
        fs::write(dir.path().join(".brasa/runs/0001.samples.csv.gz"), bytes).expect("write file");

        assert!(matches!(
            load_samples(dir.path(), 1),
            Err(HistoryError::Samples { line: 3, .. })
        ));

        fs::write(
            dir.path().join(".brasa/runs/0002.samples.csv.gz"),
            b"junk",
        )
        .expect("write");
        assert!(matches!(
            load_samples(dir.path(), 2),
            Err(HistoryError::Io { .. })
        ));
    }

    #[test]
    fn list_and_next_number_work_with_sample_files_next_to_the_json() {
        let dir = tempdir().expect("temp dir");
        save(dir.path(), &sample_record(1)).expect("save should succeed");
        save_samples(dir.path(), 1, &sample_log()).expect("save samples");
        save(dir.path(), &sample_record(2)).expect("save should succeed");
        save_samples(dir.path(), 2, &sample_log()).expect("save samples");
        // A log whose JSON was lost still reserves its number.
        save_samples(dir.path(), 5, &sample_log()).expect("save samples");

        let numbers: Vec<u32> = list(dir.path())
            .iter()
            .map(|header| header.number)
            .collect();
        assert_eq!(numbers, vec![2, 1]);
        assert_eq!(next_number(dir.path()), 6);
    }

    #[test]
    fn a_run_file_saved_before_thresholds_existed_still_loads() {
        let dir = tempdir().expect("temp dir");
        let mut value = serde_json::to_value(sample_record(1)).expect("serialize");
        value
            .as_object_mut()
            .expect("record serializes as an object")
            .remove("thresholds");
        let snapshot = value["snapshot"].as_object_mut().expect("snapshot object");
        snapshot.remove("error_examples");
        for point in snapshot["series"].as_array_mut().expect("series") {
            let point = point.as_object_mut().expect("point");
            for key in ["p50", "p90", "p99", "vus"] {
                point.remove(key);
            }
        }
        fs::create_dir_all(dir.path().join(".brasa/runs")).expect("create runs dir");
        fs::write(
            dir.path().join(".brasa/runs/0001.json"),
            serde_json::to_string_pretty(&value).expect("serialize back to text"),
        )
        .expect("write old record");

        let loaded = load(dir.path(), 1).expect("an old record still loads");
        assert!(loaded.thresholds.is_empty());
        assert!(loaded.snapshot.error_examples.is_empty());
        assert_eq!(loaded.snapshot.series[0].p95, 2_000);
        assert_eq!(loaded.snapshot.series[0].vus, 0);
        assert_eq!(list(dir.path()).len(), 1);
    }
}
