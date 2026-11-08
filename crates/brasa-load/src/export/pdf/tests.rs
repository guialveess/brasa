use std::path::PathBuf;

use super::*;
use crate::export::report::{
    ReportCurveLine, ReportCurves, ReportKpi, ReportLabels, ReportLatencyRow, ReportReferenceLine,
    ReportSeries, ReportStatusColumn, ReportTable, ReportThreshold, ReportVerdict,
};

const REGULAR: &[u8] = include_bytes!("../../../tests/fixtures/fonts/Geist-Regular.ttf");
const MEDIUM: &[u8] = include_bytes!("../../../tests/fixtures/fonts/Geist-Medium.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../../../tests/fixtures/fonts/Geist-SemiBold.ttf");
const MONO: &[u8] = include_bytes!("../../../tests/fixtures/fonts/GeistMono-Regular.ttf");

fn fonts() -> PdfFonts<'static> {
    PdfFonts {
        regular: REGULAR,
        medium: MEDIUM,
        semibold: SEMIBOLD,
        mono: MONO,
    }
}

/// The number of pages the layout needs for `report`.
fn page_count(report: &Report) -> usize {
    let fonts = Fonts::new(&fonts()).expect("test fonts are valid");
    layout(report, &fonts).len()
}

const GREEN: Rgb = Rgb(0x1f, 0xa8, 0x6a);
const RED: Rgb = Rgb(0xe0, 0x4f, 0x4f);
const AMBER: Rgb = Rgb(0xd9, 0x8e, 0x1f);
const ORANGE: Rgb = Rgb(0xe0, 0x6a, 0x45);
const TEAL: Rgb = Rgb(0x2b, 0xa8, 0x98);
const INDIGO: Rgb = Rgb(0x5b, 0x63, 0xe6);
const DARK_RED: Rgb = Rgb(0xa0, 0x2a, 0x2a);
const GRAY: Rgb = Rgb(0x8a, 0x8e, 0x9a);

fn kpi(label: &str, value: &str, unit: &str) -> ReportKpi {
    ReportKpi {
        label: label.into(),
        value: value.into(),
        unit: unit.into(),
        delta: None,
        delta_tone: ReportTone::Neutral,
        goal: None,
    }
}

fn with_delta(mut k: ReportKpi, delta: &str, tone: ReportTone) -> ReportKpi {
    k.delta = Some(delta.into());
    k.delta_tone = tone;
    k
}

fn with_goal(mut k: ReportKpi, goal: &str, met: bool) -> ReportKpi {
    k.goal = Some((goal.into(), met));
    k
}

fn series(
    label: &str,
    color: Rgb,
    axis: ReportAxis,
    style: ReportSeriesStyle,
    values: Vec<f64>,
) -> ReportSeries {
    ReportSeries {
        label: label.into(),
        color,
        axis,
        style,
        values,
    }
}

fn status(label: &str, share: f64, text: &str, color: Rgb) -> ReportStatus {
    ReportStatus {
        label: label.into(),
        share,
        share_text: text.into(),
        count_text: format!("{}", (share * 50_837.0).round()),
        color,
    }
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|c| (*c).to_string()).collect()
}

fn group(code: &str, title: &str, count: &str, share: &str, sample: &str) -> ReportErrorGroup {
    ReportErrorGroup {
        code: code.into(),
        title: title.into(),
        count: count.into(),
        share: share.into(),
        span: "First \u{b7} last: 00:36 \u{b7} 00:43".into(),
        requests: "Affected requests: POST create 301, GET list 111".into(),
        sample: sample.into(),
    }
}

/// A report like the design's: 60 seconds, 50 VUs, rate limiting around seconds 36 to 43.
fn full_report() -> Report {
    // A small deterministic generator, as in the design.
    let mut seed: u32 = 11;
    let mut rand = move || {
        seed = (seed * 9301 + 49297) % 233_280;
        f64::from(seed) / 233_280.0
    };
    let (mut vus, mut rps, mut p50, mut p90, mut p99, mut base) =
        (vec![], vec![], vec![], vec![], vec![], vec![]);
    for i in 0..60_u32 {
        let v = f64::from((5 * (i + 1)).min(50));
        let ramp = v / 50.0;
        let spike = if (36..=43).contains(&i) {
            ((f64::from(i) - 35.5) / 8.0 * std::f64::consts::PI).sin()
        } else {
            0.0
        };
        let r = ramp * (2150.0 + rand() * 120.0 - 60.0) * (1.0 - 0.3 * spike);
        let a = 6.0 + 4.0 * ramp + rand() * 1.5 + 5.0 * spike;
        let b = a * 2.1 + rand() * 2.0 + 18.0 * spike;
        let c = b * 1.5 + rand() * 4.0 + 40.0 * spike;
        vus.push(v / 50.0 * 2400.0 * 0.4);
        rps.push(r);
        p50.push(a);
        p90.push(b);
        p99.push(c.min(100.0));
        base.push(a * 2.1 * 1.45 + rand() * 3.0);
    }
    let chart = ReportChart {
        title: "Performance over time".into(),
        series: vec![
            series("VUs", GRAY, ReportAxis::Left, ReportSeriesStyle::Area, vus),
            series(
                "Run #2 p99",
                ORANGE,
                ReportAxis::Right,
                ReportSeriesStyle::Dashed(1.2),
                base,
            ),
            series(
                "p99",
                ORANGE,
                ReportAxis::Right,
                ReportSeriesStyle::Line(1.4),
                p99,
            ),
            series(
                "p90",
                AMBER,
                ReportAxis::Right,
                ReportSeriesStyle::Line(1.2),
                p90,
            ),
            series(
                "p50",
                TEAL,
                ReportAxis::Right,
                ReportSeriesStyle::Line(1.2),
                p50,
            ),
            series(
                "Req/s",
                INDIGO,
                ReportAxis::Left,
                ReportSeriesStyle::Line(1.8),
                rps,
            ),
        ],
        left_max: 2400.0,
        right_max: 100.0,
        left_ticks: ["0", "600", "1,200", "1,800", "2,400"]
            .map(String::from)
            .to_vec(),
        right_ticks: ["0 ms", "25 ms", "50 ms", "75 ms", "100 ms"]
            .map(String::from)
            .to_vec(),
        x_ticks: [
            "00:00", "00:10", "00:20", "00:30", "00:40", "00:50", "01:00",
        ]
        .map(String::from)
        .to_vec(),
        reference_lines: vec![ReportReferenceLine {
            axis: ReportAxis::Right,
            value: 50.0,
            label: "p99 goal 50 ms".into(),
            color: RED,
        }],
        window: Some((34, 45)),
    };
    Report {
        title: "Run #3 \u{b7} users".into(),
        subtitle: "Finished \u{b7} Keep-alive \u{b7} 23/10/2026 12:04 \u{b7} 50 VUs \u{b7} 60 s"
            .into(),
        window: Some("Window 34\u{2013}46 s".into()),
        comparison: Some("Compared with Run #2".into()),
        verdict: Some(ReportVerdict {
            passed: false,
            text: "Not passed \u{b7} 1 of 3".into(),
        }),
        thresholds: vec![
            ReportThreshold {
                label: "p99 < 50 ms".into(),
                value: "62 ms".into(),
                passed: false,
            },
            ReportThreshold {
                label: "Errors < 1 %".into(),
                value: "0.81 %".into(),
                passed: true,
            },
            ReportThreshold {
                label: "Req/s \u{2265} 1,800".into(),
                value: "2,030".into(),
                passed: true,
            },
        ],
        kpis: vec![
            with_goal(
                with_delta(kpi("Req/s", "2,030", ""), "+1.0 %", ReportTone::Good),
                "goal \u{2265} 1,800",
                true,
            ),
            with_delta(kpi("p50", "11", "ms"), "+10.0 %", ReportTone::Bad),
            with_delta(kpi("p95", "28", "ms"), "+7.7 %", ReportTone::Bad),
            with_goal(
                with_delta(kpi("p99", "62", "ms"), "+63.2 %", ReportTone::Bad),
                "goal < 50 ms",
                false,
            ),
            with_delta(kpi("Errors", "0.81", "%"), "+0.61 pt", ReportTone::Bad),
            with_delta(kpi("Total", "50,837", ""), "+5.4 %", ReportTone::Good),
        ],
        chart: Some(chart),
        per_request: ReportTable {
            title: "Per request".into(),
            headers: ["Request", "Total", "p50", "p95", "p99", "Errors"]
                .map(String::from)
                .to_vec(),
            align: vec![
                ReportAlign::Left,
                ReportAlign::Right,
                ReportAlign::Right,
                ReportAlign::Right,
                ReportAlign::Right,
                ReportAlign::Right,
            ],
            rows: vec![
                row(&["POST create", "16,945", "12 ms", "31 ms", "58 ms", "1.20 %"]),
                row(&["GET list", "17,130", "10 ms", "26 ms", "49 ms", "0.31 %"]),
                row(&[
                    "DELETE delete",
                    "16,762",
                    "11 ms",
                    "27 ms",
                    "51 ms",
                    "0.92 %",
                ]),
            ],
            total: Some(row(&[
                "All requests",
                "50,837",
                "11 ms",
                "28 ms",
                "62 ms",
                "0.81 %",
            ])),
            statuses: Some(ReportStatusColumn {
                header: "Statuses".into(),
                rows: vec![
                    vec![
                        status("201", 0.99, "99.0 %", GREEN),
                        status("429", 0.01, "1.0 %", AMBER),
                    ],
                    vec![status("200", 1.0, "100 %", GREEN)],
                    vec![
                        status("200", 0.98, "98.0 %", GREEN),
                        status("502", 0.01, "1.0 %", RED),
                        status("Timeout", 0.01, "1.0 %", DARK_RED),
                    ],
                ],
                total: vec![
                    status("200", 0.659, "65.9 %", GREEN),
                    status("201", 0.325, "32.5 %", GREEN),
                    status("429", 0.0081, "0.81 %", AMBER),
                    status("502", 0.0045, "0.45 %", RED),
                    status("Timeout", 0.0034, "0.34 %", DARK_RED),
                ],
            }),
        },
        latency: Some(latency()),
        errors: vec![
            group(
                "429",
                "429 Too Many Requests",
                "412",
                "0.81 %",
                "{\"error\":\"rate limit exceeded\",\"retry_after\":2}",
            ),
            group(
                "Timeout",
                "Request timed out",
                "173",
                "0.34 %",
                "operation timed out after 5000 ms",
            ),
            group(
                "502",
                "502 Bad Gateway",
                "229",
                "0.45 %",
                "<html><body><h1>502 Bad Gateway</h1></body></html>",
            ),
        ],
        labels: ReportLabels {
            summary: "Summary".into(),
            thresholds: "Thresholds".into(),
            errors: "Errors".into(),
            generated_by: "Generated by Brasa 0.4.0 on 2026-10-23 12:04".into(),
            no_errors: "No errors".into(),
        },
    }
}

/// The latency section for the three requests of `full_report`.
fn latency() -> ReportLatency {
    let names = ["POST create", "GET list", "DELETE delete"];
    let colors = [INDIGO, TEAL, Rgb(0xb0, 0x47, 0x9a)];
    let histograms = (0..3)
        .map(|k| ReportRequestHistogram {
            name: names[k].into(),
            color: colors[k],
            count_text: "16,945 responses".into(),
            counts: (0..24_u32)
                .map(|i| {
                    let x = (f64::from(i) - 6.0 - 3.0 * k as f64) / 3.0;
                    (900.0 * (-x * x).exp()) as u64
                })
                .collect(),
            markers: vec![(0.3, GREEN), (0.5, AMBER), (0.6, RED)],
            x_labels: ["10 ms", "20 ms", "30 ms"].map(String::from).to_vec(),
            caption: vec![
                ("p50 12 ms".into(), GREEN),
                ("p95 31 ms".into(), AMBER),
                ("p99 58 ms".into(), RED),
            ],
        })
        .collect();
    let mut rows = vec![ReportLatencyRow {
        name: "All requests".into(),
        color: None,
        cells: ["50,837", "11 ms", "20 ms", "28 ms", "62 ms", "310 ms"]
            .map(String::from)
            .to_vec(),
    }];
    rows.extend((0..3).map(|k| {
        ReportLatencyRow {
            name: names[k].into(),
            color: Some(colors[k]),
            cells: ["16,945", "12 ms", "21 ms", "31 ms", "58 ms", "300 ms"]
                .map(String::from)
                .to_vec(),
        }
    }));
    let lines = (0..3)
        .map(|k| ReportCurveLine {
            name: names[k].into(),
            color: colors[k],
            values_us: (0..61)
                .map(|i| 10_000.0 * (1.0 + k as f64 * 0.5) * (1.0 + f64::from(i) / 10.0))
                .collect(),
        })
        .collect();
    ReportLatency {
        title: "Latency".into(),
        caption: "Successful responses per request, each on its own latency axis.".into(),
        note: "Successful responses only, 12 errors left out.".into(),
        histograms,
        headers: ["Request", "Count", "p50", "p90", "p95", "p99", "max"]
            .map(String::from)
            .to_vec(),
        rows,
        curves: Some(ReportCurves {
            title: "Latency by percentile".into(),
            subtitle: "log axes".into(),
            lines,
            y_min_us: 5_000.0,
            y_max_us: 200_000.0,
            y_ticks: vec![
                (5_000.0, "5 ms".into()),
                (10_000.0, "10 ms".into()),
                (20_000.0, "20 ms".into()),
                (50_000.0, "50 ms".into()),
                (100_000.0, "100 ms".into()),
            ],
            x_ticks: ["min", "p90", "p99", "p99.9"].map(String::from).to_vec(),
        }),
    }
}

fn long_report() -> Report {
    let mut report = full_report();
    report.per_request.rows = (0..200)
        .map(|i| {
            row(&[
                &format!("GET /users/{i}/orders/with/a/rather/long/path/segment/{i}"),
                "1,234",
                "12 ms",
                "31 ms",
                "58 ms",
                "0.20 %",
            ])
        })
        .collect();
    report.errors = (0..10)
        .map(|i| {
            group(
                &format!("5{i:02}"),
                &format!("5{i:02} Some Server Error With A Fairly Long Descriptive Title"),
                "12",
                "0.02 %",
                "{\"error\":\"internal\"}",
            )
        })
        .collect();
    report
}

fn assert_pdf(bytes: &[u8]) {
    assert!(bytes.starts_with(b"%PDF"), "not a PDF");
    assert!(bytes.len() > 1000);
}

#[test]
fn full_report_is_a_pdf_on_few_pages() {
    let report = full_report();
    assert_pdf(&to_pdf(&report, &fonts()).expect("renders"));
    assert!(page_count(&report) <= 2);
}

#[test]
fn long_report_breaks_onto_more_pages() {
    let report = long_report();
    let bytes = to_pdf(&report, &fonts()).expect("renders");
    assert_pdf(&bytes);
    assert!(page_count(&report) > 1);
}

#[test]
fn the_latency_section_and_status_column_are_drawn() {
    let report = full_report();
    let fonts = Fonts::new(&fonts()).expect("test fonts are valid");
    let pages = layout(&report, &fonts);
    let texts: Vec<&str> = pages
        .iter()
        .flatten()
        .filter_map(|op| match op {
            Op::Text { text, .. } => Some(text.as_str()),
            Op::Shape { .. } => None,
        })
        .collect();
    for expected in [
        "Statuses",
        "All requests",
        "Latency",
        "Successful responses per request, each on its own latency axis.",
        "16,945 responses",
        "p95 31 ms",
        "Latency by percentile",
        "p99.9",
        "DELETE delete",
    ] {
        assert!(
            texts.iter().any(|t| t.contains(expected)),
            "missing {expected}"
        );
    }
    // Without the latency section and the status bars, those texts are gone.
    let mut plain = report.clone();
    plain.latency = None;
    plain.per_request.statuses = None;
    let pages = layout(&plain, &fonts);
    assert!(
        !pages.iter().flatten().any(
            |op| matches!(op, Op::Text { text, .. } if text.contains("Latency by percentile"))
        )
    );
}

#[test]
fn close_curve_labels_are_nudged_apart() {
    let mut report = full_report();
    if let Some(curves) = report.latency.as_mut().and_then(|l| l.curves.as_mut()) {
        for line in &mut curves.lines {
            line.values_us = vec![20_000.0; 61];
        }
    }
    let fonts = Fonts::new(&fonts()).expect("test fonts are valid");
    let pages = layout(&report, &fonts);
    let mut ys: Vec<f32> = pages
        .iter()
        .flatten()
        .filter_map(|op| match op {
            Op::Text { y, text, style, .. }
                if style.size == 7.5
                    && ["POST create", "GET list", "DELETE delete"].contains(&text.as_str()) =>
            {
                Some(*y)
            }
            _ => None,
        })
        .collect();
    ys.sort_by(f32::total_cmp);
    assert!(ys.len() >= 3);
    let tail = &ys[ys.len() - 3..];
    assert!(tail[1] - tail[0] >= 8.9 && tail[2] - tail[1] >= 8.9);
}

#[test]
fn gaps_and_empty_series_do_not_fail() {
    let mut report = full_report();
    if let Some(chart) = &mut report.chart {
        chart.series[0].values = vec![f64::NAN; 60];
        chart.series[2].values[10] = f64::NAN;
        chart.series[2].values[11] = f64::INFINITY;
        chart.series[3].values.clear();
        chart.series[4].values = vec![5.0];
        chart.left_max = 0.0;
        chart.window = Some((50, 100));
    }
    assert_pdf(&to_pdf(&report, &fonts()).expect("renders"));
}

#[test]
fn empty_report_does_not_fail() {
    let mut report = full_report();
    report.verdict = None;
    report.thresholds.clear();
    report.kpis.clear();
    report.chart = None;
    report.latency = None;
    report.per_request.headers.clear();
    report.per_request.rows.clear();
    report.per_request.total = None;
    report.per_request.statuses = None;
    report.errors.clear();
    assert_pdf(&to_pdf(&report, &fonts()).expect("renders"));
    assert_eq!(page_count(&report), 1);
}

#[test]
fn degenerate_inputs_do_not_fail() {
    let mut report = full_report();
    if let Some(chart) = &mut report.chart {
        chart.left_ticks.clear();
        chart.right_ticks.clear();
        chart.x_ticks.clear();
        chart.series.clear();
    }
    if let Some(latency) = &mut report.latency {
        latency.histograms[0].counts = vec![0; 5];
        latency.histograms[1].x_labels = vec!["only".into()];
        latency.histograms[2].markers.clear();
        latency.rows.truncate(1);
        if let Some(curves) = &mut latency.curves {
            curves.lines[0].values_us = vec![5.0];
            curves.lines[1].values_us.clear();
            curves.y_ticks.clear();
            curves.x_ticks.clear();
        }
    }
    report.per_request.rows.push(vec!["short row".into()]);
    report.title = "x".repeat(400);
    assert_pdf(&to_pdf(&report, &fonts()).expect("renders"));
}

#[test]
fn bad_font_is_an_error() {
    let report = full_report();
    let bad = PdfFonts {
        regular: b"not a font",
        ..fonts()
    };
    assert!(matches!(
        to_pdf(&report, &bad),
        Err(PdfError::Font("regular"))
    ));
}

#[test]
fn text_is_cut_with_an_ellipsis() {
    let fonts = Fonts::new(&fonts()).expect("test fonts are valid");
    let face = &fonts.regular;
    let cut = face.fit("A fairly long sentence that does not fit", 9.0, 60.0);
    assert!(cut.ends_with('\u{2026}'));
    assert!(face.width(&cut, 9.0) <= 60.0);
    assert_eq!(face.fit("short", 9.0, 60.0), "short");
    assert_eq!(face.fit("anything", 9.0, 1.0), "");
}

/// Every text the layout draws for `report`, on any page.
fn drawn_texts(report: &Report) -> Vec<String> {
    let fonts = Fonts::new(&fonts()).expect("test fonts are valid");
    layout(report, &fonts)
        .into_iter()
        .flatten()
        .filter_map(|op| match op {
            Op::Text { text, .. } => Some(text),
            Op::Shape { .. } => None,
        })
        .collect()
}

#[test]
fn kpi_labels_and_totals_are_not_cut_by_long_deltas() {
    let mut report = full_report();
    report.kpis = vec![
        with_delta(kpi("Requests/s", "209", ""), "+151.6 %", ReportTone::Good),
        with_delta(kpi("p50", "96", "ms"), "-1.3 %", ReportTone::Good),
        with_goal(
            with_delta(kpi("p95", "125", "ms"), "-36.7 %", ReportTone::Good),
            "goal < 250 ms",
            true,
        ),
        with_delta(kpi("p99", "218", "ms"), "-7.8 %", ReportTone::Good),
        with_delta(kpi("Errors", "0.0", "%"), "0.0 pt", ReportTone::Neutral),
        with_delta(kpi("Total", "2,111", ""), "+208.6 %", ReportTone::Good),
    ];
    if let Some(total) = report.per_request.total.as_mut() {
        total[4] = "218 ms".into();
    }
    let texts = drawn_texts(&report);
    for whole in ["Requests/s", "Total", "+151.6 %", "+208.6 %", "218 ms"] {
        assert!(
            texts.iter().any(|text| text == whole),
            "{whole} is drawn whole"
        );
    }
    assert!(
        !texts.iter().any(|text| text.contains('\u{2026}')),
        "nothing is cut"
    );
}

/// Writes the full report to `$CARGO_TARGET_DIR/report-sample.pdf` (and the long one next to it)
/// to look at the layout: `cargo test -p brasa-load report_sample -- --ignored`.
#[test]
#[ignore = "writes a file to look at the layout"]
fn report_sample() {
    let dir =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from);
    for (name, report) in [
        ("report-sample.pdf", full_report()),
        ("report-sample-long.pdf", long_report()),
    ] {
        let bytes = to_pdf(&report, &fonts()).expect("renders");
        std::fs::write(dir.join(name), bytes).expect("writes the sample");
    }
}
