//! [`run_json`], a whole run as pretty printed JSON: the numbers of a time window, the series, the
//! histogram, the errors and, when the sample log is at hand, every request.

use std::ops::Range;

use serde::ser::{Error as _, SerializeSeq as _};
use serde::{Serialize, Serializer};

use super::csv::{target_label, window_rows};
use crate::history::{LoadConfigSummary, RunRecord, status_text};
use crate::progress::{Progress, Ticker};
use crate::samples::SampleLog;
use crate::threshold::{ThresholdMetric, evaluate};
use crate::window::WindowStats;

#[derive(Serialize)]
struct RunJson<'a> {
    run: RunInfo<'a>,
    summary: Summary,
    thresholds: Vec<ThresholdJson>,
    series: Vec<SeriesJson>,
    histogram: HistogramJson<'a>,
    statuses: Vec<StatusJson>,
    per_request: Vec<PerRequestJson<'a>>,
    errors: Vec<ErrorJson<'a>>,
    requests: TrackedRequests<'a>,
}

/// The requests of the document, reporting each one to a [`Progress`] as it is written: pretty
/// printing them is most of the time of a big export. Fails once the job is cancelled.
struct TrackedRequests<'a> {
    rows: Vec<RequestJson<'a>>,
    progress: &'a Progress,
}

impl Serialize for TrackedRequests<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.rows.len()))?;
        let mut ticker = Ticker::new(self.progress);
        for row in &self.rows {
            if !ticker.tick() {
                return Err(S::Error::custom("cancelled"));
            }
            seq.serialize_element(row)?;
        }
        seq.end()
    }
}

#[derive(Serialize)]
struct RunInfo<'a> {
    number: u32,
    started_at_unix: u64,
    target: &'a str,
    config: &'a LoadConfigSummary,
    stopped_early: bool,
    window: Option<WindowJson>,
}

#[derive(Serialize)]
struct WindowJson {
    start_s: u32,
    end_s: u32,
}

#[derive(Serialize)]
struct Summary {
    total: u64,
    rps: f64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    error_rate: f64,
}

#[derive(Serialize)]
struct ThresholdJson {
    metric: ThresholdMetric,
    limit: f64,
    value: f64,
    passed: bool,
}

#[derive(Serialize)]
struct SeriesJson {
    second: u32,
    rps: f64,
    vus: u32,
    p50_ms: f64,
    p90_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
}

#[derive(Serialize)]
struct HistogramJson<'a> {
    bucket_ms: f64,
    counts: &'a [u64],
}

#[derive(Serialize)]
struct StatusJson {
    status: String,
    count: u64,
}

#[derive(Serialize)]
struct PerRequestJson<'a> {
    method: &'a str,
    name: String,
    count: u64,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    error_rate: f64,
}

#[derive(Serialize)]
struct ErrorJson<'a> {
    status: String,
    count: u64,
    first_ms: u32,
    last_ms: u32,
    example: Option<&'a str>,
}

#[derive(Serialize)]
struct RequestJson<'a> {
    id: usize,
    time_ms: u32,
    vu: u32,
    method: &'a str,
    request: &'a str,
    url: &'a str,
    status: String,
    latency_ms: f64,
    size_bytes: u32,
    error: bool,
}

/// Microseconds as milliseconds.
fn ms(us: u32) -> f64 {
    f64::from(us) / 1_000.0
}

/// `record` as pretty printed JSON, with the numbers of `stats` (the statistics of `range`, see
/// [`crate::window::window_stats`]), so exporting a selected stretch of the run exports that
/// stretch.
///
/// The top level keys, all in English:
///
/// - `run`: `number`, `started_at_unix`, `target`, `config`, `stopped_early` and `window`
///   (`{start_s, end_s}` for a range, `null` for the whole run).
/// - `summary`: `total`, `rps`, `p50_ms`, `p95_ms`, `p99_ms`, `error_rate`.
/// - `thresholds`: `metric`, `limit`, `value` and `passed` for each threshold of the record,
///   checked against `stats`.
/// - `series`: `second`, `rps`, `vus`, `p50_ms`, `p90_ms`, `p95_ms` and `p99_ms` of each second
///   of the record's series inside the window.
/// - `histogram`: `bucket_ms` and `counts`.
/// - `statuses`: `status` and `count`. A status is a string everywhere in this file: the HTTP
///   code (`"200"`), `"timeout"` or `"failed"`.
/// - `per_request`: `method`, `name`, `count`, `p50_ms`, `p95_ms`, `p99_ms`, `error_rate`.
/// - `errors`: `status`, `count`, `first_ms`, `last_ms` and `example`, the text of the first
///   error of that status in the run (`null` when the record has none).
/// - `requests`: every sample of `log` inside the window with the same fields as
///   [`super::csv::requests_csv`] (`error` is a boolean here). An empty array when `log` is
///   `None`, which is the case for a run saved before the samples were kept.
#[must_use]
pub fn run_json(
    record: &RunRecord,
    log: Option<&SampleLog>,
    stats: &WindowStats,
    range: Option<Range<u32>>,
) -> String {
    build(record, log, stats, range, &Progress::default())
}

/// [`run_json`] reporting to `progress` each sample of the log as it gathers the requests, then
/// each request as it writes it (twice the log for the whole run), `None` when it is cancelled.
#[must_use]
pub fn run_json_with(
    record: &RunRecord,
    log: Option<&SampleLog>,
    stats: &WindowStats,
    range: Option<Range<u32>>,
    progress: &Progress,
) -> Option<String> {
    let json = build(record, log, stats, range, progress);
    (!progress.is_cancelled()).then_some(json)
}

/// [`run_json_with`]; when cancelled, the document is skipped.
fn build(
    record: &RunRecord,
    log: Option<&SampleLog>,
    stats: &WindowStats,
    range: Option<Range<u32>>,
    progress: &Progress,
) -> String {
    let in_window = |second: u32| range.as_ref().is_none_or(|range| range.contains(&second));

    let thresholds = evaluate(&record.thresholds, &stats.threshold_inputs())
        .into_iter()
        .map(|outcome| ThresholdJson {
            metric: outcome.threshold.metric,
            limit: outcome.threshold.limit,
            value: outcome.value,
            passed: outcome.passed,
        })
        .collect();

    let series = record
        .snapshot
        .series
        .iter()
        .enumerate()
        .filter_map(|(second, point)| {
            let second = u32::try_from(second).ok()?;
            in_window(second).then_some(SeriesJson {
                second,
                rps: point.rps,
                vus: point.vus,
                p50_ms: ms(point.p50),
                p90_ms: ms(point.p90),
                p95_ms: ms(point.p95),
                p99_ms: ms(point.p99),
            })
        })
        .collect();

    let per_request = stats
        .per_target
        .iter()
        .enumerate()
        .map(|(index, target)| {
            let (method, name) = record
                .target_labels
                .get(index)
                .map_or(("", format!("Target {}", index + 1)), |(method, name)| {
                    (method.as_str(), name.clone())
                });
            PerRequestJson {
                method,
                name,
                count: target.count,
                p50_ms: ms(target.p50),
                p95_ms: ms(target.p95),
                p99_ms: ms(target.p99),
                error_rate: target.error_rate,
            }
        })
        .collect();

    let errors = stats
        .error_groups
        .iter()
        .map(|group| ErrorJson {
            status: status_text(group.status),
            count: group.count,
            first_ms: group.first_ms,
            last_ms: group.last_ms,
            example: record
                .snapshot
                .error_examples
                .iter()
                .find(|example| example.status == group.status)
                .map(|example| example.text.as_str()),
        })
        .collect();

    let mut ticker = Ticker::new(progress);
    let requests = log.map_or_else(Vec::new, |log| {
        window_rows(log, range.as_ref(), &mut ticker)
            .map(|(id, sample)| {
                let (method, request) = target_label(&record.target_labels, sample.target);
                RequestJson {
                    id,
                    time_ms: sample.elapsed_ms,
                    vu: sample.vu + 1,
                    method,
                    request,
                    url: log.url(sample.url),
                    status: status_text(sample.status),
                    latency_ms: f64::from(sample.latency_us) / 1_000.0,
                    size_bytes: sample.size,
                    error: sample.is_error,
                }
            })
            .collect()
    });

    let export = RunJson {
        run: RunInfo {
            number: record.number,
            started_at_unix: record.started_at_unix,
            target: &record.target_label,
            config: &record.config,
            stopped_early: record.stopped_early,
            window: range.as_ref().map(|range| WindowJson {
                start_s: range.start,
                end_s: range.end,
            }),
        },
        summary: Summary {
            total: stats.total,
            rps: stats.rps,
            p50_ms: ms(stats.p50),
            p95_ms: ms(stats.p95),
            p99_ms: ms(stats.p99),
            error_rate: stats.error_rate,
        },
        thresholds,
        series,
        histogram: HistogramJson {
            bucket_ms: ms(stats.histogram.bucket_us),
            counts: &stats.histogram.counts,
        },
        statuses: stats
            .statuses
            .iter()
            .map(|(status, count)| StatusJson {
                status: status_text(*status),
                count: *count,
            })
            .collect(),
        per_request,
        errors,
        requests: TrackedRequests {
            rows: requests,
            progress,
        },
    };
    drop(ticker);
    if progress.is_cancelled() {
        return String::new();
    }
    // Plain data with string keys cannot fail to serialize; an empty document is the fallback
    // instead of a panic.
    serde_json::to_string_pretty(&export).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::time::Duration;

    use pretty_assertions::assert_eq;
    use serde_json::Value;

    use super::*;
    use crate::config::{LoadConfig, LoadTarget};
    use crate::metrics::{ErrorExample, LoadSnapshot, SecondPoint, TargetStats};
    use crate::samples::SampleRecord;
    use crate::status::StatusKey;
    use crate::threshold::Threshold;
    use crate::window::window_stats;

    fn log() -> SampleLog {
        let mut log = SampleLog::new();
        let create = log.intern("http://t/users");
        let list = log.intern("http://t/users?page=2");
        let rows = [
            (100, 0, 0, create, StatusKey::Code(201), 10_000, 312),
            (900, 1, 1, list, StatusKey::Code(200), 20_000, 4_210),
            (1_100, 0, 0, create, StatusKey::Code(429), 2_000, 64),
            (1_500, 1, 1, list, StatusKey::Timeout, 5_000_000, 0),
            (2_200, 0, 0, create, StatusKey::Code(201), 12_000, 312),
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

    fn record() -> RunRecord {
        let config = LoadConfig {
            targets: vec![
                LoadTarget::new("create", brasa_core::Request::default()),
                LoadTarget::new("list", brasa_core::Request::default()),
            ],
            vus: 2,
            duration: Duration::from_secs(3),
            ramp_up: Duration::from_secs(1),
            think_time: Duration::ZERO,
            stop_on_error_rate: None,
            reuse_connections: true,
        };
        let point = |rps, vus| SecondPoint {
            rps,
            p95: 20_000,
            p50: 10_000,
            p90: 18_000,
            p99: 30_000,
            vus,
        };
        RunRecord {
            number: 7,
            started_at_unix: 1_700_000_000,
            target_label: "2 requests".to_string(),
            config: LoadConfigSummary::from(&config),
            snapshot: LoadSnapshot {
                elapsed: Duration::from_millis(3_000),
                active_vus: 0,
                total: 5,
                rps: 5.0 / 3.0,
                p50: 12_000,
                p95: 5_000_000,
                p99: 5_000_000,
                error_rate: 0.4,
                series: vec![point(2.0, 1), point(2.0, 2), point(1.0, 2)],
                histogram: [0; 26],
                status_counts: Vec::new(),
                per_target: vec![TargetStats {
                    count: 3,
                    p50: 10_000,
                    p95: 12_000,
                    p99: 12_000,
                    error_rate: 0.0,
                }],
                error_examples: vec![
                    ErrorExample {
                        status: StatusKey::Code(429),
                        text: "HTTP 429 \u{b7} slow down".to_string(),
                    },
                    ErrorExample {
                        status: StatusKey::Timeout,
                        text: "timed out".to_string(),
                    },
                ],
            },
            stopped_early: true,
            target_labels: vec![
                ("POST".to_string(), "create".to_string()),
                ("GET".to_string(), "list".to_string()),
            ],
            thresholds: vec![
                Threshold {
                    metric: ThresholdMetric::P99,
                    limit: 50.0,
                },
                Threshold {
                    metric: ThresholdMetric::Rps,
                    limit: 1.0,
                },
            ],
        }
    }

    fn parse(json: &str) -> Value {
        serde_json::from_str(json).expect("the export is valid JSON")
    }

    #[test]
    fn the_progress_counts_the_log_twice_and_a_cancel_drops_the_document() {
        let log = log();
        let stats = window_stats(&log, 2, 3, None, 3.0);
        let progress = Progress::new(1);
        progress.begin_step(0, 2 * log.len() as u64);
        let json = run_json_with(&record(), Some(&log), &stats, None, &progress);
        assert_eq!(
            json.map(|json| parse(&json)["requests"].as_array().map(Vec::len)),
            Some(Some(5))
        );
        assert_eq!(progress.fraction(), 1.0);

        progress.cancel();
        assert_eq!(
            run_json_with(&record(), Some(&log), &stats, None, &progress),
            None
        );
    }

    #[test]
    fn the_whole_run_exports_every_section() {
        let log = log();
        let stats = window_stats(&log, 2, 3, None, 3.0);
        let json = run_json(&record(), Some(&log), &stats, None);
        let value = parse(&json);

        assert_eq!(value["run"]["number"], 7);
        assert_eq!(value["run"]["started_at_unix"], 1_700_000_000u64);
        assert_eq!(value["run"]["target"], "2 requests");
        assert_eq!(value["run"]["stopped_early"], true);
        assert_eq!(value["run"]["window"], Value::Null);
        assert_eq!(value["run"]["config"]["vus"], 2);

        assert_eq!(value["summary"]["total"], 5);
        assert_eq!(value["summary"]["p50_ms"], 12.0);
        assert_eq!(value["summary"]["p99_ms"], 5_000.0);
        assert_eq!(value["summary"]["error_rate"], 0.4);

        // p99 is 5000 ms (fails < 50) and 5 requests over 3 s reach 1 req/s (passes).
        let thresholds = value["thresholds"].as_array().expect("array");
        assert_eq!(thresholds.len(), 2);
        assert_eq!(thresholds[0]["metric"], "p99");
        assert_eq!(thresholds[0]["limit"], 50.0);
        assert_eq!(thresholds[0]["value"], 5_000.0);
        assert_eq!(thresholds[0]["passed"], false);
        assert_eq!(thresholds[1]["metric"], "rps");
        assert_eq!(thresholds[1]["passed"], true);

        let series = value["series"].as_array().expect("array");
        assert_eq!(series.len(), 3);
        assert_eq!(series[1]["second"], 1);
        assert_eq!(series[1]["vus"], 2);
        assert_eq!(series[1]["p90_ms"], 18.0);

        assert_eq!(
            value["histogram"]["counts"]
                .as_array()
                .expect("array")
                .len(),
            26
        );
        assert_eq!(
            value["histogram"]["bucket_ms"],
            f64::from(stats.histogram.bucket_us) / 1_000.0
        );

        let statuses = value["statuses"].as_array().expect("array");
        assert_eq!(statuses[0]["status"], "200");
        assert_eq!(statuses.last().expect("one")["status"], "timeout");

        let per_request = value["per_request"].as_array().expect("array");
        assert_eq!(per_request[0]["method"], "POST");
        assert_eq!(per_request[0]["name"], "create");
        assert_eq!(per_request[0]["count"], 3);
        assert_eq!(per_request[1]["name"], "list");

        let errors = value["errors"].as_array().expect("array");
        assert_eq!(errors.len(), 2);
        let timeout = errors
            .iter()
            .find(|e| e["status"] == "timeout")
            .expect("timeout group");
        assert_eq!(timeout["example"], "timed out");
        assert_eq!(timeout["first_ms"], 1_500);
        let too_many = errors
            .iter()
            .find(|e| e["status"] == "429")
            .expect("429 group");
        assert_eq!(too_many["example"], "HTTP 429 \u{b7} slow down");

        let requests = value["requests"].as_array().expect("array");
        assert_eq!(requests.len(), 5);
        assert_eq!(requests[0]["id"], 1);
        assert_eq!(requests[0]["vu"], 1);
        assert_eq!(requests[0]["method"], "POST");
        assert_eq!(requests[0]["request"], "create");
        assert_eq!(requests[0]["url"], "http://t/users");
        assert_eq!(requests[0]["status"], "201");
        assert_eq!(requests[0]["latency_ms"], 10.0);
        assert_eq!(requests[0]["size_bytes"], 312);
        assert_eq!(requests[0]["error"], false);
        assert_eq!(requests[3]["status"], "timeout");
        assert_eq!(requests[3]["error"], true);
    }

    #[test]
    fn a_window_restricts_the_numbers_the_series_and_the_requests() {
        let log = log();
        let stats = window_stats(&log, 2, 3, Some(1..2), 3.0);
        let value = parse(&run_json(&record(), Some(&log), &stats, Some(1..2)));

        assert_eq!(value["run"]["window"]["start_s"], 1);
        assert_eq!(value["run"]["window"]["end_s"], 2);
        assert_eq!(value["summary"]["total"], 2);
        assert_eq!(value["summary"]["rps"], 2.0);
        assert_eq!(value["series"].as_array().expect("array").len(), 1);
        assert_eq!(value["series"][0]["second"], 1);
        let requests = value["requests"].as_array().expect("array");
        let ids: Vec<&Value> = requests.iter().map(|request| &request["id"]).collect();
        assert_eq!(ids, vec![3, 4]);
        assert_eq!(value["errors"].as_array().expect("array").len(), 2);
    }

    #[test]
    fn without_a_log_the_requests_are_an_empty_array() {
        let stats = window_stats(&SampleLog::new(), 2, 3, None, 3.0);
        let value = parse(&run_json(&record(), None, &stats, None));
        assert_eq!(value["requests"], serde_json::json!([]));
        assert_eq!(value["summary"]["total"], 0);
    }

    #[test]
    fn a_target_without_a_label_falls_back_to_a_numbered_name() {
        let log = log();
        let stats = window_stats(&log, 2, 3, None, 3.0);
        let mut record = record();
        record.target_labels.clear();
        let value = parse(&run_json(&record, Some(&log), &stats, None));
        assert_eq!(value["per_request"][0]["name"], "Target 1");
        assert_eq!(value["per_request"][0]["method"], "");
        assert_eq!(value["requests"][0]["method"], "");
    }
}
