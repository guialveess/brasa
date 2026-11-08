//! The load test results' Export menu actions: saving the run as Markdown, PDF, CSV or JSON, and
//! copying the summary as Markdown (`Ctrl+Shift+C`).
//!
//! Everything that costs time runs on the background executor: the statistics of the covered
//! range, the report's latency tables (they read the whole log), rendering the PDF, building the
//! CSV and the JSON and writing the file. The UI thread only gathers what the tab holds (cheap
//! clones and `Arc`s), formats the report's texts and shows the result.
//!
//! Saving a file is a job of steps that reports to a [`Progress`] ([`run_export`]). When it takes
//! longer than [`EXPORT_DIALOG_DELAY`], a modal shows the step, a progress bar and a Cancel
//! button; a quick export never flashes it. The file is written next to its final name and
//! renamed once complete, so a cancelled or failed export leaves an existing file untouched.

use std::fs::File;
use std::io::Write as _;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use brasa_load::export::csv::requests_csv_with;
use brasa_load::export::json::run_json_with;
use brasa_load::export::markdown::to_markdown;
use brasa_load::export::pdf::{PdfFonts, to_pdf};
use brasa_load::export::report::Report;
use brasa_load::history::{self, LoadConfigSummary, RunRecord};
use brasa_load::{
    LoadSnapshot, Progress, SampleLog, Threshold, WindowStats, window_stats, window_stats_with,
};
use rust_i18n::t;
use telaio_ui::fonts::{GEIST_MEDIUM, GEIST_MONO_REGULAR, GEIST_REGULAR, GEIST_SEMIBOLD};

use crate::actions::CopyLoadTestSummary;
use crate::state::format::shorten_path;
use crate::state::load_report::{
    ReportColors, ReportInput, build_report, connection_label, latency_units, stats_from_snapshot,
    status_label,
};
use crate::state::load_results::{ChartData, Shared, SummaryData};
use crate::state::load_test::{LoadTestStatus, LoadTestTab};
use crate::state::number::format_integer;
use crate::theme::PaletteExt;
use crate::theme::metrics::{MODAL_HEADER_HEIGHT, MODAL_WIDTH_DEFAULT};
use crate::views::components::{
    ModalPlacement, ProgressBar, SecondaryButton, framed_dialog, modal_body, modal_footer,
};
use crate::views::root::AppView;

/// How long an export runs before its progress modal opens.
const EXPORT_DIALOG_DELAY: Duration = Duration::from_millis(300);
/// How often the progress modal is redrawn while it is open.
const EXPORT_REPAINT: Duration = Duration::from_millis(100);
/// How many bytes are written between two checks of the progress.
const WRITE_CHUNK: usize = 1 << 20;

/// A file format of the Export menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportFormat {
    /// The Markdown report.
    Markdown,
    /// The PDF report.
    Pdf,
    /// One CSV row per request.
    Csv,
    /// The whole run as JSON.
    Json,
}

impl ExportFormat {
    /// The file extension, without the dot.
    fn extension(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "md",
            ExportFormat::Pdf => "pdf",
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
        }
    }

    /// The name shown to people, in the progress modal.
    fn title(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "Markdown",
            ExportFormat::Pdf => "PDF",
            ExportFormat::Csv => "CSV",
            ExportFormat::Json => "JSON",
        }
    }

    /// Whether the format needs the statistics of the covered range (the CSV only lists the
    /// requests).
    fn needs_stats(self) -> bool {
        self != ExportFormat::Csv
    }

    /// How many times building the file goes through the log: the CSV writes every request,
    /// the JSON gathers them and then writes them, the reports do not list them.
    fn request_passes(self) -> u64 {
        match self {
            ExportFormat::Csv => 1,
            ExportFormat::Json => 2,
            ExportFormat::Markdown | ExportFormat::Pdf => 0,
        }
    }

    /// The name used in the log.
    fn name(self) -> &'static str {
        match self {
            ExportFormat::Markdown => "markdown",
            ExportFormat::Pdf => "pdf",
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
        }
    }

    /// Whether the format is a document built from a [`Report`].
    fn is_report(self) -> bool {
        matches!(self, ExportFormat::Markdown | ExportFormat::Pdf)
    }
}

/// What an export needs from a load test tab, copied out of it so the work can move to the
/// background: the log and the chart are shared `Arc`s, the rest is small.
struct ExportSource {
    number: u32,
    target_label: String,
    status_label: String,
    connection_label: Option<String>,
    started_at: Option<u64>,
    vus: Option<u32>,
    duration_secs: u64,
    snapshot: LoadSnapshot,
    compare: Option<(u32, LoadSnapshot)>,
    thresholds: Vec<Threshold>,
    chart: Option<Shared<ChartData>>,
    /// The selected window, as inclusive second indices.
    window: Option<(usize, usize)>,
    /// Whether only the window is exported. `false` when there is no window or no log to cut.
    window_only: bool,
    /// Each target's method and name.
    targets: Vec<(String, String)>,
    timeout_secs: u64,
    log: Option<Shared<SampleLog>>,
    /// The tab's statistics when they already cover what is exported.
    analysis: Option<Shared<WindowStats>>,
    target_count: usize,
    run_secs: u32,
    elapsed_secs: f64,
    /// The workspace folder, where the run's record is saved.
    root: Option<PathBuf>,
    /// The record to export as JSON when the saved one cannot be read.
    record: RunRecord,
}

impl ExportSource {
    /// Gathers what `load_test` shows, `None` while a run is in progress or before there is
    /// anything to export.
    fn from_tab(load_test: &LoadTestTab, timeout_secs: u64, root: Option<&Path>) -> Option<Self> {
        if load_test.is_running() {
            return None;
        }
        let snapshot = load_test.snapshot.clone()?;
        let number = load_test.run_number?;
        let log = load_test.log.clone();
        // Without the requests there is nothing to cut a window from.
        let window_only =
            load_test.export_window_only && load_test.window.is_some() && log.is_some();
        let targets: Vec<(String, String)> = load_test
            .target_rows
            .iter()
            .map(|(method, name)| (method.to_string(), name.clone()))
            .collect();
        let compare = load_test
            .compare_with
            .zip(load_test.compare_snapshot.clone());
        let chart = load_test.chart.clone().or_else(|| {
            Some(Shared::new(ChartData::new(
                &snapshot,
                compare.as_ref().map(|(_, snapshot)| snapshot),
            )))
        });
        let target_count = log.as_ref().map_or(targets.len(), |log| {
            targets.len().max(
                log.samples()
                    .iter()
                    .map(|sample| sample.target as usize + 1)
                    .max()
                    .unwrap_or(0),
            )
        });
        let elapsed_secs = snapshot.elapsed.as_secs_f64();
        let run_secs = (load_test.shown_duration_secs() as f64)
            .max(elapsed_secs.floor())
            .max(1.0) as u32;
        // The tab's statistics are of its selected window: they are what is exported when that
        // window is, or when there is none.
        let analysis = load_test
            .analysis
            .as_ref()
            .filter(|(key, _)| {
                key.generation == load_test.log_generation
                    && key.window == load_test.window
                    && (window_only || load_test.window.is_none())
            })
            .map(|(_, stats)| stats.clone());
        let config = load_test.config.resolved();
        let started_at = load_test.run_started_at;
        let record = RunRecord {
            number,
            started_at_unix: started_at.unwrap_or(0),
            target_label: load_test.target_label.clone(),
            config: LoadConfigSummary {
                vus: load_test.run_vus.unwrap_or(config.vus),
                duration_secs: load_test.shown_duration_secs(),
                ramp_up_secs: config.ramp_up_secs,
                think_time_ms: config.think_time_ms,
                stop_on_error_rate: config.stop_on_error_rate,
                target_count: target_count.max(snapshot.per_target.len()),
                reuse_connections: load_test.run_reuse_connections.unwrap_or(false),
            },
            snapshot: snapshot.clone(),
            stopped_early: load_test.status == LoadTestStatus::Stopped,
            target_labels: targets.clone(),
            thresholds: load_test.run_thresholds.clone(),
        };
        Some(Self {
            number,
            target_label: load_test.target_label.clone(),
            status_label: status_label(&load_test.status),
            connection_label: load_test.run_reuse_connections.map(connection_label),
            started_at,
            vus: load_test.run_vus,
            duration_secs: load_test.shown_duration_secs(),
            snapshot,
            compare,
            thresholds: load_test.run_thresholds.clone(),
            chart,
            window: load_test.window,
            window_only,
            targets,
            timeout_secs,
            log,
            analysis,
            target_count,
            run_secs,
            elapsed_secs,
            root: root.map(Path::to_path_buf),
            record,
        })
    }

    /// The covered range in whole seconds `[start, end)`: the window when only it is exported,
    /// the whole run (`None`) otherwise.
    fn range(&self) -> Option<Range<u32>> {
        self.window
            .filter(|_| self.window_only)
            .map(|(first, last)| first as u32..last as u32 + 1)
    }

    /// The statistics of the covered range. Reads the whole log: run it in the background.
    fn covered_stats(&self) -> Arc<WindowStats> {
        if let Some(stats) = &self.analysis {
            return stats.0.clone();
        }
        Arc::new(match &self.log {
            Some(log) => window_stats(
                log,
                self.target_count,
                self.run_secs,
                self.range(),
                self.elapsed_secs,
            ),
            None => stats_from_snapshot(&self.snapshot),
        })
    }

    /// [`Self::covered_stats`] reporting its pass over the log to `progress`, `None` when it is
    /// cancelled.
    fn covered_stats_with(&self, progress: &Progress) -> Option<Arc<WindowStats>> {
        match (&self.analysis, &self.log) {
            (None, Some(log)) => window_stats_with(
                log,
                self.target_count,
                self.run_secs,
                self.range(),
                self.elapsed_secs,
                progress,
            )
            .map(Arc::new),
            _ => Some(self.covered_stats()),
        }
    }

    /// How many samples the log has, `0` without one.
    fn samples(&self) -> u64 {
        self.log.as_ref().map_or(0, |log| log.len() as u64)
    }

    /// How many steps exporting `format` takes, see [`run_export`].
    fn steps(&self, format: ExportFormat) -> u32 {
        u32::from(format.needs_stats()) + u32::from(format.is_report()) + 2
    }

    /// The report of the covered range, with its texts in the interface language. Its Latency
    /// section reports to `progress` and is left out when it is cancelled.
    fn report(&self, stats: &WindowStats, progress: &Progress) -> Report {
        let summary = SummaryData::from_stats(stats);
        let colors = ReportColors::light();
        let input = ReportInput {
            run_number: self.number,
            target_label: &self.target_label,
            status_label: &self.status_label,
            connection_label: self.connection_label.as_deref(),
            started_at: self.started_at,
            vus: self.vus,
            duration_secs: self.duration_secs,
            summary: &summary,
            compare: self
                .compare
                .as_ref()
                .map(|(number, snapshot)| (*number, snapshot)),
            thresholds: &self.thresholds,
            run_inputs: SummaryData::from_snapshot(&self.snapshot).threshold_inputs(),
            chart: self.chart.as_deref(),
            window: self.window,
            window_only: self.window_only,
            targets: &self.targets,
            log: self.log.as_deref(),
            target_count: self.target_count,
            error_groups: &stats.error_groups,
            error_examples: &self.snapshot.error_examples,
            timeout_secs: self.timeout_secs,
            generated_at: unix_now(),
            colors: &colors,
            progress,
        };
        build_report(&input)
    }

    /// The suggested file name: `run-0003.md`, or `run-0003-34-46s.md` for a window.
    fn file_name(&self, format: ExportFormat) -> String {
        let window = self
            .window
            .filter(|_| self.window_only)
            .map(|(first, last)| format!("-{first}-{}s", last + 1))
            .unwrap_or_default();
        format!("run-{:04}{window}.{}", self.number, format.extension())
    }

    /// The bytes of `format`, from the report or the data, `Ok(None)` when `progress` is
    /// cancelled. Slow: run it in the background.
    fn render(
        &self,
        format: ExportFormat,
        report: Option<&Report>,
        stats: Option<&WindowStats>,
        progress: &Progress,
    ) -> Result<Option<Vec<u8>>, String> {
        match (format, report) {
            (ExportFormat::Markdown, Some(report)) => Ok(Some(to_markdown(report).into_bytes())),
            (ExportFormat::Pdf, Some(report)) => to_pdf(
                report,
                &PdfFonts {
                    regular: GEIST_REGULAR,
                    medium: GEIST_MEDIUM,
                    semibold: GEIST_SEMIBOLD,
                    mono: GEIST_MONO_REGULAR,
                },
            )
            .map(Some)
            .map_err(|error| error.to_string()),
            (ExportFormat::Csv, _) => {
                let empty = SampleLog::new();
                let log = self.log.as_deref().unwrap_or(&empty);
                Ok(
                    requests_csv_with(log, &self.targets, self.range(), progress)
                        .map(String::into_bytes),
                )
            }
            (ExportFormat::Json, _) => {
                let Some(stats) = stats else {
                    return Err("no statistics to export".to_string());
                };
                // The record saved when the run ended is the reference; the tab's own copy
                // stands in when it cannot be read.
                let saved = self
                    .root
                    .as_deref()
                    .and_then(|root| history::load(root, self.number).ok());
                let record = saved.as_ref().unwrap_or(&self.record);
                Ok(
                    run_json_with(record, self.log.as_deref(), stats, self.range(), progress)
                        .map(String::into_bytes),
                )
            }
            _ => Err("no report to render".to_string()),
        }
    }
}

/// The current time as Unix seconds.
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// What a step of an export does, the `stage` of its [`Progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportStage {
    /// The statistics of the covered range.
    Statistics = 1,
    /// The report's Latency section.
    Latency = 2,
    /// Building the file's bytes.
    Render = 3,
    /// Writing them.
    Write = 4,
}

impl ExportStage {
    /// The stage `progress` is at, the first one before any.
    fn of(progress: &Progress) -> Self {
        match progress.stage() {
            2 => ExportStage::Latency,
            3 => ExportStage::Render,
            4 => ExportStage::Write,
            _ => ExportStage::Statistics,
        }
    }

    /// Starts this stage on `progress`, `units` long.
    fn begin(self, progress: &Progress, units: u64) {
        progress.begin_step(self as u32, units);
    }

    /// The line under the modal's title.
    fn label(self, format: ExportFormat) -> String {
        match self {
            ExportStage::Statistics => t!("load_test.export.progress.statistics").into_owned(),
            ExportStage::Latency => t!("load_test.export.progress.latency").into_owned(),
            ExportStage::Render => {
                t!("load_test.export.progress.render", format = format.title()).into_owned()
            }
            ExportStage::Write => t!("load_test.export.progress.write").into_owned(),
        }
    }
}

/// How an export that did not fail ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportOutcome {
    /// The file is saved.
    Written,
    /// It was cancelled; nothing was saved.
    Cancelled,
}

/// Exports `source` as `format` to `path`, one step after another (see [`ExportSource::steps`]):
/// the statistics when the format needs them, the report when it is one, the file's bytes and
/// writing them. Reports to `progress` and stops at its next check once it is cancelled. Slow:
/// run it in the background.
fn run_export(
    source: &ExportSource,
    format: ExportFormat,
    path: &Path,
    progress: &Progress,
) -> Result<ExportOutcome, String> {
    let samples = source.samples();
    let stats = if format.needs_stats() {
        let units = if source.analysis.is_some() {
            0
        } else {
            samples
        };
        ExportStage::Statistics.begin(progress, units);
        let Some(stats) = source.covered_stats_with(progress) else {
            return Ok(ExportOutcome::Cancelled);
        };
        Some(stats)
    } else {
        None
    };
    let report = match (&stats, format.is_report()) {
        (Some(stats), true) => {
            let units = source.log.as_ref().map_or(0, |log| {
                latency_units(log.len(), source.target_count.max(source.targets.len()))
            });
            ExportStage::Latency.begin(progress, units);
            let report = source.report(stats, progress);
            if progress.is_cancelled() {
                return Ok(ExportOutcome::Cancelled);
            }
            Some(report)
        }
        _ => None,
    };
    ExportStage::Render.begin(progress, samples * format.request_passes());
    let Some(bytes) = source.render(format, report.as_ref(), stats.as_deref(), progress)? else {
        return Ok(ExportOutcome::Cancelled);
    };
    ExportStage::Write.begin(progress, bytes.len() as u64);
    write_file(path, &bytes, progress)
}

/// Writes `bytes` to `path` by chunks, reported to `progress`: first to a `.part` file next to
/// it, renamed to `path` once complete, so a cancelled or failed export leaves whatever `path`
/// held untouched.
fn write_file(path: &Path, bytes: &[u8], progress: &Progress) -> Result<ExportOutcome, String> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    let partial = path.with_file_name(name);
    let written = write_chunks(&partial, bytes, progress).and_then(|complete| {
        if complete {
            std::fs::rename(&partial, path)?;
        }
        Ok(complete)
    });
    match written {
        Ok(true) => Ok(ExportOutcome::Written),
        Ok(false) => {
            let _ = std::fs::remove_file(&partial);
            Ok(ExportOutcome::Cancelled)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            Err(error.to_string())
        }
    }
}

/// Writes `bytes` to a new file at `path`, [`WRITE_CHUNK`] at a time; `false` when `progress` was
/// cancelled before the end.
fn write_chunks(path: &Path, bytes: &[u8], progress: &Progress) -> std::io::Result<bool> {
    let mut file = File::create(path)?;
    for chunk in bytes.chunks(WRITE_CHUNK) {
        if progress.is_cancelled() {
            return Ok(false);
        }
        file.write_all(chunk)?;
        progress.advance(chunk.len() as u64);
    }
    Ok(true)
}

/// The export in progress. There is at most one: the modal keeps the user from starting another.
pub(crate) struct ExportJob {
    /// What is exported, for the modal's texts.
    format: ExportFormat,
    /// How far it has got. Also tells this export apart from a later one.
    progress: Arc<Progress>,
    /// Whether its progress modal is open.
    dialog_open: bool,
}

/// Opens the progress modal of the export `progress` reports to: its step, a bar with the
/// percent done and Cancel, the only way to close it before the export ends.
fn open_export_dialog(
    view: WeakEntity<AppView>,
    format: ExportFormat,
    progress: Arc<Progress>,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, window, _| {
        let view = view.clone();
        let progress = progress.clone();
        framed_dialog(
            dialog.overlay_closable(false).keyboard(false),
            MODAL_WIDTH_DEFAULT,
            None,
            ModalPlacement::DEFAULT,
            window,
            move |_, cx| {
                let palette = cx.palette();
                let fraction = progress.fraction();
                let percent = format!("{} %", format_integer((fraction * 100.0).floor() as u64));
                let cancel_progress = progress.clone();
                let cancel_view = view.clone();
                v_flex()
                    .child(
                        // `modal_header` without its close button: only Cancel ends the export.
                        h_flex()
                            .h(px(MODAL_HEADER_HEIGHT))
                            .flex_none()
                            .items_center()
                            .px(px(20.0))
                            .border_b_1()
                            .border_color(palette.border)
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!(
                                "load_test.export.progress.title",
                                format = format.title()
                            )),
                    )
                    .child(
                        modal_body()
                            .child(
                                div()
                                    .text_color(palette.fg_muted)
                                    .child(ExportStage::of(&progress).label(format)),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap(px(12.0))
                                    .child(ProgressBar::new(fraction))
                                    .child(
                                        div()
                                            .flex_none()
                                            .min_w(px(40.0))
                                            .text_right()
                                            .text_color(palette.fg_muted)
                                            .child(percent),
                                    ),
                            ),
                    )
                    .child(
                        modal_footer(cx).child(
                            SecondaryButton::new("load-test-export-cancel", t!("common.cancel"))
                                .on_click(move |_, window, cx| {
                                    cancel_progress.cancel();
                                    let _ = cancel_view.update(cx, |view, _| {
                                        if let Some(job) = view.export_job.as_mut() {
                                            job.dialog_open = false;
                                        }
                                    });
                                    window.close_dialog(cx);
                                }),
                        ),
                    )
                    .into_any_element()
            },
        )
    });
}

impl AppView {
    /// The export source of `tab_id`, or `None` for a tab that is not a finished load test.
    fn load_test_export_source(&self, tab_id: &str) -> Option<ExportSource> {
        let load_test = self
            .state
            .tabs
            .index_of(tab_id)
            .and_then(|index| self.state.tabs.get(index))
            .and_then(|tab| tab.load_test())?;
        ExportSource::from_tab(
            load_test,
            self.send_options.timeout.as_secs(),
            self.state
                .workspace
                .as_ref()
                .map(|workspace| workspace.root()),
        )
    }

    /// Asks where to save the results of `tab_id`'s run as `format`, then writes them there.
    pub(crate) fn export_load_test(
        &mut self,
        tab_id: &str,
        format: ExportFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.export_job.is_some() {
            return;
        }
        let Some(source) = self.load_test_export_source(tab_id) else {
            return;
        };
        let directory = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.root().to_path_buf())
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        // `BRASA_EXPORT_DIR`, a hidden debug hook for the website's screenshots: saves there
        // under the suggested name instead of asking, since scripts cannot drive the native dialog.
        let preset = std::env::var_os("BRASA_EXPORT_DIR")
            .map(|dir| PathBuf::from(dir).join(source.file_name(format)));
        let receiver = preset
            .is_none()
            .then(|| cx.prompt_for_new_path(&directory, Some(&source.file_name(format))));
        let source = Arc::new(source);
        cx.spawn_in(window, async move |this, cx| {
            let path = match (preset, receiver) {
                (Some(path), _) => path,
                (None, Some(receiver)) => {
                    let Ok(Ok(Some(path))) = receiver.await else {
                        return;
                    };
                    path
                }
                (None, None) => return,
            };
            let progress = Arc::new(Progress::new(source.steps(format)));
            let started = this.update_in(cx, |view, window, cx| {
                if view.export_job.is_some() {
                    return false;
                }
                view.export_job = Some(ExportJob {
                    format,
                    progress: progress.clone(),
                    dialog_open: false,
                });
                view.watch_export(progress.clone(), window, cx);
                true
            });
            if !matches!(started, Ok(true)) {
                return;
            }
            let number = source.number;
            let target = path.clone();
            let worker = progress.clone();
            let outcome = cx
                .background_spawn(async move { run_export(&source, format, &target, &worker) })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                if let Some(job) = view.export_job.take()
                    && job.dialog_open
                {
                    window.close_dialog(cx);
                }
                let notification = match outcome {
                    Ok(ExportOutcome::Written) => {
                        log::info!("load test run {number} exported as {}", format.name());
                        let shown = shorten_path(&path, dirs::home_dir().as_deref());
                        Notification::success(
                            t!("load_test.export.saved", path = shown).into_owned(),
                        )
                    }
                    Ok(ExportOutcome::Cancelled) => {
                        log::info!(
                            "load test run {number} export as {} cancelled",
                            format.name()
                        );
                        Notification::info(t!("load_test.export.cancelled").into_owned())
                    }
                    Err(error) => {
                        log::warn!(
                            "load test run {number} could not be exported as {}: {error}",
                            format.name()
                        );
                        Notification::error(
                            t!("load_test.export.failed", error = error).into_owned(),
                        )
                    }
                };
                window.push_notification(notification, cx);
            });
        })
        .detach();
    }

    /// Redraws the progress of the export `progress` reports to while it runs: nothing during
    /// its first [`EXPORT_DIALOG_DELAY`], then its modal every [`EXPORT_REPAINT`]. Stops when the
    /// export ends.
    fn watch_export(
        &mut self,
        progress: Arc<Progress>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let started = Instant::now();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let delay = if started.elapsed() < EXPORT_DIALOG_DELAY {
                    EXPORT_DIALOG_DELAY
                } else {
                    EXPORT_REPAINT
                };
                cx.background_executor().timer(delay).await;
                let running = this.update_in(cx, |view, window, cx| {
                    let weak = cx.weak_entity();
                    let Some(job) = view
                        .export_job
                        .as_mut()
                        .filter(|job| Arc::ptr_eq(&job.progress, &progress))
                    else {
                        return false;
                    };
                    if !job.dialog_open && !progress.is_cancelled() {
                        job.dialog_open = true;
                        open_export_dialog(weak, job.format, progress.clone(), window, cx);
                    }
                    window.refresh();
                    true
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Handles the `Ctrl+Shift+C` / `Cmd+Shift+C` key binding (see `main.rs`'s `bind_keys`):
    /// copies the summary of the active tab when it is a load test, otherwise does nothing.
    pub(crate) fn on_copy_load_test_summary_action(
        &mut self,
        _: &CopyLoadTestSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .state
            .tabs
            .active()
            .filter(|tab| tab.load_test().is_some())
        else {
            return;
        };
        let tab_id = tab.id.clone();
        self.copy_load_test_summary(&tab_id, window, cx);
    }

    /// Copies the summary of `tab_id`'s run to the clipboard as Markdown.
    pub(crate) fn copy_load_test_summary(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.load_test_export_source(tab_id) else {
            return;
        };
        let source = Arc::new(source);
        cx.spawn_in(window, async move |this, cx| {
            let covered = source.clone();
            let stats = cx
                .background_spawn(async move { covered.covered_stats() })
                .await;
            let number = source.number;
            let markdown = cx
                .background_spawn(async move {
                    to_markdown(&source.report(&stats, &Progress::default()))
                })
                .await;
            let _ = this.update_in(cx, |_, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(markdown));
                log::info!("load test run {number} summary copied as markdown");
                window.push_notification(
                    Notification::success(t!("load_test.export.copied").into_owned()),
                    cx,
                );
            });
        })
        .detach();
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use brasa_core::Method;
    use brasa_load::{SampleRecord, SecondPoint, StatusKey};
    // `gpui_kit::*` brings gpui's own `#[test]`, which needs a gpui test context.
    use std::prelude::v1::test;

    /// A finished run of 6 seconds with two requests per second, the last second all errors.
    fn finished_tab() -> LoadTestTab {
        let mut log = SampleLog::new();
        let url = log.intern("http://localhost/users");
        for second in 0..6u32 {
            for half in 0..2 {
                let failed = second == 5;
                log.push(SampleRecord {
                    elapsed_ms: second * 1_000 + half * 500,
                    vu: half,
                    target: 0,
                    url,
                    status: if failed {
                        StatusKey::Code(503)
                    } else {
                        StatusKey::Code(200)
                    },
                    latency_us: 20_000,
                    size: 100,
                    is_error: failed,
                });
            }
        }
        let stats = window_stats(&log, 1, 6, None, 6.0);
        let mut snapshot = LoadSnapshot {
            elapsed: std::time::Duration::from_secs(6),
            active_vus: 0,
            total: stats.total,
            rps: stats.rps,
            p50: stats.p50,
            p95: stats.p95,
            p99: stats.p99,
            error_rate: stats.error_rate,
            series: (0..6)
                .map(|_| SecondPoint {
                    rps: 2.0,
                    vus: 2,
                    p50: 20_000,
                    p90: 20_000,
                    p95: 20_000,
                    p99: 20_000,
                })
                .collect(),
            histogram: [0; 26],
            status_counts: stats.statuses.clone(),
            per_target: stats.per_target.clone(),
            error_examples: Vec::new(),
        };
        snapshot.histogram[2] = stats.total;
        let mut tab = LoadTestTab::for_request("users.brasa".to_string(), "users".to_string());
        tab.status = LoadTestStatus::Finished;
        tab.run_number = Some(3);
        tab.run_duration_secs = Some(6);
        tab.target_rows = vec![(Method::Get, "users".to_string())];
        tab.snapshot = Some(snapshot);
        tab.replace_log(Some(log));
        tab
    }

    #[test]
    fn nothing_is_exported_while_running_or_without_results() {
        let mut tab = finished_tab();
        assert!(ExportSource::from_tab(&tab, 30, None).is_some());
        tab.status = LoadTestStatus::Running;
        assert!(ExportSource::from_tab(&tab, 30, None).is_none());
        assert!(ExportSource::from_tab(&LoadTestTab::unset(), 30, None).is_none());
    }

    #[test]
    fn the_file_name_and_range_follow_the_window() {
        let mut tab = finished_tab();
        tab.window = Some((2, 4));
        tab.export_window_only = false;
        let whole = ExportSource::from_tab(&tab, 30, None).expect("a finished run");
        assert_eq!(whole.file_name(ExportFormat::Markdown), "run-0003.md");
        assert_eq!(whole.range(), None);

        tab.export_window_only = true;
        let part = ExportSource::from_tab(&tab, 30, None).expect("a finished run");
        assert_eq!(part.file_name(ExportFormat::Pdf), "run-0003-2-5s.pdf");
        assert_eq!(part.range(), Some(2..5));
        assert_eq!(part.covered_stats().total, 6);

        // Without the requests there is no window to cut.
        tab.replace_log(None);
        tab.window = Some((2, 4));
        let no_log = ExportSource::from_tab(&tab, 30, None).expect("a finished run");
        assert_eq!(no_log.range(), None);
        assert_eq!(no_log.file_name(ExportFormat::Csv), "run-0003.csv");
        assert_eq!(no_log.covered_stats().total, 12);
    }

    #[test]
    fn every_format_renders() {
        let tab = finished_tab();
        let source = ExportSource::from_tab(&tab, 30, None).expect("a finished run");
        let progress = Progress::default();
        let stats = source.covered_stats();
        assert_eq!(stats.total, 12);
        let report = source.report(&stats, &progress);
        assert_eq!(report.title, "Run #3 \u{b7} users");
        assert_eq!(report.errors.len(), 1);

        let render = |format, report| {
            source
                .render(format, report, Some(&stats), &progress)
                .expect("rendered")
                .expect("not cancelled")
        };
        let markdown = render(ExportFormat::Markdown, Some(&report));
        assert!(String::from_utf8_lossy(&markdown).contains("Run #3"));

        let pdf = render(ExportFormat::Pdf, Some(&report));
        assert!(pdf.starts_with(b"%PDF"));

        let csv = render(ExportFormat::Csv, None);
        assert_eq!(String::from_utf8_lossy(&csv).lines().count(), 13);

        let json = render(ExportFormat::Json, None);
        let json: serde_json::Value = serde_json::from_slice(&json).expect("valid json");
        assert_eq!(json["run"]["number"], 3);
        assert_eq!(json["requests"].as_array().map(Vec::len), Some(12));
    }

    /// A fresh directory in the system temporary folder for one test's files.
    fn scratch_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("brasa-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    #[test]
    fn an_export_runs_every_step_and_writes_the_file() {
        let tab = finished_tab();
        let source = ExportSource::from_tab(&tab, 30, None).expect("a finished run");
        let dir = scratch_dir("written");
        for format in [ExportFormat::Pdf, ExportFormat::Csv, ExportFormat::Json] {
            let path = dir.join(source.file_name(format));
            let progress = Progress::new(source.steps(format));
            let outcome = run_export(&source, format, &path, &progress).expect("exported");
            assert_eq!(outcome, ExportOutcome::Written);
            assert_eq!(progress.fraction(), 1.0, "{format:?}");
            assert_eq!(ExportStage::of(&progress), ExportStage::Write);
            assert!(path.exists());
        }
        let left: Vec<_> = std::fs::read_dir(&dir)
            .expect("the directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .collect();
        assert!(left.is_empty(), "no partial file is left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_export_keeps_the_existing_file() {
        let tab = finished_tab();
        let source = ExportSource::from_tab(&tab, 30, None).expect("a finished run");
        let dir = scratch_dir("cancelled");
        let path = dir.join("run.csv");
        std::fs::write(&path, "before").expect("an existing file");
        let progress = Progress::new(source.steps(ExportFormat::Csv));
        progress.cancel();
        let outcome = run_export(&source, ExportFormat::Csv, &path, &progress).expect("no error");
        assert_eq!(outcome, ExportOutcome::Cancelled);
        assert_eq!(std::fs::read_to_string(&path).expect("kept"), "before");
        assert_eq!(std::fs::read_dir(&dir).expect("the directory").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
