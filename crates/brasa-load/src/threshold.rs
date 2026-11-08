//! [`Threshold`], a pass or fail goal for a load test run, and [`evaluate`], checking a run's
//! numbers against a list of them.
//!
//! Every metric has a fixed direction, so a threshold is just a metric and a limit: latencies and
//! the error rate must stay below their limit, the throughput must reach its limit. A run that
//! misses any threshold is "not passed"; thresholds never stop a run (that is
//! [`crate::LoadConfig::stop_on_error_rate`]).

use serde::{Deserialize, Serialize};

/// The metric a [`Threshold`] checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdMetric {
    /// Requests per second, a lower bound: passes when the value is at least the limit.
    Rps,
    /// The median latency in milliseconds, an upper bound: passes when below the limit.
    P50,
    /// The 95th percentile latency in milliseconds, an upper bound.
    P95,
    /// The 99th percentile latency in milliseconds, an upper bound.
    P99,
    /// The error rate as a fraction (`0.005` for 0.5 %), an upper bound.
    ErrorRate,
}

impl ThresholdMetric {
    /// Every metric, in the order the config panel's "Add threshold" menu lists them.
    pub const ALL: [ThresholdMetric; 5] = [
        ThresholdMetric::P50,
        ThresholdMetric::P95,
        ThresholdMetric::P99,
        ThresholdMetric::ErrorRate,
        ThresholdMetric::Rps,
    ];

    /// Whether a higher value is better: `true` only for [`ThresholdMetric::Rps`].
    #[must_use]
    pub fn higher_is_better(self) -> bool {
        self == ThresholdMetric::Rps
    }

    /// Whether `value` meets `limit` for this metric: `value >= limit` when a higher value is
    /// better, `value < limit` otherwise.
    #[must_use]
    pub fn passes(self, value: f64, limit: f64) -> bool {
        if self.higher_is_better() {
            value >= limit
        } else {
            value < limit
        }
    }

    /// The comparison operator shown next to the metric: `"≥"` or `"<"`.
    #[must_use]
    pub fn operator(self) -> &'static str {
        if self.higher_is_better() {
            "\u{2265}"
        } else {
            "<"
        }
    }
}

/// A goal for a run: `metric` compared against `limit`, see [`ThresholdMetric`] for the
/// direction and the unit of each metric.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Threshold {
    /// What is checked.
    pub metric: ThresholdMetric,
    /// The limit, in the metric's unit: requests per second, milliseconds, or a fraction for the
    /// error rate.
    pub limit: f64,
}

/// The numbers a [`Threshold`] is checked against, in the units [`ThresholdMetric`] documents.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ThresholdInputs {
    /// Requests per second.
    pub rps: f64,
    /// Median latency, in milliseconds.
    pub p50_ms: f64,
    /// 95th percentile latency, in milliseconds.
    pub p95_ms: f64,
    /// 99th percentile latency, in milliseconds.
    pub p99_ms: f64,
    /// Error rate, `0.0` to `1.0`.
    pub error_rate: f64,
}

impl ThresholdInputs {
    /// The value of `metric`.
    #[must_use]
    pub fn value(&self, metric: ThresholdMetric) -> f64 {
        match metric {
            ThresholdMetric::Rps => self.rps,
            ThresholdMetric::P50 => self.p50_ms,
            ThresholdMetric::P95 => self.p95_ms,
            ThresholdMetric::P99 => self.p99_ms,
            ThresholdMetric::ErrorRate => self.error_rate,
        }
    }
}

/// One checked [`Threshold`]: the value the run reached and whether it passed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThresholdOutcome {
    /// The threshold checked.
    pub threshold: Threshold,
    /// The run's value for [`Threshold::metric`].
    pub value: f64,
    /// Whether the value meets the limit.
    pub passed: bool,
}

/// Checks every threshold against `inputs`, in the given order.
#[must_use]
pub fn evaluate(thresholds: &[Threshold], inputs: &ThresholdInputs) -> Vec<ThresholdOutcome> {
    thresholds
        .iter()
        .map(|threshold| {
            let value = inputs.value(threshold.metric);
            ThresholdOutcome {
                threshold: *threshold,
                value,
                passed: threshold.metric.passes(value, threshold.limit),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn inputs() -> ThresholdInputs {
        ThresholdInputs {
            rps: 1_850.0,
            p50_ms: 9.0,
            p95_ms: 31.0,
            p99_ms: 62.0,
            error_rate: 0.003,
        }
    }

    #[test]
    fn rps_is_a_lower_bound_and_the_limit_itself_passes() {
        assert!(ThresholdMetric::Rps.passes(1_800.0, 1_800.0));
        assert!(!ThresholdMetric::Rps.passes(1_799.9, 1_800.0));
    }

    #[test]
    fn latencies_and_errors_are_strict_upper_bounds() {
        assert!(ThresholdMetric::P99.passes(49.9, 50.0));
        assert!(!ThresholdMetric::P99.passes(50.0, 50.0));
        assert!(ThresholdMetric::ErrorRate.passes(0.004, 0.005));
    }

    #[test]
    fn evaluate_keeps_the_order_and_reports_the_values() {
        let thresholds = [
            Threshold {
                metric: ThresholdMetric::P99,
                limit: 50.0,
            },
            Threshold {
                metric: ThresholdMetric::ErrorRate,
                limit: 0.005,
            },
            Threshold {
                metric: ThresholdMetric::Rps,
                limit: 1_800.0,
            },
        ];
        let outcomes = evaluate(&thresholds, &inputs());
        let passed: Vec<bool> = outcomes.iter().map(|outcome| outcome.passed).collect();
        assert_eq!(passed, vec![false, true, true]);
        assert!((outcomes[0].value - 62.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_threshold_round_trips_through_json_with_snake_case_metrics() {
        let threshold = Threshold {
            metric: ThresholdMetric::ErrorRate,
            limit: 0.005,
        };
        let json = serde_json::to_string(&threshold).unwrap_or_default();
        assert_eq!(json, r#"{"metric":"error_rate","limit":0.005}"#);
        let back: Threshold = serde_json::from_str(&json).unwrap_or(Threshold {
            metric: ThresholdMetric::Rps,
            limit: 0.0,
        });
        assert_eq!(back, threshold);
    }
}
