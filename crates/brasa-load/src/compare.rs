//! [`compare`], turning two [`crate::LoadSnapshot`]s into the "Compare with" rows of the load test
//! dashboard.

use crate::metrics::LoadSnapshot;

/// The unit a [`Delta::change`] is expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaUnit {
    /// A percent change relative to the previous value: `(current - previous) / previous * 100`.
    Percent,
    /// A difference in percentage points, used for a metric that is already a fraction (an error
    /// rate): `(current - previous) * 100`, not relative to `previous`.
    PercentagePoints,
}

/// One row of the "Compare with" table: a metric, its previous and current value, and the
/// signed change between them.
#[derive(Debug, Clone, PartialEq)]
pub struct Delta {
    /// The metric name, as shown in the UI ("Requests/s", "p95", "p99", "Errors").
    pub label: &'static str,
    /// The value of this metric in the previous run.
    pub previous: f64,
    /// The value of this metric in the current run.
    pub current: f64,
    /// The signed change from `previous` to `current`, in `unit`.
    pub change: f64,
    /// The unit `change` is expressed in.
    pub unit: DeltaUnit,
    /// Whether this change is an improvement: for "Requests/s" a higher value is better; for
    /// "p95", "p99" and "Errors" a lower value is better.
    pub is_improvement: bool,
}

/// Compares `current` against `previous`, one [`Delta`] each for Requests/s, p95, p99 and Errors,
/// in that order.
///
/// "Requests/s" compares [`LoadSnapshot::rps`] (the last fully elapsed second of each run, the
/// steadiest single reading available); "p95" and "p99" compare the run-wide percentiles, not a
/// single second's. Both are rounded to whole milliseconds first, the precision the dashboard
/// shows them in, so a row never reads "3 ms, 3 ms, +5.1%".
#[must_use]
pub fn compare(previous: &LoadSnapshot, current: &LoadSnapshot) -> Vec<Delta> {
    vec![
        higher_is_better("Requests/s", previous.rps, current.rps),
        lower_is_better("p95", whole_ms(previous.p95), whole_ms(current.p95)),
        lower_is_better("p99", whole_ms(previous.p99), whole_ms(current.p99)),
        error_rate_delta(previous.error_rate, current.error_rate),
    ]
}

/// A [`Delta`] for a metric where a higher value is better (Requests/s).
fn higher_is_better(label: &'static str, previous: f64, current: f64) -> Delta {
    Delta {
        label,
        previous,
        current,
        change: percent_change(previous, current),
        unit: DeltaUnit::Percent,
        is_improvement: current > previous,
    }
}

/// A [`Delta`] for a metric where a lower value is better (p95, p99).
fn lower_is_better(label: &'static str, previous: f64, current: f64) -> Delta {
    Delta {
        label,
        previous,
        current,
        change: percent_change(previous, current),
        unit: DeltaUnit::Percent,
        is_improvement: current < previous,
    }
}

/// The error rate delta, in percentage points rather than a percent change, since the metric
/// itself is already a fraction (errors delta in
/// percentage points).
fn error_rate_delta(previous: f64, current: f64) -> Delta {
    Delta {
        label: "Errors",
        previous,
        current,
        change: (current - previous) * 100.0,
        unit: DeltaUnit::PercentagePoints,
        is_improvement: current < previous,
    }
}

/// `micros` rounded to the nearest whole millisecond, still in microseconds.
fn whole_ms(micros: u32) -> f64 {
    (f64::from(micros) / 1000.0).round() * 1000.0
}

/// `(current - previous) / previous * 100`, or `0.0` when `previous` is (near) zero, instead of
/// dividing by zero.
fn percent_change(previous: f64, current: f64) -> f64 {
    if previous.abs() < f64::EPSILON {
        0.0
    } else {
        (current - previous) / previous * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::StatusKey;

    /// A bare [`LoadSnapshot`] with only the fields [`compare`] reads set, for readable tests.
    fn snapshot(rps: f64, p95: u32, p99: u32, error_rate: f64) -> LoadSnapshot {
        LoadSnapshot {
            elapsed: std::time::Duration::ZERO,
            active_vus: 0,
            total: 0,
            rps,
            p50: 0,
            p95,
            p99,
            error_rate,
            series: Vec::new(),
            histogram: [0; 26],
            status_counts: Vec::<(StatusKey, u64)>::new(),
            per_target: Vec::new(),
            error_examples: Vec::new(),
        }
    }

    #[test]
    fn matches_the_design_example() {
        let previous = snapshot(388.0, 104_000, 262_000, 0.009);
        let current = snapshot(412.0, 112_000, 240_000, 0.008);

        let deltas = compare(&previous, &current);
        assert_eq!(deltas.len(), 4);

        assert_eq!(deltas[0].label, "Requests/s");
        assert!((deltas[0].change - 6.185_567_010_309_278).abs() < 1e-9);
        assert!(deltas[0].is_improvement);

        assert_eq!(deltas[1].label, "p95");
        assert!((deltas[1].change - 7.692_307_692_307_693).abs() < 1e-9);
        assert!(!deltas[1].is_improvement);

        assert_eq!(deltas[2].label, "p99");
        assert!((deltas[2].change - -8.396_946_564_885_496).abs() < 1e-9);
        assert!(deltas[2].is_improvement);

        assert_eq!(deltas[3].label, "Errors");
        assert_eq!(deltas[3].unit, DeltaUnit::PercentagePoints);
        assert!((deltas[3].change - -0.1).abs() < 1e-9);
        assert!(deltas[3].is_improvement);
    }

    #[test]
    fn percent_change_of_a_zero_previous_value_is_zero_not_infinite() {
        let previous = snapshot(0.0, 0, 0, 0.0);
        let current = snapshot(50.0, 10_000, 10_000, 0.0);

        let deltas = compare(&previous, &current);
        assert!((deltas[0].change - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn latencies_that_display_the_same_whole_ms_have_no_change() {
        let previous = snapshot(0.0, 3_000, 3_020, 0.0);
        let current = snapshot(0.0, 3_150, 3_400, 0.0);

        let deltas = compare(&previous, &current);
        assert!((deltas[1].change - 0.0).abs() < f64::EPSILON);
        assert!(!deltas[1].is_improvement);
        assert!((deltas[2].change - 0.0).abs() < f64::EPSILON);
        assert!((deltas[1].current - 3_000.0).abs() < f64::EPSILON);
    }
}
