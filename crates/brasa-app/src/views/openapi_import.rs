//! The import wizard (GitHub #201, Claude Design "OpenAPI Import", 1a, and #211, "Import Flow", 4):
//! three steps, Source, Options and Preview, in one modal, then a Done step with the result, for
//! OpenAPI specs and Postman exports alike. The format is detected from what is read. The logic
//! lives in `state/openapi_import.rs`; this renders it and runs the blocking parts (reading a
//! file, downloading a URL) on the background executor.
//!
//! The workspace is the destination of the import, chosen in the Options step and created when
//! the user confirms (GitHub #208, `views/import_destination.rs`).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_format::openapi::{ChangeKind, Grouping, ManifestSource, PlanChange};
use brasa_format::postman::{IssueLevel, PostmanFormat};
use brasa_runner::Client;
use brasa_workspace::{
    ImportOrigin, ImportReport, ProjectWrite, Workspace, WorkspaceError, openapi_collections_in,
};
use telaio_ui::components::{CheckState, TextArea, TriCheckbox};
use telaio_ui::metrics::CHECKBOX_HALO_PADDING;

use crate::state::env_color::EnvColor;
use crate::state::import_destination::{Target, slug};
use crate::state::locale::plural;
use crate::state::number::format_integer;
use crate::state::openapi_import::{
    Check, ImportFormat, Loaded, LoadedSpec, PreviewRow, RowKind, SourceKind, Step, Wizard,
    clone_repository, find_existing, issue_text, load_files, load_pasted, load_url, report_text,
    scan_folder, source_kind_name, source_kind_of, sources_of,
};
use crate::theme::metrics::{RADIUS_MD, RADIUS_SM};
use crate::theme::{Palette, PaletteExt, BrasaColors};
use crate::views::components::{
    GhostButton, IconButton, InlineMessage, InlineMessageKind, MethodBadge, ModalPlacement,
    PrimaryButton, SecondaryButton, SectionLabel, TreeRow, framed_dialog, text_field,
};

use super::import_destination::{
    DestinationBlock, ResolvedDestination, import_outside, render_destination,
};
use super::root::AppView;

/// The card's size, from the design. Fixed, so the steps do not change the dialog's size.
const DIALOG_WIDTH: f32 = 780.0;
const DIALOG_HEIGHT: f32 = 600.0;
/// Horizontal padding of the header, the stepper, the body and the footer.
const PADDING_X: f32 = 20.0;
/// Size of a stepper dot.
const STEP_DOT: f32 = 22.0;
/// Height of a source tab.
const SOURCE_TAB_HEIGHT: f32 = 30.0;
/// Height of the file drop zone.
const DROP_ZONE_HEIGHT: f32 = 150.0;
/// Height of the paste text area.
const PASTE_HEIGHT: f32 = 200.0;
/// Width of the Git source's branch field, from the design.
const GIT_BRANCH_WIDTH: f32 = 140.0;
/// Height of a row of the preview tree and of the list of changes.
const ROW_HEIGHT: f32 = 28.0;
/// Width of the change kind column of the list of changes ("Added", "Removed").
const CHANGE_KIND_WIDTH: f32 = 72.0;
/// Widest a change's details get before they are cut.
const CHANGE_DETAIL_WIDTH: f32 = 240.0;
/// Width of a Postman request's path in the preview tree.
const ROW_PATH_WIDTH: f32 = 112.0;
/// How many steps the stepper shows (Done is not one of them).
const STEP_COUNT: usize = 3;

/// The result shown by the Done step.
struct Done {
    count: usize,
    name: String,
    warnings: Vec<String>,
    /// `(added, changed, removed)` when the wizard updated an existing collection.
    updated: Option<(usize, usize, usize)>,
    /// The workspace the import went to when it is not the one that was open.
    workspace: Option<String>,
    /// Whether that workspace was created by the import.
    created: bool,
    /// The import report, when there is something in it.
    report: Option<PathBuf>,
    /// How many requests need a review.
    review: usize,
}

/// The wizard's entity: the plain state plus the inputs and what is being loaded.
pub(crate) struct ImportWizard {
    state: Wizard,
    /// Whether the format comes from what was read, rather than from the chips.
    detected: bool,
    view: WeakEntity<AppView>,
    /// Sends the spec downloads, with the TLS settings of requests. Built on the first load: the
    /// wizard may open from inside an `AppView` update, where `AppView` cannot be read.
    client: Option<Arc<Client>>,
    name: Entity<InputState>,
    /// The destination block, built when the Options step first shows.
    destination: Option<Entity<DestinationBlock>>,
    url: Entity<InputState>,
    paste: Entity<TextareaState>,
    repository: Entity<InputState>,
    branch: Entity<InputState>,
    /// The folder of the Folder source.
    folder: Option<PathBuf>,
    /// The source of the specs loaded, for the manifest and to find an earlier import of it.
    source: Option<ManifestSource>,
    /// The git Brasa uses, read on the first clone like the client.
    git: Option<Option<telaio_git::Git>>,
    /// The temporary checkout of the Git source, deleted when the wizard closes or the source
    /// changes.
    checkout: Option<tempfile::TempDir>,
    /// Whether a file is being read or a URL downloaded.
    loading: bool,
    /// Why the last load failed.
    error: Option<String>,
    /// Whether the name input still has to be filled from the loaded spec.
    name_pending: bool,
    /// Moves to the next step once the load in progress succeeds (Next on a URL or pasted text).
    advance_after_load: bool,
    done: Option<Done>,
    rows_scroll: UniformListScrollHandle,
    body_scroll: ScrollHandle,
    /// Holds the keyboard on the steps without an input, so Enter still reaches the dialog.
    focus: FocusHandle,
    /// The step the keyboard focus was last placed for.
    focused_step: Option<Step>,
}

impl ImportWizard {
    fn load(&mut self, kind: SourceKind, input: LoadInput, advance: bool, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        self.advance_after_load = advance;
        cx.notify();
        let client = match &self.client {
            Some(client) => client.clone(),
            None => {
                let options = self
                    .view
                    .upgrade()
                    .map(|view| view.read(cx).send_options.clone())
                    .unwrap_or_default();
                let client = Arc::new(Client::new(options));
                self.client = Some(client.clone());
                client
            }
        };
        if matches!(input, LoadInput::Git { .. }) && self.git.is_none() {
            let git = self
                .view
                .upgrade()
                .and_then(|view| view.read(cx).git.availability.git().cloned());
            self.git = Some(git);
        }
        let git = self.git.clone().flatten();
        let source = manifest_source(kind, &input);
        let task = cx.background_executor().spawn(async move {
            let single = |result: Result<LoadedSpec, String>| {
                result.map(|spec| {
                    (
                        None,
                        Loaded {
                            specs: vec![spec],
                            ..Loaded::default()
                        },
                    )
                })
            };
            match input {
                LoadInput::Files(paths) => load_files(&paths, &client).map(|loaded| (None, loaded)),
                LoadInput::Folder(path) => scan_folder(&path, &client).map(|loaded| (None, loaded)),
                LoadInput::Git { url, branch } => match git {
                    Some(git) => clone_repository(&git, &url, &branch, &client)
                        .map(|(checkout, loaded)| (Some(checkout), loaded)),
                    None => Err(t!("openapi.git.missing").into_owned()),
                },
                LoadInput::Url(url) => single(load_url(&url, &client)),
                LoadInput::Text(text) => load_pasted(&text).map(|loaded| (None, loaded)),
            }
        });
        cx.spawn(async move |wizard, cx| {
            let result = task.await;
            let _ = wizard.update(cx, |wizard, cx| wizard.loaded(kind, source, result, cx));
        })
        .detach();
    }

    fn loaded(
        &mut self,
        kind: SourceKind,
        source: ManifestSource,
        result: Result<(Option<tempfile::TempDir>, Loaded), String>,
        cx: &mut Context<Self>,
    ) {
        self.loading = false;
        // The user switched to another source meanwhile.
        if self.state.source != kind {
            cx.notify();
            return;
        }
        match result {
            Ok((checkout, loaded)) => {
                log::info!(
                    "import: read {} Postman exports, {} specs ({} operations), {} files failed, {} catalogs",
                    loaded.postman.len(),
                    loaded.specs.len(),
                    loaded
                        .specs
                        .iter()
                        .map(|spec| spec.spec.operations.len())
                        .sum::<usize>(),
                    loaded.failures.len(),
                    loaded.catalogs.len()
                );
                let found = !loaded.specs.is_empty()
                    || !loaded.postman.is_empty()
                    || loaded.bruno.is_some();
                if !found && loaded.failures.is_empty() {
                    self.error = Some(match self.state.format {
                        ImportFormat::OpenApi => t!("openapi.none_found").into_owned(),
                        ImportFormat::Postman => t!("import.postman.none_found").into_owned(),
                        ImportFormat::Bruno => t!("import.bruno.none_found").into_owned(),
                    });
                }
                self.checkout = checkout;
                let detected = loaded.format();
                self.state.set_loaded(loaded);
                self.detected = detected.is_some();
                self.name_pending = found;
                if found {
                    let source = match self.state.format {
                        ImportFormat::Postman => self.state.postman_source(),
                        ImportFormat::Bruno => self.state.bruno_source(),
                        ImportFormat::OpenApi => source,
                    };
                    // An earlier import of the same source turns the Preview into an update.
                    let collections = self
                        .view
                        .upgrade()
                        .and_then(|view| {
                            view.read(cx)
                                .state
                                .workspace
                                .as_ref()
                                .map(|workspace| workspace.openapi_collections())
                        })
                        .unwrap_or_default();
                    self.state.set_existing(find_existing(collections, &source));
                    self.source = Some(source);
                }
                if self.advance_after_load && found {
                    self.state.step = Step::Options;
                }
            }
            Err(error) => {
                log::warn!("import: could not read the source: {error}");
                self.error = Some(t!("openapi.read_failed", error = error).into_owned());
            }
        }
        self.advance_after_load = false;
        cx.notify();
    }

    fn select_source(&mut self, source: SourceKind, cx: &mut Context<Self>) {
        if self.state.source == source {
            return;
        }
        self.state.source = source;
        self.state.set_specs(Vec::new());
        self.checkout = None;
        self.error = None;
        cx.notify();
    }

    /// Clears what a source input loaded once its text changes.
    fn source_edited(&mut self, cx: &mut Context<Self>) {
        if !self.state.is_empty() || self.error.is_some() {
            self.state.set_specs(Vec::new());
            self.checkout = None;
            self.error = None;
            cx.notify();
        }
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        self.state.rebuild(&name);
        cx.notify();
    }

    /// The destination of the import, `None` while the block has something to answer.
    fn resolved(&self, cx: &App) -> Option<ResolvedDestination> {
        self.destination.as_ref()?.read(cx).resolved(cx)
    }

    /// The name of the collection folder the import would write.
    fn collection_name(&self, cx: &App) -> String {
        let name = self.name.read(cx).value();
        let name = name.trim();
        if name.is_empty() {
            self.state.suggested_name()
        } else {
            name.to_string()
        }
    }

    /// Looks for an earlier import of this source in the destination: the Preview offers to
    /// update it. The collection may be there only when the destination changed from the
    /// workspace that was open when the source was read.
    fn find_in_destination(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(destination) = self.resolved(cx) else {
            return;
        };
        let collections = match destination.target.path() {
            Some(path) => openapi_collections_in(path),
            None => self
                .view
                .upgrade()
                .and_then(|view| {
                    view.read(cx)
                        .state
                        .workspace
                        .as_ref()
                        .map(Workspace::openapi_collections)
                })
                .unwrap_or_default(),
        };
        let found = find_existing(collections, &source);
        let known = self.state.existing().map(|existing| &existing.folder);
        if found.as_ref().map(|existing| &existing.folder) != known {
            self.state.set_existing(found);
        }
    }

    /// Whether the import has no requests to write, only environments.
    fn environments_only(&self) -> bool {
        self.state.total_count() == 0
            && self
                .state
                .plan()
                .is_some_and(|plan| !plan.environments.is_empty())
    }

    /// Whether the primary button does something now.
    fn can_advance(&self, cx: &App) -> bool {
        if self.loading {
            return false;
        }
        match self.state.step {
            Step::Source => match self.state.source {
                SourceKind::File => self.state.has_content(),
                SourceKind::Url => {
                    self.state.has_content() || !self.url.read(cx).value().trim().is_empty()
                }
                SourceKind::Paste => {
                    self.state.has_content() || !self.paste.read(cx).value().trim().is_empty()
                }
                SourceKind::Folder => self.state.has_content(),
                SourceKind::Git => {
                    self.state.has_content() || !self.repository.read(cx).value().trim().is_empty()
                }
            },
            Step::Options => {
                let something = match self.state.format {
                    ImportFormat::OpenApi => self.state.enabled_count() > 0,
                    ImportFormat::Postman => (0..self.state.postman_files().len())
                        .any(|index| self.state.is_postman_enabled(index)),
                    ImportFormat::Bruno => self.state.bruno().is_some(),
                };
                something && self.resolved(cx).is_some()
            }
            Step::Preview if self.state.is_update() => self.state.selected_change_count() > 0,
            Step::Preview => self.state.selected_count() > 0 || self.environments_only(),
            Step::Done => true,
        }
    }
}

/// The manifest source of a load of `input` from the `kind` source.
fn manifest_source(kind: SourceKind, input: &LoadInput) -> ManifestSource {
    let canonical = |path: &PathBuf| {
        dunce::canonicalize(path)
            .unwrap_or_else(|_| path.clone())
            .to_string_lossy()
            .into_owned()
    };
    let (location, branch) = match input {
        LoadInput::Files(paths) => (
            paths.first().map(canonical).unwrap_or_default(),
            String::new(),
        ),
        LoadInput::Folder(path) => (canonical(path), String::new()),
        LoadInput::Git { url, branch } => (url.trim().to_string(), branch.trim().to_string()),
        LoadInput::Url(url) => (url.trim().to_string(), String::new()),
        LoadInput::Text(_) => (String::new(), String::new()),
    };
    ManifestSource {
        kind: source_kind_name(kind).to_string(),
        location,
        branch,
    }
}

/// Today, as a manifest writes it.
fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// What to load.
enum LoadInput {
    Files(Vec<PathBuf>),
    Folder(PathBuf),
    Git { url: String, branch: String },
    Url(String),
    Text(String),
}

/// Opens the wizard on its first step with `source` selected, for OpenAPI.
pub(crate) fn open_openapi_wizard(
    view: WeakEntity<AppView>,
    source: SourceKind,
    window: &mut Window,
    cx: &mut App,
) {
    open_wizard(view, ImportFormat::OpenApi, source, window, cx);
}

/// Opens the wizard on its first step for Postman.
pub(crate) fn open_postman_wizard(view: WeakEntity<AppView>, window: &mut Window, cx: &mut App) {
    open_wizard(view, ImportFormat::Postman, SourceKind::File, window, cx);
}

/// Opens the wizard on its first step for Bruno, on its Folder source (#222).
pub(crate) fn open_bruno_wizard(view: WeakEntity<AppView>, window: &mut Window, cx: &mut App) {
    open_wizard(view, ImportFormat::Bruno, SourceKind::Folder, window, cx);
}

/// Opens the wizard on files dropped on the window (design 4e): they are read, the format is
/// detected and, when something was found, the wizard goes on to the Options step.
pub(crate) fn open_wizard_with_files(
    view: WeakEntity<AppView>,
    paths: Vec<PathBuf>,
    window: &mut Window,
    cx: &mut App,
) {
    let wizard = open_wizard(view, ImportFormat::OpenApi, SourceKind::File, window, cx);
    wizard.update(cx, |wizard, cx| {
        wizard.load(SourceKind::File, LoadInput::Files(paths), true, cx);
    });
}

/// Opens the wizard on the source an earlier import recorded, and reads it right away: once
/// read, the Preview step offers to update the collection (GitHub #203). Pasted text cannot be
/// read again and opens nothing.
pub(crate) fn open_openapi_update(
    view: WeakEntity<AppView>,
    source: &ManifestSource,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(kind) = source_kind_of(&source.kind) else {
        return;
    };
    let wizard = open_wizard(view, ImportFormat::OpenApi, kind, window, cx);
    let location = source.location.clone();
    let branch = source.branch.clone();
    wizard.update(cx, |wizard, cx| {
        let input = match kind {
            SourceKind::File => LoadInput::Files(vec![PathBuf::from(&location)]),
            SourceKind::Folder => {
                wizard.folder = Some(PathBuf::from(&location));
                LoadInput::Folder(PathBuf::from(&location))
            }
            SourceKind::Url => {
                wizard.url.update(cx, |state, cx| {
                    state.set_value(location.clone(), window, cx)
                });
                LoadInput::Url(location)
            }
            SourceKind::Git => {
                wizard.repository.update(cx, |state, cx| {
                    state.set_value(location.clone(), window, cx)
                });
                wizard
                    .branch
                    .update(cx, |state, cx| state.set_value(branch.clone(), window, cx));
                LoadInput::Git {
                    url: location,
                    branch,
                }
            }
            SourceKind::Paste => return,
        };
        wizard.load(kind, input, false, cx);
    });
}

/// Builds the wizard on `format` and `source` and opens it.
fn open_wizard(
    view: WeakEntity<AppView>,
    format: ImportFormat,
    source: SourceKind,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ImportWizard> {
    let name = cx.new(|cx| InputState::new(window, cx));
    let url = cx
        .new(|cx| InputState::new(window, cx).placeholder("https://api.example.com/openapi.yaml"));
    let paste =
        cx.new(|cx| TextareaState::new(window, cx).placeholder(t!("openapi.paste.placeholder")));
    let repository =
        cx.new(|cx| InputState::new(window, cx).placeholder("git@github.com:acme/payments.git"));
    let branch =
        cx.new(|cx| InputState::new(window, cx).placeholder(t!("openapi.git.branch_placeholder")));
    let wizard = cx.new(|cx| ImportWizard {
        state: Wizard::new(format, source),
        detected: false,
        view: view.clone(),
        client: None,
        name: name.clone(),
        destination: None,
        url: url.clone(),
        paste: paste.clone(),
        repository: repository.clone(),
        branch: branch.clone(),
        folder: None,
        source: None,
        git: None,
        checkout: None,
        loading: false,
        error: None,
        name_pending: false,
        advance_after_load: false,
        done: None,
        rows_scroll: UniformListScrollHandle::new(),
        body_scroll: ScrollHandle::new(),
        focus: cx.focus_handle(),
        focused_step: Some(Step::Source),
    });
    {
        // Weak, or the subscriptions would keep the wizard alive forever.
        let weak = wizard.downgrade();
        window
            .subscribe(&url, cx, move |_, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let _ = weak.update(cx, |wizard, cx| wizard.source_edited(cx));
                }
            })
            .detach();
        let weak = wizard.downgrade();
        window
            .subscribe(&paste, cx, move |_, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let _ = weak.update(cx, |wizard, cx| wizard.source_edited(cx));
                }
            })
            .detach();
        for input in [&repository, &branch] {
            let weak = wizard.downgrade();
            window
                .subscribe(input, cx, move |_, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let _ = weak.update(cx, |wizard, cx| wizard.source_edited(cx));
                    }
                })
                .detach();
        }
    }
    let dialog_wizard = wizard.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let wizard = dialog_wizard.clone();
        let enter = wizard.clone();
        // Enter is the primary button; the dialog stays open unless the wizard is done.
        let dialog = dialog.on_ok(move |_, window, cx| primary_action(&enter, window, cx));
        let wizard = wizard.clone();
        framed_dialog(
            dialog,
            DIALOG_WIDTH,
            Some(DIALOG_HEIGHT),
            ModalPlacement::Centered,
            window,
            move |window, cx| render_wizard(&wizard, window, cx),
        )
    });
    match source {
        SourceKind::Url => url.update(cx, |state, cx| state.focus(window, cx)),
        SourceKind::Git => repository.update(cx, |state, cx| state.focus(window, cx)),
        _ => {}
    }
    wizard
}

/// Runs the primary button (Next, Import or Close). Returns whether the dialog closes.
fn primary_action(wizard: &Entity<ImportWizard>, window: &mut Window, cx: &mut App) -> bool {
    if !wizard.read(cx).can_advance(cx) {
        return false;
    }
    let step = wizard.read(cx).state.step;
    match step {
        Step::Source => {
            wizard.update(cx, |wizard, cx| {
                if wizard.state.has_content() {
                    wizard.state.step = Step::Options;
                    cx.notify();
                    return;
                }
                match wizard.state.source {
                    SourceKind::File | SourceKind::Folder => {}
                    SourceKind::Git => {
                        let url = wizard.repository.read(cx).value().to_string();
                        let branch = wizard.branch.read(cx).value().to_string();
                        wizard.load(SourceKind::Git, LoadInput::Git { url, branch }, true, cx);
                    }
                    SourceKind::Url => {
                        let url = wizard.url.read(cx).value().to_string();
                        wizard.load(SourceKind::Url, LoadInput::Url(url), true, cx);
                    }
                    SourceKind::Paste => {
                        let text = wizard.paste.read(cx).value().to_string();
                        wizard.load(SourceKind::Paste, LoadInput::Text(text), true, cx);
                    }
                }
            });
            false
        }
        Step::Options => {
            wizard.update(cx, |wizard, cx| {
                wizard.find_in_destination(cx);
                wizard.rebuild(cx);
                wizard.state.step = Step::Preview;
                cx.notify();
            });
            false
        }
        Step::Preview => run_import(wizard, window, cx),
        Step::Done => true,
    }
}

/// Imports the selected requests into the destination and moves to the Done step. A failure
/// stays on the Preview step with the reason.
fn run_import(wizard: &Entity<ImportWizard>, window: &mut Window, cx: &mut App) -> bool {
    let (view, plan, excluded, origin, update, destination, issues, colors) = {
        let wizard = wizard.read(cx);
        let colors = wizard.state.bruno_environment_colors();
        // An update leaves the unchanged requests alone: there is nothing new to review.
        let issues = if wizard.state.is_collection_tool() && !wizard.state.is_update() {
            Some(wizard.state.selected_issues())
        } else {
            None
        };
        let Some(plan) = wizard.state.plan().cloned() else {
            return false;
        };
        let Some(destination) = wizard.resolved(cx) else {
            return false;
        };
        let origin = ImportOrigin {
            source: wizard.source.clone().unwrap_or_else(|| ManifestSource {
                kind: "paste".to_string(),
                ..ManifestSource::default()
            }),
            date: today(),
            options: wizard.state.options,
        };
        let update = wizard.state.is_update().then(|| {
            let folder = wizard
                .state
                .existing()
                .map(|existing| existing.folder.clone())
                .unwrap_or_default();
            let skipped: HashSet<String> = wizard
                .state
                .changes()
                .iter()
                .filter(|change| !wizard.state.is_change_selected(&change.key))
                .map(|change| change.key.clone())
                .collect();
            (folder, skipped)
        });
        (
            wizard.view.clone(),
            plan,
            wizard.state.excluded().clone(),
            origin,
            update,
            destination,
            issues,
            colors,
        )
    };
    let Some(app) = view.upgrade() else {
        return true;
    };
    let result = app.update(cx, |app, cx| match update {
        Some((folder, skipped)) => {
            let opened = folder.clone();
            app.import_to(
                &destination,
                |workspace| {
                    workspace.update_openapi(&folder, &plan, &|key| !skipped.contains(key), &origin)
                },
                move |_| Some(opened),
                window,
                cx,
            )
            .map(|report| Done {
                count: report.added + report.changed + report.removed,
                name: folder.clone(),
                updated: Some((report.added, report.changed, report.removed)),
                warnings: report.warnings,
                workspace: None,
                created: false,
                report: None,
                review: 0,
            })
        }
        None => app
            .import_to(
                &destination,
                |workspace| {
                    let report =
                        workspace.import_openapi(&plan, &|key| !excluded.contains(key), &origin)?;
                    write_environment_colors(workspace, &colors);
                    Ok(report)
                },
                collection_folder,
                window,
                cx,
            )
            .map(|report| Done {
                count: report
                    .created_files
                    .iter()
                    .filter(|id| id.ends_with(".brasa"))
                    .count(),
                name: plan.root.name.clone(),
                warnings: report.warnings,
                updated: None,
                workspace: None,
                created: false,
                report: None,
                review: 0,
            }),
    });
    wizard.update(cx, |wizard, cx| {
        match result {
            Ok(mut done) => {
                if let Some(path) = destination.target.path() {
                    done.workspace = path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned());
                    done.created = matches!(destination.target, Target::Create(_));
                }
                if let Some(issues) = &issues {
                    let root = destination.target.path().map(PathBuf::from).or_else(|| {
                        view.upgrade().and_then(|view| {
                            view.read(cx)
                                .state
                                .workspace
                                .as_ref()
                                .map(|workspace| workspace.root().to_path_buf())
                        })
                    });
                    let text = report_text(&done.name, issues, &done.warnings);
                    done.review = issues
                        .iter()
                        .filter(|(_, issues)| {
                            issues.iter().any(|issue| issue.level() != IssueLevel::Note)
                        })
                        .count();
                    if let Some(root) = root.filter(|_| !text.is_empty()) {
                        done.report = write_report(&root, &text);
                    }
                }
                wizard.done = Some(done);
                wizard.state.step = Step::Done;
            }
            Err(error) => wizard.error = Some(error),
        }
        cx.notify();
    });
    false
}

/// Saves the colors a Bruno import brought for its environments in `.brasa/project.toml`
/// (#217, #222), for the environments the workspace has now. A failure only loses the colors.
fn write_environment_colors(workspace: &Workspace, colors: &[(String, EnvColor)]) {
    if colors.is_empty() {
        return;
    }
    let existing = workspace.list_environments().unwrap_or_default();
    let colors: Vec<&(String, EnvColor)> = colors
        .iter()
        .filter(|(name, _)| existing.contains(name))
        .collect();
    if colors.is_empty() {
        return;
    }
    let result = workspace.update_project(ProjectWrite::Create, |config| {
        for (name, color) in &colors {
            config.set_environment_color(name, Some(color.key()));
        }
    });
    if let Err(error) = result {
        crate::state::workspace_log::log_workspace_error("save the environment colors", &error);
    }
}

/// Writes the import report to `.brasa/import-report.md`. A failure only loses the report.
fn write_report(root: &std::path::Path, text: &str) -> Option<PathBuf> {
    let folder = root.join(".brasa");
    let path = folder.join("import-report.md");
    let written = std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&path, text));
    match written {
        Ok(()) => Some(path),
        Err(error) => {
            log::warn!("could not write the import report: {error}");
            None
        }
    }
}

/// The workspace id of the collection folder an import report's first request is in.
fn collection_folder(report: &ImportReport) -> Option<String> {
    report
        .created_files
        .iter()
        .find(|id| id.ends_with(".brasa"))
        .and_then(|id| id.split('/').next())
        .map(str::to_string)
}

impl AppView {
    /// Runs `import` in the workspace of `destination`. In the open one the sidebar and the
    /// environments refresh; another one, created when needed, opens once the import is done. The
    /// collection folder `opened` picks from the result shows open in the sidebar.
    pub(crate) fn import_to<R: 'static>(
        &mut self,
        destination: &ResolvedDestination,
        import: impl FnOnce(&mut Workspace) -> Result<R, WorkspaceError>,
        opened: impl FnOnce(&R) -> Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<R, String> {
        let result = match destination.target.path() {
            None => match self.state.workspace.as_mut() {
                Some(workspace) => import(workspace),
                None => return Err("no workspace is open".to_string()),
            },
            Some(_) => import_outside(destination, import),
        };
        let report = result.map_err(|error| {
            crate::state::workspace_log::log_workspace_error("import", &error);
            error.to_string()
        })?;
        log::info!("import written to {:?}", destination.target);
        self.workspace_error = None;
        let collection = opened(&report);
        let expand = move |view: &mut Self, cx: &mut Context<Self>| {
            view.refresh_tree_with(cx, |expansion| {
                if let Some(collection) = collection {
                    expansion.insert(collection, true);
                }
            });
            view.refresh_environments();
            cx.notify();
        };
        match destination.target.path() {
            None => expand(self, cx),
            Some(path) => {
                self.open_workspace_at(path, window, cx);
                self.when_workspace_opens(window, cx, move |view, _, cx| expand(view, cx));
            }
        }
        Ok(report)
    }
}

fn render_wizard(wizard: &Entity<ImportWizard>, window: &mut Window, cx: &mut App) -> AnyElement {
    // The destination block is built when the Options step first shows, where `AppView` can be
    // read.
    if wizard.read(cx).destination.is_none() && wizard.read(cx).state.step == Step::Options {
        let view = wizard.read(cx).view.clone();
        let block = DestinationBlock::build(&view, window, cx);
        wizard.update(cx, |wizard, _| wizard.destination = Some(block));
    }
    // The source can change going back to the first step.
    if wizard.read(cx).state.step == Step::Options {
        let (block, folder) = {
            let wizard = wizard.read(cx);
            // A Bruno folder never becomes the workspace: .bru and .brasa files would mix.
            let folder = (wizard.state.source == SourceKind::Folder
                && wizard.state.format != ImportFormat::Bruno)
                .then(|| wizard.folder.clone())
                .flatten();
            (wizard.destination.clone(), folder)
        };
        if let Some(block) = block
            && block.read(cx).source_folder() != folder.as_ref()
        {
            block.update(cx, |block, cx| block.set_source_folder(folder, cx));
        }
    }
    // The name inputs are filled from the spec once, when the Options step first shows them.
    if wizard.read(cx).name_pending && wizard.read(cx).state.step == Step::Options {
        let (name, suggested, block) = {
            let wizard = wizard.read(cx);
            (
                wizard.name.clone(),
                wizard.state.suggested_name(),
                wizard.destination.clone(),
            )
        };
        if let Some(block) = block {
            let workspace_name = slug(&suggested);
            block.update(cx, |block, cx| {
                block.suggest_name(workspace_name, window, cx);
            });
        }
        name.update(cx, |state, cx| state.set_value(suggested, window, cx));
        wizard.update(cx, |wizard, cx| {
            wizard.name_pending = false;
            wizard.rebuild(cx);
        });
    }
    let step = wizard.read(cx).state.step;
    // A new step moves the keyboard focus: the input it removed may have had it, and without a
    // focused element inside the dialog Enter would not reach it.
    if wizard.read(cx).focused_step != Some(step) {
        let (name, focus) = {
            let wizard = wizard.read(cx);
            (wizard.name.clone(), wizard.focus.clone())
        };
        match step {
            Step::Options => name.update(cx, |state, cx| state.focus(window, cx)),
            _ => window.focus(&focus, cx),
        }
        wizard.update(cx, |wizard, _| wizard.focused_step = Some(step));
    }
    let palette = cx.palette();
    let body = match step {
        Step::Source => render_source(wizard, &palette, cx),
        Step::Options => render_options(wizard, &palette, cx),
        Step::Preview => render_preview(wizard, &palette, cx),
        Step::Done => render_done(wizard, &palette, cx),
    };
    let scrolls = matches!(step, Step::Source | Step::Options | Step::Done);
    let body_scroll = wizard.read(cx).body_scroll.clone();
    let content = div()
        .id("openapi-body")
        .size_full()
        .px(px(PADDING_X))
        .py(px(18.0))
        .child(body)
        .when(scrolls, |content| {
            content.overflow_y_scroll().track_scroll(&body_scroll)
        });
    let focus = wizard.read(cx).focus.clone();
    v_flex()
        .size_full()
        .track_focus(&focus)
        .text_size(px(13.0))
        .text_color(palette.fg)
        .child(render_header(step, &palette))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .child(content)
                .when(scrolls, |area| {
                    area.child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(Scrollbar::vertical(&body_scroll)),
                    )
                }),
        )
        .child(render_footer(wizard, &palette, cx))
        .into_any_element()
}

/// The title row and the stepper, over one bottom border.
fn render_header(step: Step, palette: &Palette) -> AnyElement {
    let labels: [SharedString; STEP_COUNT] = [
        t!("openapi.step.source").into(),
        t!("openapi.step.options").into(),
        t!("openapi.step.preview").into(),
    ];
    let current = match step {
        Step::Source => 1,
        Step::Options => 2,
        Step::Preview => 3,
        Step::Done => 4,
    };
    let stepper = h_flex()
        .gap(px(10.0))
        .px(px(PADDING_X))
        .pb(px(14.0))
        .children(labels.into_iter().enumerate().map(|(index, label)| {
            let number = index + 1;
            let done = current > number;
            let active = current == number;
            let last = number == STEP_COUNT;
            let dot = div()
                .size(px(STEP_DOT))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .map(|dot| {
                    if active {
                        dot.bg(palette.accent)
                            .text_color(palette.accent_fg)
                            .child(number.to_string())
                    } else if done {
                        dot.bg(palette.border).child(
                            Icon::new(IconName::Check)
                                .size(px(12.0))
                                .text_color(palette.accent_text),
                        )
                    } else {
                        dot.text_color(palette.fg_subtle).child(number.to_string())
                    }
                });
            h_flex()
                .gap(px(12.0))
                .when(!last, |row| row.flex_1())
                .child(
                    h_flex().flex_none().gap(px(8.0)).child(dot).child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if active || done {
                                palette.fg
                            } else {
                                palette.fg_subtle
                            })
                            .child(label),
                    ),
                )
                .when(!last, |row| {
                    row.child(div().flex_1().h(px(1.0)).bg(if done {
                        palette.accent
                    } else {
                        palette.border
                    }))
                })
        }));
    v_flex()
        .flex_none()
        .border_b_1()
        .border_color(palette.border)
        .child(
            h_flex()
                .gap(px(10.0))
                .px(px(PADDING_X))
                .pt(px(14.0))
                .pb(px(10.0))
                .child(
                    Icon::new(IconName::Download)
                        .size(px(16.0))
                        .text_color(palette.accent_text),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("import.title")),
                )
                .child(
                    IconButton::new("openapi-close", IconName::X)
                        .large()
                        .tooltip(t!("common.close"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
        )
        .child(stepper)
        .into_any_element()
}

/// The footer: a note at the left, Back and the primary button at the right.
fn render_footer(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let step = state.state.step;
    let selected = state.state.selected_count();
    let note: SharedString = match step {
        Step::Source => match state.state.format {
            ImportFormat::OpenApi => t!("openapi.supported").into(),
            ImportFormat::Postman => t!("import.postman.supported").into(),
            ImportFormat::Bruno => t!("import.bruno.supported").into(),
        },
        Step::Options if state.state.specs().len() > 1 => t!(
            &telaio_app::language::plural_key("openapi.from_versions", state.state.enabled_count()),
            operations = plural("openapi.operations", state.state.operation_count()),
            count = format_integer(state.state.enabled_count() as u64)
        )
        .into(),
        Step::Options if state.state.is_collection_tool() => {
            plural("import.postman.requests", state.state.operation_count()).into()
        }
        Step::Options => plural("openapi.operations", state.state.operation_count()).into(),
        Step::Preview if state.state.is_update() => {
            plural("openapi.selected", state.state.selected_change_count()).into()
        }
        Step::Preview => plural("openapi.selected", selected).into(),
        Step::Done => SharedString::default(),
    };
    let label: SharedString = match step {
        Step::Source if state.loading && state.advance_after_load => match state.state.source {
            SourceKind::Url => t!("openapi.url.fetching").into(),
            SourceKind::Git => t!("openapi.git.cloning").into(),
            _ => t!("openapi.file.reading").into(),
        },
        Step::Source | Step::Options => t!("openapi.next").into(),
        Step::Preview if state.state.is_update() => {
            plural("openapi.update.button", state.state.selected_change_count()).into()
        }
        Step::Preview => {
            let create = matches!(
                state.resolved(cx).map(|destination| destination.target),
                Some(Target::Create(_))
            );
            match (create, state.environments_only()) {
                (true, true) => t!("import.create_and_import_environments").into(),
                (false, true) => t!("import.environments_only").into(),
                (true, false) => plural("import.create_and_import", selected).into(),
                (false, false) => plural("openapi.import", selected).into(),
            }
        }
        Step::Done => match state.done.as_ref().and_then(|done| done.workspace.as_ref()) {
            Some(_) => t!("import.done.open_workspace").into(),
            None => t!("common.close").into(),
        },
    };
    let can_advance = state.can_advance(cx);
    let back = wizard.downgrade();
    let next = wizard.clone();
    h_flex()
        .flex_none()
        .gap(px(8.0))
        .px(px(PADDING_X))
        .py(px(12.0))
        .border_t_1()
        .border_color(palette.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.0))
                .text_color(palette.fg_subtle)
                .child(note),
        )
        .when(matches!(step, Step::Options | Step::Preview), |footer| {
            footer.child(
                GhostButton::new("openapi-back", t!("openapi.back")).on_click(move |_, _, cx| {
                    let _ = back.update(cx, |wizard, cx| {
                        wizard.state.step = match wizard.state.step {
                            Step::Preview => Step::Options,
                            _ => Step::Source,
                        };
                        cx.notify();
                    });
                }),
            )
        })
        .child(
            PrimaryButton::new("openapi-next", label)
                .disabled(!can_advance)
                .on_click(move |_, window, cx| {
                    if primary_action(&next, window, cx) {
                        window.close_dialog(cx);
                    }
                }),
        )
        .into_any_element()
}

/// Step 1: the source tabs, the input of the selected source, the error and what was found.
fn render_source(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let source = state.state.source;
    let tabs = sources_of(state.state.format).iter().map(|kind| {
        let (label, icon) = match kind {
            SourceKind::File => (t!("openapi.source.file"), IconName::FileUp),
            SourceKind::Url => (t!("openapi.source.url"), IconName::Link),
            SourceKind::Paste => (t!("openapi.source.paste"), IconName::Clipboard),
            SourceKind::Folder => (t!("openapi.source.folder"), IconName::Folder),
            SourceKind::Git => (t!("openapi.source.git"), IconName::GitBranch),
        };
        (*kind, label, icon)
    });
    let tab_row = h_flex()
        .gap(px(6.0))
        .flex_wrap()
        .children(tabs.map(|(kind, label, icon)| {
            let selected = kind == source;
            let weak = wizard.downgrade();
            let (rest, hover, pressed) = if selected {
                (
                    palette.raised,
                    palette.hover_over(palette.raised),
                    palette.pressed_over(palette.raised),
                )
            } else {
                (
                    gpui_kit::transparent_black(),
                    palette.hover,
                    palette.pressed,
                )
            };
            h_flex()
                .id(SharedString::from(format!("openapi-source-{kind:?}")))
                .h(px(SOURCE_TAB_HEIGHT))
                .gap(px(6.0))
                .px(px(12.0))
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(palette.border)
                .bg(rest)
                .when(selected, |tab| tab.shadow(vec![pill_shadow()]))
                .text_color(if selected {
                    palette.fg
                } else {
                    palette.fg_muted
                })
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(pressed))
                .child(Icon::new(icon).size(px(14.0)))
                .child(label)
                .on_click(move |_, window, cx| {
                    let _ = weak.update(cx, |wizard, cx| {
                        wizard.select_source(kind, cx);
                        // The input of the new source takes the keyboard, as the design's focus ring.
                        match kind {
                            SourceKind::Url => {
                                wizard.url.update(cx, |state, cx| state.focus(window, cx))
                            }
                            SourceKind::Paste => {
                                wizard.paste.update(cx, |state, cx| state.focus(window, cx));
                            }
                            SourceKind::Git => {
                                wizard
                                    .repository
                                    .update(cx, |state, cx| state.focus(window, cx));
                            }
                            SourceKind::File | SourceKind::Folder => {}
                        }
                    });
                })
        }));

    let input = match source {
        SourceKind::File => render_drop_zone(wizard, palette, cx),
        SourceKind::Url => render_url_input(wizard, palette, cx),
        SourceKind::Paste => v_flex()
            .gap(px(6.0))
            .child(
                div()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(px(12.0))
                    .child(
                        TextArea::new(&state.paste)
                            .h(px(PASTE_HEIGHT))
                            .disabled(state.loading),
                    ),
            )
            .child(hint(
                match state.state.format {
                    ImportFormat::OpenApi => t!("openapi.paste.hint"),
                    ImportFormat::Postman => t!("import.postman.paste_hint"),
                    ImportFormat::Bruno => t!("import.bruno.paste_hint"),
                },
                palette,
            ))
            .into_any_element(),
        SourceKind::Folder => render_folder_input(wizard, palette, cx),
        SourceKind::Git => render_git_input(wizard, palette, cx),
    };

    v_flex()
        .gap(px(18.0))
        .child(
            v_flex()
                .gap(px(14.0))
                .child(render_format_row(wizard, palette, cx))
                .child(
                    h_flex()
                        .gap(px(10.0))
                        .child(field_label(t!("import.source"), palette))
                        .child(tab_row),
                ),
        )
        // Splits the two choices (what, from where) from the input they lead to: the row of
        // pills below used to run straight into the drop zone with nothing marking where one
        // question ends and the other begins.
        .child(div().h(px(1.0)).bg(palette.border))
        .child(input)
        .children(
            state
                .error
                .clone()
                .map(|error| InlineMessage::new(InlineMessageKind::Danger, error)),
        )
        .when(!state.state.is_empty(), |column| {
            column.child(render_found(wizard, palette, cx))
        })
        .into_any_element()
}

/// The format chips of the first step, and whether the format was detected.
fn render_format_row(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let chip = |id: &'static str, format: ImportFormat, label: SharedString| {
        let weak = wizard.downgrade();
        choice_pill(id, label, state.state.format == format, palette).on_click(move |_, _, cx| {
            let _ = weak.update(cx, |wizard, cx| {
                if wizard.state.format != format {
                    wizard.state.set_format(format);
                    wizard.detected = false;
                    wizard.checkout = None;
                    wizard.error = None;
                    cx.notify();
                }
            });
        })
    };
    h_flex()
        .gap(px(10.0))
        .child(field_label(t!("import.format"), palette))
        .child(
            h_flex()
                .gap(px(6.0))
                .child(chip(
                    "import-format-openapi",
                    ImportFormat::OpenApi,
                    "OpenAPI / Swagger".into(),
                ))
                .child(chip(
                    "import-format-postman",
                    ImportFormat::Postman,
                    "Postman".into(),
                ))
                .child(chip(
                    "import-format-bruno",
                    ImportFormat::Bruno,
                    "Bruno".into(),
                )),
        )
        .child(div().flex_1())
        .when(state.detected, |row| {
            row.child(
                h_flex()
                    .gap(px(6.0))
                    .text_size(px(12.0))
                    .text_color(palette.success)
                    .child(Icon::new(IconName::WandSparkles).size(px(13.0)))
                    .child(t!("import.detected")),
            )
        })
        .into_any_element()
}

fn hint(text: impl Into<SharedString>, palette: &Palette) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(palette.fg_subtle)
        .child(text.into())
}

fn field_label(text: impl Into<SharedString>, palette: &Palette) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(palette.fg_muted)
        .child(text.into())
}

/// The dashed zone that takes a dropped file, or opens the file picker on a click.
fn render_drop_zone(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let loading = wizard.read(cx).loading;
    let format = wizard.read(cx).state.format;
    let pick = wizard.downgrade();
    let drop = wizard.downgrade();
    let (hover, pressed, accent_subtle) = (palette.hover, palette.pressed, palette.accent_subtle);
    let (border_strong, accent) = (palette.fg_subtle, palette.accent);
    v_flex()
        .id("openapi-drop-zone")
        .h(px(DROP_ZONE_HEIGHT))
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .rounded(px(10.0))
        .border_1()
        .border_dashed()
        .border_color(palette.border_strong)
        .text_color(palette.fg_muted)
        .when(!loading, |zone| {
            zone.cursor_pointer()
                .hover(move |style| style.bg(hover).border_color(border_strong))
                .active(move |style| style.bg(pressed))
                .drag_over::<ExternalPaths>(move |style, _, _, _| {
                    style.bg(accent_subtle).border_color(accent)
                })
                .on_drop(move |paths: &ExternalPaths, _, cx| {
                    let paths = paths.paths().to_vec();
                    if !paths.is_empty() {
                        let _ = drop.update(cx, |wizard, cx| {
                            wizard.load(SourceKind::File, LoadInput::Files(paths), false, cx);
                        });
                    }
                })
                .on_click(move |_, _, cx| pick_file(pick.clone(), cx))
        })
        .child(Icon::new(IconName::Upload).size(px(22.0)))
        .map(|zone| {
            if loading {
                zone.child(t!("openapi.file.reading"))
            } else {
                zone.child(
                    h_flex()
                        .gap(px(4.0))
                        .child(match format {
                            ImportFormat::OpenApi => t!("openapi.file.drop"),
                            ImportFormat::Postman => t!("import.postman.drop"),
                            ImportFormat::Bruno => t!("import.bruno.drop"),
                        })
                        .child(
                            div()
                                .text_color(palette.accent_text)
                                .child(t!("openapi.file.browse")),
                        ),
                )
                .child(hint(
                    match format {
                        ImportFormat::OpenApi => t!("openapi.file.types"),
                        ImportFormat::Postman => t!("import.postman.types"),
                        ImportFormat::Bruno => t!("import.bruno.types"),
                    },
                    palette,
                ))
            }
        })
        .into_any_element()
}

/// Opens the native file picker and reads the picked spec.
fn pick_file(wizard: WeakEntity<ImportWizard>, cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: true,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = receiver.await else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let _ = wizard.update(cx, |wizard, cx| {
            wizard.load(SourceKind::File, LoadInput::Files(paths), false, cx);
        });
    })
    .detach();
}

fn render_url_input(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let fetch = wizard.downgrade();
    let empty = state.url.read(cx).value().trim().is_empty();
    v_flex()
        .gap(px(6.0))
        .child(field_label(t!("openapi.url.label"), palette))
        .child(
            h_flex()
                .gap(px(8.0))
                .child(div().flex_1().min_w_0().child(text_field(&state.url, cx)))
                .child(
                    SecondaryButton::new(
                        "openapi-fetch",
                        if state.loading {
                            t!("openapi.url.fetching")
                        } else {
                            t!("openapi.url.fetch")
                        },
                    )
                    .disabled(state.loading || empty)
                    .on_click(move |_, _, cx| {
                        let _ = fetch.update(cx, |wizard, cx| {
                            let url = wizard.url.read(cx).value().to_string();
                            wizard.load(SourceKind::Url, LoadInput::Url(url), false, cx);
                        });
                    }),
                ),
        )
        .child(hint(t!("openapi.url.hint"), palette))
        .into_any_element()
}

/// What the source gave: its catalog files, then one card per spec read and per file that
/// could not be read.
fn render_found(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = &wizard.read(cx).state;
    let mono = cx.theme().mono_font_family.clone();
    let card = || {
        h_flex()
            .gap(px(10.0))
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(RADIUS_MD))
            .border_1()
            .border_color(palette.border)
    };
    let source_line = |text: String| {
        div()
            .truncate()
            .font_family(mono.clone())
            .text_size(px(11.0))
            .text_color(palette.fg_subtle)
            .child(text)
    };
    let catalogs = state.catalogs().iter().map(|catalog| {
        h_flex()
            .gap(px(10.0))
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(RADIUS_MD))
            .bg(palette.raised)
            .child(
                Icon::new(IconName::FileText)
                    .size(px(15.0))
                    .text_color(palette.fg_muted),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(mono.clone())
                    .text_size(px(12.0))
                    .child(catalog.source.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.0))
                    .text_color(palette.fg_muted)
                    .child(plural("openapi.catalog", catalog.apis)),
            )
    });
    let specs = state.specs().iter().map(|loaded| {
        let spec = &loaded.spec;
        let format = if spec.spec_version == "2.0" {
            "Swagger 2.0".to_string()
        } else {
            format!("OpenAPI {}", spec.spec_version)
        };
        let meta = format!(
            "{format} · {}",
            plural("openapi.operations", spec.operations.len())
        );
        let title = if spec.api_version.is_empty() {
            spec.title.clone()
        } else {
            format!("{} {}", spec.title, spec.api_version)
        };
        card()
            .child(
                Icon::new(IconName::CircleCheck)
                    .size(px(15.0))
                    .text_color(palette.success),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(div().truncate().child(title))
                    .when(!loaded.source.is_empty(), |column| {
                        column.child(source_line(loaded.source.clone()))
                    }),
            )
            .child(
                v_flex()
                    .flex_none()
                    .items_end()
                    .gap(px(2.0))
                    .text_size(px(12.0))
                    .text_color(palette.fg_muted)
                    .child(meta)
                    .when(!spec.warnings.is_empty(), |column| {
                        column.child(
                            div()
                                .text_color(palette.warning)
                                .child(plural("openapi.warnings", spec.warnings.len())),
                        )
                    }),
            )
    });
    let failures = state.failures().iter().map(|failure| {
        card()
            .child(
                Icon::new(IconName::CircleAlert)
                    .size(px(15.0))
                    .text_color(palette.danger),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(source_line(failure.source.clone()).text_color(palette.fg_muted))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(palette.danger)
                            .child(failure.error.clone()),
                    ),
            )
    });
    v_flex()
        .gap(px(8.0))
        .children(catalogs)
        .child(
            div()
                .pt(px(4.0))
                .child(SectionLabel::new(t!("openapi.found"))),
        )
        .children(specs)
        .children(render_postman_found(wizard, palette, cx))
        .children(render_bruno_found(wizard, palette, cx))
        .children(failures)
        .into_any_element()
}

/// The Postman exports the source gave: a collection is imported, an environment has a checkbox.
fn render_postman_found(
    wizard: &Entity<ImportWizard>,
    palette: &Palette,
    cx: &App,
) -> Vec<AnyElement> {
    let state = &wizard.read(cx).state;
    let mono = cx.theme().mono_font_family.clone();
    let files = state.postman_files();
    let has_environments = files
        .iter()
        .any(|file| file.format != PostmanFormat::Collection);
    let mut rows: Vec<AnyElement> = files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let meta = postman_meta(file);
            let weak = wizard.downgrade();
            let checked = state.is_postman_enabled(index);
            let row = h_flex()
                .id(("import-postman-file", index))
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(palette.border)
                .cursor_pointer()
                .hover(|style| style.bg(palette.hover))
                .active(|style| style.bg(palette.pressed))
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |wizard, cx| {
                        wizard.state.toggle_postman(index);
                        cx.notify();
                    });
                });
            let box_weak = wizard.downgrade();
            row.child(
                TriCheckbox::new(
                    ("import-postman-check", index),
                    if checked {
                        CheckState::Checked
                    } else {
                        CheckState::Unchecked
                    },
                )
                .on_click(move |_, _, cx| {
                    let _ = box_weak.update(cx, |wizard, cx| {
                        wizard.state.toggle_postman(index);
                        cx.notify();
                    });
                }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(div().truncate().child(file.name.clone()))
                    .when(!file.source.is_empty(), |column| {
                        column.child(
                            div()
                                .truncate()
                                .font_family(mono.clone())
                                .text_size(px(11.0))
                                .text_color(palette.fg_subtle)
                                .child(file.source.clone()),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.0))
                    .text_color(palette.fg_muted)
                    .child(meta),
            )
            .into_any_element()
        })
        .collect();
    if has_environments {
        rows.push(hint(t!("import.postman.env_note"), palette).into_any_element());
    }
    rows
}

/// The Bruno collection the source gave: the collection, then each environment with a checkbox
/// and its color (#222).
fn render_bruno_found(
    wizard: &Entity<ImportWizard>,
    palette: &Palette,
    cx: &App,
) -> Vec<AnyElement> {
    let state = &wizard.read(cx).state;
    let Some(bruno) = state.bruno() else {
        return Vec::new();
    };
    let mono = cx.theme().mono_font_family.clone();
    let source_line = |text: String| {
        div()
            .truncate()
            .font_family(mono.clone())
            .text_size(px(11.0))
            .text_color(palette.fg_subtle)
            .child(text)
    };
    let mut rows = vec![
        h_flex()
            .gap(px(10.0))
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(RADIUS_MD))
            .border_1()
            .border_color(palette.border)
            .child(
                Icon::new(IconName::CircleCheck)
                    .size(px(15.0))
                    .text_color(palette.success),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.0))
                    .child(div().truncate().child(bruno.name.clone()))
                    .when(!bruno.source.is_empty(), |column| {
                        column.child(source_line(bruno.source.clone()))
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.0))
                    .text_color(palette.fg_muted)
                    .child(
                        t!(
                            "import.bruno.collection_meta",
                            formats = bruno.formats.as_str(),
                            requests = plural("import.postman.requests", bruno.requests),
                            folders = plural("import.postman.folders", bruno.folders)
                        )
                        .into_owned(),
                    ),
            )
            .into_any_element(),
    ];
    for (index, environment) in bruno.environments.iter().enumerate() {
        let weak = wizard.downgrade();
        let box_weak = wizard.downgrade();
        let checked = state.is_bruno_environment_enabled(index);
        let toggle = move |weak: &WeakEntity<ImportWizard>, cx: &mut App| {
            let _ = weak.update(cx, |wizard, cx| {
                wizard.state.toggle_bruno_environment(index);
                cx.notify();
            });
        };
        rows.push(
            h_flex()
                .id(("import-bruno-env", index))
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(palette.border)
                .cursor_pointer()
                .hover(|style| style.bg(palette.hover))
                .active(|style| style.bg(palette.pressed))
                .on_click(move |_, _, cx| toggle(&weak, cx))
                .child(
                    TriCheckbox::new(
                        ("import-bruno-env-check", index),
                        if checked {
                            CheckState::Checked
                        } else {
                            CheckState::Unchecked
                        },
                    )
                    .on_click(move |_, _, cx| toggle(&box_weak, cx)),
                )
                .child(environment_dot(environment.color, palette))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.0))
                        .child(div().truncate().child(environment.name.clone()))
                        .child(source_line(format!("environments/{}", environment.name))),
                )
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.0))
                        .text_color(palette.fg_muted)
                        .child(bruno_environment_meta(environment)),
                )
                .into_any_element(),
        );
    }
    if bruno
        .environments
        .iter()
        .any(|environment| environment.secrets > 0)
    {
        rows.push(hint(t!("import.bruno.env_note"), palette).into_any_element());
    }
    rows
}

/// The dot of a Bruno environment: its color, or a hollow ring when it has none.
fn environment_dot(color: Option<EnvColor>, palette: &Palette) -> Div {
    let dot = div().flex_none().size(px(8.0)).rounded_full();
    match color {
        Some(color) => dot.bg(palette.env_color(color)),
        None => dot.border_1().border_color(palette.fg_subtle),
    }
}

/// The right side of a Bruno environment's row: its variables, and how many are secret.
fn bruno_environment_meta(environment: &crate::state::openapi_import::BrunoEnvironment) -> String {
    let variables = plural("import.postman.variables", environment.variables);
    if environment.secrets == 0 {
        t!("import.bruno.environment_variables", variables = variables).into_owned()
    } else {
        t!(
            "import.bruno.environment_meta",
            variables = variables,
            secrets = plural("import.bruno.secrets", environment.secrets)
        )
        .into_owned()
    }
}

/// The right side of a Postman export's row.
fn postman_meta(file: &crate::state::openapi_import::PostmanFile) -> String {
    let variables = plural("import.postman.variables", file.items);
    match file.format {
        PostmanFormat::Collection => t!(
            "import.postman.collection_meta",
            version = file.version.as_str(),
            requests = plural("import.postman.requests", file.items),
            folders = plural("import.postman.folders", file.others)
        )
        .into_owned(),
        PostmanFormat::Environment => {
            t!("import.postman.environment_meta", variables = variables).into_owned()
        }
        PostmanFormat::Globals => {
            t!("import.postman.globals_meta", variables = variables).into_owned()
        }
    }
}

/// The Folder source: the chosen folder and the button that opens the folder picker.
fn render_folder_input(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let pick = wizard.downgrade();
    let shown: SharedString = match &state.folder {
        Some(folder) => {
            crate::state::format::shorten_path(folder, dirs::home_dir().as_deref()).into()
        }
        None => t!("openapi.folder.none").into(),
    };
    v_flex()
        .gap(px(6.0))
        .child(
            h_flex()
                .gap(px(8.0))
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h(px(telaio_ui::components::text_field::FIELD_HEIGHT))
                        .gap(px(8.0))
                        .px(px(10.0))
                        .rounded(px(RADIUS_MD))
                        .bg(palette.surface)
                        .border_1()
                        .border_color(palette.border)
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(px(12.0))
                        .text_color(if state.folder.is_some() {
                            palette.fg
                        } else {
                            palette.fg_subtle
                        })
                        .child(
                            Icon::new(IconName::Folder)
                                .size(px(14.0))
                                .text_color(palette.fg_muted),
                        )
                        .child(div().truncate().child(shown)),
                )
                .child(
                    SecondaryButton::new(
                        "openapi-choose-folder",
                        if state.loading {
                            t!("openapi.folder.scanning")
                        } else {
                            t!("openapi.folder.choose")
                        },
                    )
                    .disabled(state.loading)
                    .on_click(move |_, _, cx| pick_folder(pick.clone(), cx)),
                ),
        )
        .child(hint(
            match state.state.format {
                ImportFormat::OpenApi => t!("openapi.folder.hint"),
                ImportFormat::Postman => t!("import.postman.folder_hint"),
                ImportFormat::Bruno => t!("import.bruno.folder_hint"),
            },
            palette,
        ))
        .into_any_element()
}

/// Opens the native folder picker and scans the picked folder.
fn pick_folder(wizard: WeakEntity<ImportWizard>, cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: None,
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(mut paths))) = receiver.await else {
            return;
        };
        let Some(path) = paths.pop() else {
            return;
        };
        let _ = wizard.update(cx, |wizard, cx| {
            wizard.folder = Some(path.clone());
            wizard.load(SourceKind::Folder, LoadInput::Folder(path), false, cx);
        });
    })
    .detach();
}

/// The Git source: the repository, the branch and the Clone button.
fn render_git_input(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let clone = wizard.downgrade();
    let empty = state.repository.read(cx).value().trim().is_empty();
    v_flex()
        .gap(px(6.0))
        .child(
            h_flex()
                .items_start()
                .gap(px(10.0))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(6.0))
                        .child(field_label(t!("openapi.git.repository"), palette))
                        .child(text_field(&state.repository, cx)),
                )
                .child(
                    v_flex()
                        .w(px(GIT_BRANCH_WIDTH))
                        .flex_none()
                        .gap(px(6.0))
                        .child(field_label(t!("openapi.git.branch"), palette))
                        .child(text_field(&state.branch, cx)),
                )
                .child(
                    v_flex()
                        .flex_none()
                        .gap(px(6.0))
                        // An empty label line, so the button lines up with the fields.
                        .child(field_label(" ", palette))
                        .child(
                            SecondaryButton::new(
                                "openapi-clone",
                                if state.loading {
                                    t!("openapi.git.cloning")
                                } else {
                                    t!("openapi.git.clone")
                                },
                            )
                            .disabled(state.loading || empty)
                            .on_click(move |_, _, cx| {
                                let _ = clone.update(cx, |wizard, cx| {
                                    let url = wizard.repository.read(cx).value().to_string();
                                    let branch = wizard.branch.read(cx).value().to_string();
                                    wizard.load(
                                        SourceKind::Git,
                                        LoadInput::Git { url, branch },
                                        false,
                                        cx,
                                    );
                                });
                            }),
                        ),
                ),
        )
        .child(hint(t!("openapi.git.hint"), palette))
        .into_any_element()
}

/// Step 2: the collection name and folders on the left, what to generate on the right.
fn render_options(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    if state.state.format == ImportFormat::Postman {
        return render_postman_options(wizard, palette, cx);
    }
    if state.state.format == ImportFormat::Bruno {
        return render_bruno_options(wizard, palette, cx);
    }
    let options = state.state.options;
    let mono = cx.theme().mono_font_family.clone();

    let groupings = [
        (
            Grouping::PathSegments,
            t!("openapi.options.group_path"),
            state
                .state
                .sample_path()
                .unwrap_or("/customers/{id}")
                .to_string(),
        ),
        (
            Grouping::Tags,
            t!("openapi.options.group_tags"),
            state.state.sample_tag().unwrap_or("Customers").to_string(),
        ),
    ];
    let folders = v_flex()
        .gap(px(4.0))
        .child(
            div()
                .pb(px(4.0))
                .child(SectionLabel::new(t!("openapi.options.folders"))),
        )
        .children(groupings.into_iter().map(|(grouping, label, sample)| {
            let weak = wizard.downgrade();
            h_flex()
                .gap(px(10.0))
                .py(px(6.0))
                .child(
                    telaio_ui::components::labelled_radio(
                        SharedString::from(format!("openapi-group-{grouping:?}")),
                        options.grouping == grouping,
                        label,
                        palette,
                    )
                    .on_change(move |_, _, cx| {
                        let _ = weak.update(cx, |wizard, cx| {
                            wizard.state.options.grouping = grouping;
                            cx.notify();
                        });
                    }),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(mono.clone())
                        .text_size(px(12.0))
                        .text_color(palette.fg_subtle)
                        .child(sample),
                )
        }));

    let labels = state.state.version_labels();
    let versions = (state.state.specs().len() > 1).then(|| {
        v_flex()
            .gap(px(4.0))
            .child(
                div()
                    .pb(px(4.0))
                    .child(SectionLabel::new(t!("openapi.options.versions"))),
            )
            .children(
                state
                    .state
                    .specs()
                    .iter()
                    .enumerate()
                    .map(|(index, loaded)| {
                        let weak = wizard.downgrade();
                        let file = loaded.source.rsplit('/').next().unwrap_or_default();
                        let label = if file.is_empty() {
                            labels[index].clone()
                        } else {
                            format!("{} · {file}", labels[index])
                        };
                        let format = if loaded.spec.spec_version == "2.0" {
                            "Swagger 2.0".to_string()
                        } else {
                            loaded.spec.spec_version.clone()
                        };
                        h_flex()
                            .gap(px(10.0))
                            .py(px(6.0))
                            .child(
                                div().min_w_0().child(
                                    telaio_ui::components::labelled_checkbox(
                                        SharedString::from(format!("openapi-version-{index}")),
                                        state.state.is_enabled(index),
                                        label,
                                        palette,
                                    )
                                    .on_change(
                                        move |_, _, cx| {
                                            let _ = weak.update(cx, |wizard, cx| {
                                                wizard.state.toggle_version(index);
                                                wizard.rebuild(cx);
                                            });
                                        },
                                    ),
                                ),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.0))
                                    .text_color(palette.fg_subtle)
                                    .child(format!(
                                        "{format} · {}",
                                        plural("openapi.operations", loaded.spec.operations.len())
                                    )),
                            )
                    }),
            )
    });

    let left = v_flex()
        .gap(px(16.0))
        .child(
            v_flex()
                .gap(px(6.0))
                .child(field_label(t!("openapi.options.name"), palette))
                .child(text_field(&state.name, cx)),
        )
        .children(versions)
        .child(folders);

    let base_urls = state.state.base_urls();
    let env_sub: SharedString = match base_urls.split_first() {
        Some((first, [])) => t!("openapi.gen.env_sub", url = first).into(),
        Some((first, rest)) => t!(
            &telaio_app::language::plural_key("openapi.gen.env_more", rest.len()),
            url = first,
            count = format_integer(rest.len() as u64)
        )
        .into(),
        None => t!("openapi.gen.env_sub", url = "http://localhost").into(),
    };
    let variables = state.state.auth_variables();
    let auth_sub: SharedString = if variables.is_empty() {
        t!("openapi.gen.auth_none").into()
    } else {
        let list: Vec<String> = variables
            .iter()
            .map(|name| format!("{{{{{name}}}}}"))
            .collect();
        t!("openapi.gen.auth_sub", variables = list.join(", ")).into()
    };
    type Toggle = fn(&mut brasa_format::openapi::ImportOptions) -> &mut bool;
    let generate: [(&str, SharedString, SharedString, bool, Toggle); 5] = [
        (
            "env",
            t!("openapi.gen.env").into(),
            env_sub,
            options.environments,
            |o| &mut o.environments,
        ),
        (
            "auth",
            t!("openapi.gen.auth").into(),
            auth_sub,
            options.auth,
            |o| &mut o.auth,
        ),
        (
            "bodies",
            t!("openapi.gen.bodies").into(),
            t!("openapi.gen.bodies_sub").into(),
            options.bodies,
            |o| &mut o.bodies,
        ),
        (
            "tests",
            t!("openapi.gen.tests").into(),
            t!("openapi.gen.tests_sub").into(),
            options.tests,
            |o| &mut o.tests,
        ),
        (
            "docs",
            t!("openapi.gen.docs").into(),
            t!("openapi.gen.docs_sub").into(),
            options.docs,
            |o| &mut o.docs,
        ),
    ];
    let right = v_flex()
        .gap(px(4.0))
        .child(
            div()
                .pb(px(4.0))
                .child(SectionLabel::new(t!("openapi.options.generate"))),
        )
        .children(
            generate
                .into_iter()
                .map(|(id, label, sub, checked, field)| {
                    let toggle = {
                        let weak = wizard.downgrade();
                        move |cx: &mut App| {
                            let _ = weak.update(cx, |wizard, cx| {
                                let flag = field(&mut wizard.state.options);
                                *flag = !*flag;
                                cx.notify();
                            });
                        }
                    };
                    described_checkbox(id, checked, label, sub, toggle, palette)
                }),
        );

    let collection = if state.environments_only()
        || !state.state.has_collection() && state.state.is_collection_tool()
    {
        String::new()
    } else {
        state.collection_name(cx)
    };
    v_flex()
        .gap(px(18.0))
        .children(
            state
                .destination
                .as_ref()
                .map(|block| render_destination(block, &collection, palette, cx)),
        )
        .child(
            h_flex()
                .items_start()
                .gap(px(20.0))
                .child(div().flex_1().min_w_0().child(left))
                .child(div().flex_1().min_w_0().child(right)),
        )
        .into_any_element()
}

/// Step 2 of a Postman import (design 4b): the destination, the name and environments on the left,
/// what to convert on the right.
fn render_postman_options(
    wizard: &Entity<ImportWizard>,
    palette: &Palette,
    cx: &App,
) -> AnyElement {
    let state = wizard.read(cx);
    let options = state.state.postman_options;
    let environments: Vec<(usize, String, String)> = state
        .state
        .postman_files()
        .iter()
        .enumerate()
        .filter(|(_, file)| file.format != PostmanFormat::Collection)
        .map(|(index, file)| (index, file.name.clone(), postman_meta(file)))
        .collect();
    let left = v_flex()
        .gap(px(16.0))
        .when(state.state.has_collection(), |column| {
            column.child(
                v_flex()
                    .gap(px(6.0))
                    .child(field_label(t!("openapi.options.name"), palette))
                    .child(text_field(&state.name, cx)),
            )
        })
        .when(!environments.is_empty(), |column| {
            column.child(
                v_flex()
                    .gap(px(4.0))
                    .child(
                        div()
                            .pb(px(4.0))
                            .child(SectionLabel::new(t!("import.postman.environments"))),
                    )
                    .children(environments.into_iter().map(|(index, name, meta)| {
                        let weak = wizard.downgrade();
                        h_flex()
                            .gap(px(10.0))
                            .py(px(5.0))
                            .child(
                                div().min_w_0().child(
                                    telaio_ui::components::labelled_checkbox(
                                        SharedString::from(format!("import-postman-env-{index}")),
                                        state.state.is_postman_enabled(index),
                                        name,
                                        palette,
                                    )
                                    .on_change(
                                        move |_, _, cx| {
                                            let _ = weak.update(cx, |wizard, cx| {
                                                wizard.state.toggle_postman(index);
                                                wizard.rebuild(cx);
                                            });
                                        },
                                    ),
                                ),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.0))
                                    .text_color(palette.fg_subtle)
                                    .child(meta),
                            )
                    })),
            )
        });
    type Toggle = fn(&mut brasa_format::postman::PostmanOptions) -> &mut bool;
    let convert: [(&str, SharedString, SharedString, bool, Toggle); 6] = [
        (
            "folders",
            t!("import.postman.opt_folders").into(),
            t!("import.postman.opt_folders_sub").into(),
            options.folders,
            |o| &mut o.folders,
        ),
        (
            "variables",
            t!("import.postman.opt_variables").into(),
            t!("import.postman.opt_variables_sub").into(),
            options.variables,
            |o| &mut o.variables,
        ),
        (
            "secrets",
            t!("import.postman.opt_secrets").into(),
            t!("import.postman.opt_secrets_sub").into(),
            options.secrets,
            |o| &mut o.secrets,
        ),
        (
            "auth",
            t!("import.postman.opt_auth").into(),
            t!("import.postman.opt_auth_sub").into(),
            options.auth,
            |o| &mut o.auth,
        ),
        (
            "scripts",
            t!("import.postman.opt_scripts").into(),
            t!("import.postman.opt_scripts_sub").into(),
            options.scripts,
            |o| &mut o.scripts,
        ),
        (
            "docs",
            t!("import.postman.opt_docs").into(),
            t!("import.postman.opt_docs_sub").into(),
            options.docs,
            |o| &mut o.docs,
        ),
    ];
    let right = v_flex()
        .gap(px(4.0))
        .child(
            div()
                .pb(px(4.0))
                .child(SectionLabel::new(t!("import.postman.convert"))),
        )
        .children(convert.into_iter().map(|(id, label, sub, checked, field)| {
            let toggle = {
                let weak = wizard.downgrade();
                move |cx: &mut App| {
                    let _ = weak.update(cx, |wizard, cx| {
                        let flag = field(&mut wizard.state.postman_options);
                        *flag = !*flag;
                        cx.notify();
                    });
                }
            };
            described_checkbox(id, checked, label, sub, toggle, palette)
        }));
    let collection = if state.environments_only()
        || !state.state.has_collection() && state.state.is_collection_tool()
    {
        String::new()
    } else {
        state.collection_name(cx)
    };
    v_flex()
        .gap(px(18.0))
        .children(
            state
                .destination
                .as_ref()
                .map(|block| render_destination(block, &collection, palette, cx)),
        )
        .child(
            h_flex()
                .items_start()
                .gap(px(20.0))
                .child(div().flex_1().min_w_0().child(left))
                .child(div().flex_1().min_w_0().child(right)),
        )
        .into_any_element()
}

/// Step 2 of a Bruno import: the collection name and the environments on the left, what to
/// convert on the right, as for Postman (#222).
fn render_bruno_options(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let options = state.state.bruno_options;
    let environments: Vec<(usize, String, Option<EnvColor>, String)> = state
        .state
        .bruno()
        .map(|bruno| {
            bruno
                .environments
                .iter()
                .enumerate()
                .map(|(index, environment)| {
                    (
                        index,
                        environment.name.clone(),
                        environment.color,
                        bruno_environment_meta(environment),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let left = v_flex()
        .gap(px(16.0))
        .when(state.state.has_collection(), |column| {
            column.child(
                v_flex()
                    .gap(px(6.0))
                    .child(field_label(t!("openapi.options.name"), palette))
                    .child(text_field(&state.name, cx)),
            )
        })
        .when(!environments.is_empty(), |column| {
            column.child(
                v_flex()
                    .gap(px(4.0))
                    .child(
                        div()
                            .pb(px(4.0))
                            .child(SectionLabel::new(t!("import.postman.environments"))),
                    )
                    .children(environments.into_iter().map(|(index, name, color, meta)| {
                        let weak = wizard.downgrade();
                        h_flex()
                            .gap(px(10.0))
                            .py(px(5.0))
                            .child(
                                div().min_w_0().child(
                                    telaio_ui::components::labelled_checkbox(
                                        SharedString::from(format!(
                                            "import-bruno-option-env-{index}"
                                        )),
                                        state.state.is_bruno_environment_enabled(index),
                                        name,
                                        palette,
                                    )
                                    .on_change(
                                        move |_, _, cx| {
                                            let _ = weak.update(cx, |wizard, cx| {
                                                wizard.state.toggle_bruno_environment(index);
                                                wizard.rebuild(cx);
                                            });
                                        },
                                    ),
                                ),
                            )
                            .child(environment_dot(color, palette))
                            .child(div().flex_1())
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(px(12.0))
                                    .text_color(palette.fg_subtle)
                                    .child(meta),
                            )
                    })),
            )
        });
    type Toggle = fn(&mut crate::state::openapi_import::Wizard) -> &mut bool;
    let convert: [(&str, SharedString, SharedString, bool, Toggle); 7] = [
        (
            "bruno-folders",
            t!("import.postman.opt_folders").into(),
            t!("import.postman.opt_folders_sub").into(),
            options.folders,
            |w| &mut w.bruno_options.folders,
        ),
        (
            "bruno-variables",
            t!("import.bruno.opt_variables").into(),
            t!("import.bruno.opt_variables_sub").into(),
            options.variables,
            |w| &mut w.bruno_options.variables,
        ),
        (
            "bruno-secrets",
            t!("import.postman.opt_secrets").into(),
            t!("import.bruno.opt_secrets_sub").into(),
            options.secrets,
            |w| &mut w.bruno_options.secrets,
        ),
        (
            "bruno-auth",
            t!("import.postman.opt_auth").into(),
            t!("import.postman.opt_auth_sub").into(),
            options.auth,
            |w| &mut w.bruno_options.auth,
        ),
        (
            "bruno-scripts",
            t!("import.bruno.opt_scripts").into(),
            t!("import.bruno.opt_scripts_sub").into(),
            options.scripts,
            |w| &mut w.bruno_options.scripts,
        ),
        (
            "bruno-docs",
            t!("import.bruno.opt_docs").into(),
            t!("import.bruno.opt_docs_sub").into(),
            options.docs,
            |w| &mut w.bruno_options.docs,
        ),
        (
            "bruno-colors",
            t!("import.bruno.opt_colors").into(),
            t!("import.bruno.opt_colors_sub").into(),
            state.state.bruno_colors,
            |w| &mut w.bruno_colors,
        ),
    ];
    let right = v_flex()
        .gap(px(4.0))
        .child(
            div()
                .pb(px(4.0))
                .child(SectionLabel::new(t!("import.postman.convert"))),
        )
        .children(convert.into_iter().map(|(id, label, sub, checked, field)| {
            let toggle = {
                let weak = wizard.downgrade();
                move |cx: &mut App| {
                    let _ = weak.update(cx, |wizard, cx| {
                        let flag = field(&mut wizard.state);
                        *flag = !*flag;
                        cx.notify();
                    });
                }
            };
            described_checkbox(id, checked, label, sub, toggle, palette)
        }));
    let collection = if state.environments_only() || !state.state.has_collection() {
        String::new()
    } else {
        state.collection_name(cx)
    };
    v_flex()
        .gap(px(18.0))
        .children(
            state
                .destination
                .as_ref()
                .map(|block| render_destination(block, &collection, palette, cx)),
        )
        .child(
            h_flex()
                .items_start()
                .gap(px(20.0))
                .child(div().flex_1().min_w_0().child(left))
                .child(div().flex_1().min_w_0().child(right)),
        )
        .into_any_element()
}

/// A checkbox with a label and a description under it. Like telaio's `labelled_checkbox`, the
/// halo lights up under the pointer anywhere on the row, and a click on the text toggles it.
fn described_checkbox(
    id: &str,
    checked: bool,
    label: SharedString,
    description: SharedString,
    toggle: impl Fn(&mut App) + Clone + 'static,
    palette: &Palette,
) -> AnyElement {
    let group = SharedString::from(format!("openapi-gen-{id}-group"));
    let (hover, pressed) = (palette.pressed, palette.border_strong);
    let box_toggle = toggle.clone();
    h_flex()
        .group(group.clone())
        .items_start()
        .gap(px(10.0))
        .py(px(6.0))
        .child(
            div().pt(px(1.0)).child(
                Checkbox::new(SharedString::from(format!("openapi-gen-{id}")))
                    .checked(checked)
                    .p(px(CHECKBOX_HALO_PADDING))
                    .m(px(-CHECKBOX_HALO_PADDING))
                    .rounded(px(RADIUS_SM))
                    .group_hover(group.clone(), move |style| style.bg(hover))
                    .group_active(group.clone(), move |style| style.bg(pressed))
                    .on_click(move |_, _, cx| box_toggle(cx)),
            ),
        )
        .child(
            v_flex()
                .id(SharedString::from(format!("openapi-gen-{id}-label")))
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .cursor_pointer()
                .child(label)
                .child(
                    div()
                        .text_size(px(12.0))
                        .line_height(relative(1.4))
                        .text_color(palette.fg_subtle)
                        .child(description),
                )
                .on_click(move |_, _, cx| toggle(cx)),
        )
        .into_any_element()
}

/// Step 3: the counts, the tree of requests to pick from and what else is written.
fn render_preview(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let Some(existing) = state.state.existing() else {
        return render_tree_preview(wizard, palette, cx);
    };
    let update = state.state.is_update();
    let choice = |id: &'static str, label: SharedString, selected: bool, value: bool| {
        let weak = wizard.downgrade();
        choice_pill(id, label, selected, palette).on_click(move |_, _, cx| {
            let _ = weak.update(cx, |wizard, cx| {
                wizard.state.set_update(value);
                cx.notify();
            });
        })
    };
    v_flex()
        .size_full()
        .gap(px(12.0))
        .children(
            state
                .error
                .clone()
                .map(|error| InlineMessage::new(InlineMessageKind::Danger, error)),
        )
        .child(
            h_flex()
                .flex_none()
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(RADIUS_MD))
                .bg(palette.raised)
                .child(
                    Icon::new(IconName::Info)
                        .size(px(15.0))
                        .text_color(palette.accent_text),
                )
                .child(div().flex_1().min_w_0().child(t!(
                    "openapi.update.imported",
                    name = existing.folder.clone(),
                    date = existing.manifest.imported.clone()
                ))),
        )
        .child(
            h_flex()
                .flex_none()
                .gap(px(6.0))
                .child(choice(
                    "openapi-update-existing",
                    t!("openapi.update.existing").into(),
                    update,
                    true,
                ))
                .child(choice(
                    "openapi-update-new",
                    t!("openapi.update.new").into(),
                    !update,
                    false,
                )),
        )
        .child(div().flex_1().min_h_0().w_full().map(|area| {
            if update {
                area.child(render_changes(wizard, palette, cx))
            } else {
                area.child(render_tree_preview(wizard, palette, cx))
            }
        }))
        .into_any_element()
}

/// A pill button of a two way choice, styled like the source buttons of the first step.
fn choice_pill(
    id: &'static str,
    label: SharedString,
    selected: bool,
    palette: &Palette,
) -> Stateful<Div> {
    let (rest, hover, pressed) = if selected {
        (
            palette.raised,
            palette.hover_over(palette.raised),
            palette.pressed_over(palette.raised),
        )
    } else {
        (
            gpui_kit::transparent_black(),
            palette.hover,
            palette.pressed,
        )
    };
    h_flex()
        .id(id)
        .h(px(SOURCE_TAB_HEIGHT - 2.0))
        .px(px(12.0))
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(palette.border)
        .bg(rest)
        // Selected reads from its fill and a small shadow, like every other picker in the app
        // (`SegmentedControl`, the document tabs): a colored border alone, the one place this
        // used, read weaker than those and inconsistent with them.
        .when(selected, |pill| {
            pill.shadow(vec![pill_shadow()])
        })
        .text_color(if selected {
            palette.fg
        } else {
            palette.fg_muted
        })
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .active(move |style| style.bg(pressed))
        .child(label)
}

/// The selected pill's shadow: opaque black at 12% alpha, the same fixed value
/// `SegmentedControl` uses elsewhere, not a palette token (see that component's doc comment).
fn pill_shadow() -> BoxShadow {
    BoxShadow::new(px(0.0), px(1.0), SHADOW_COLOR).blur_radius(px(2.0))
}

const SHADOW_COLOR: Hsla = Hsla {
    h: 0.0,
    s: 0.0,
    l: 0.0,
    a: 0.12,
};

/// The changes since the existing collection was imported, each with its checkbox (design 1e).
fn render_changes(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let (added, changed, removed, unchanged) = state.state.change_counts();
    let changes = state.state.changes();
    let count = changes.len();
    let note = div()
        .flex_none()
        .text_size(px(12.0))
        .line_height(relative(1.5))
        .text_color(palette.fg_subtle)
        .child(t!("openapi.update.note"));
    let summary = div()
        .flex_none()
        .text_size(px(12.0))
        .text_color(palette.fg_muted)
        .child(t!(
            "openapi.update.summary",
            added = format_integer(added as u64),
            changed = format_integer(changed as u64),
            removed = format_integer(removed as u64),
            unchanged = format_integer(unchanged as u64)
        ));
    if count == 0 {
        return v_flex()
            .size_full()
            .gap(px(12.0))
            .child(summary)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(palette.fg_muted)
                    .child(t!("openapi.update.up_to_date")),
            )
            .into_any_element();
    }
    let selected: Vec<bool> = changes
        .iter()
        .map(|change| state.state.is_change_selected(&change.key))
        .collect();
    let weak = wizard.downgrade();
    let row_palette = palette.clone();
    let mono = cx.theme().mono_font_family.clone();
    let list = uniform_list("openapi-changes", count, move |range, _, _| {
        range
            .map(|index| {
                render_change(
                    index,
                    &changes[index],
                    selected[index],
                    &weak,
                    &row_palette,
                    &mono,
                )
            })
            .collect::<Vec<_>>()
    })
    .track_scroll(&state.rows_scroll)
    .size_full();
    let rows_scroll = state.rows_scroll.clone();
    v_flex()
        .size_full()
        .gap(px(12.0))
        .child(summary)
        .child(
            div()
                .flex_1()
                .min_h_0()
                .relative()
                .py(px(6.0))
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(palette.border)
                .bg(palette.bg)
                .child(list)
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .child(Scrollbar::vertical(&rows_scroll)),
                ),
        )
        .child(note)
        .into_any_element()
}

/// One change: its checkbox, what happened, the method, the path and what changed. The whole
/// row toggles it.
fn render_change(
    index: usize,
    change: &PlanChange,
    selected: bool,
    wizard: &WeakEntity<ImportWizard>,
    palette: &Palette,
    mono: &SharedString,
) -> AnyElement {
    let (label, color) = match change.kind {
        ChangeKind::Added => (t!("openapi.update.added"), palette.success),
        ChangeKind::Changed => (t!("openapi.update.changed"), palette.warning),
        _ => (t!("openapi.update.removed"), palette.danger),
    };
    let detail = if change.kind == ChangeKind::Removed && !selected {
        t!("openapi.update.kept").into_owned()
    } else {
        change.details.join(", ")
    };
    let toggle = |weak: WeakEntity<ImportWizard>, key: String| {
        move |cx: &mut App| {
            let _ = weak.update(cx, |wizard, cx| {
                wizard.state.toggle_change(&key);
                cx.notify();
            });
        }
    };
    let box_toggle = toggle(wizard.clone(), change.key.clone());
    let row_toggle = toggle(wizard.clone(), change.key.clone());
    let state = if selected {
        CheckState::Checked
    } else {
        CheckState::Unchecked
    };
    let method = change
        .method
        .parse::<brasa_core::Method>()
        .unwrap_or_default();
    h_flex()
        .id(("openapi-change", index))
        .w_full()
        .h(px(ROW_HEIGHT))
        .gap(px(10.0))
        .px(px(12.0))
        .text_size(px(12.0))
        .cursor_pointer()
        .hover(|style| style.bg(palette.hover))
        .active(|style| style.bg(palette.pressed))
        .on_click(move |_, _, cx| row_toggle(cx))
        .child(
            TriCheckbox::new(("openapi-change-check", index), state)
                .on_click(move |_, _, cx| box_toggle(cx)),
        )
        .child(
            div()
                .flex_none()
                .w(px(CHANGE_KIND_WIDTH))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(label),
        )
        .child(div().flex_none().child(MethodBadge::label(method)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(mono.clone())
                .child(change.path.clone()),
        )
        .child(
            div()
                .flex_none()
                .max_w(px(CHANGE_DETAIL_WIDTH))
                .truncate()
                .text_color(palette.fg_subtle)
                .child(detail),
        )
        .into_any_element()
}

/// The new collection's tree of requests, with the counts and what else is written.
fn render_tree_preview(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let state = wizard.read(cx);
    let environments_only = state.environments_only();
    let total = state.state.total_count();
    let selected = state.state.selected_count();
    let summary = format!(
        "{} · {}",
        t!(
            &telaio_app::language::plural_key("openapi.preview.requests", total),
            selected = format_integer(selected as u64),
            count = format_integer(total as u64)
        ),
        plural("openapi.preview.folders", state.state.folder_count())
    );
    let link = |id: &'static str, label: SharedString, all: bool| {
        let weak = wizard.downgrade();
        div()
            .id(id)
            .text_size(px(12.0))
            .text_color(palette.accent_text)
            .cursor_pointer()
            .hover(|style| style.underline())
            .active(|style| style.opacity(0.7))
            .child(label)
            .on_click(move |_, _, cx| {
                let _ = weak.update(cx, |wizard, cx| {
                    wizard.state.select_all(all);
                    cx.notify();
                });
            })
    };

    let rows = state.state.rows();
    let count = rows.len();
    let weak = wizard.downgrade();
    let row_palette = palette.clone();
    let mono = cx.theme().mono_font_family.clone();
    let list = uniform_list("openapi-preview-rows", count, move |range, _, _| {
        range
            .map(|index| {
                // A row spans the list, so its hover and its count at the end reach the edge.
                div()
                    .w_full()
                    .child(render_row(index, &rows[index], &weak, &row_palette, &mono))
            })
            .collect::<Vec<_>>()
    })
    .track_scroll(&state.rows_scroll)
    .size_full();
    let rows_scroll = state.rows_scroll.clone();

    let plan = state.state.plan();
    let environments: Vec<String> = plan
        .map(|plan| {
            plan.environments
                .iter()
                .filter(|environment| !environment.base.is_empty())
                .map(|environment| environment.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let secrets: Vec<String> = plan
        .and_then(|plan| plan.environments.first())
        .map(|environment| {
            environment
                .secret
                .iter()
                .map(|entry| entry.key.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut env_text = if environments.is_empty() {
        t!("openapi.preview.no_environment").into_owned()
    } else {
        t!(
            &telaio_app::language::plural_key("openapi.preview.environments", environments.len()),
            count = format_integer(environments.len() as u64),
            names = environments.join(", ")
        )
        .into_owned()
    };
    if !secrets.is_empty() {
        env_text.push_str(" · ");
        env_text.push_str(&plural("openapi.preview.auth", secrets.len()));
    }

    v_flex()
        .size_full()
        .gap(px(12.0))
        .children(
            state
                .error
                .clone()
                .map(|error| InlineMessage::new(InlineMessageKind::Danger, error)),
        )
        .when(!environments_only, |column| {
            column.child(
                h_flex()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(palette.fg_muted)
                            .child(summary),
                    )
                    .child(link(
                        "openapi-select-all",
                        t!("openapi.preview.select_all").into(),
                        true,
                    ))
                    .child(link(
                        "openapi-select-none",
                        t!("openapi.preview.select_none").into(),
                        false,
                    )),
            )
        })
        .when(
            state.state.is_collection_tool() && !environments_only,
            |column| {
                let (complete, warning, unsupported) = state.state.compat_counts();
                let chip = |icon: IconName, bg: Hsla, fg: Hsla, text: String| {
                    h_flex()
                        .flex_none()
                        .gap(px(6.0))
                        .h(px(24.0))
                        .px(px(10.0))
                        .rounded(px(12.0))
                        .bg(bg)
                        .text_color(fg)
                        .text_size(px(12.0))
                        .child(Icon::new(icon).size(px(12.0)))
                        .child(text)
                };
                let count = |value: usize| format_integer(value as u64);
                column.child(
                    h_flex()
                        .flex_none()
                        .gap(px(8.0))
                        .child(chip(
                            IconName::CircleCheck,
                            palette.success_subtle,
                            palette.success,
                            t!("import.compat.ok", count = count(complete)).into_owned(),
                        ))
                        .child(chip(
                            IconName::TriangleAlert,
                            palette.warning_subtle,
                            palette.warning,
                            t!("import.compat.warning", count = count(warning)).into_owned(),
                        ))
                        .child(chip(
                            IconName::CircleX,
                            palette.danger_subtle,
                            palette.danger,
                            t!("import.compat.unsupported", count = count(unsupported))
                                .into_owned(),
                        )),
                )
            },
        )
        .when(!environments_only, |column| {
            column.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .py(px(6.0))
                    .px(px(4.0))
                    .rounded(px(RADIUS_MD))
                    .border_1()
                    .border_color(palette.border)
                    .bg(palette.bg)
                    .child(list)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(Scrollbar::vertical(&rows_scroll)),
                    ),
            )
        })
        .child(
            h_flex()
                .flex_none()
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(10.0))
                .rounded(px(RADIUS_MD))
                .bg(palette.raised)
                .child(
                    Icon::new(IconName::Layers)
                        .size(px(15.0))
                        .text_color(palette.fg_muted),
                )
                .child(div().flex_1().min_w_0().truncate().child(env_text))
                .when(!secrets.is_empty(), |card| {
                    card.child(
                        div()
                            .flex_none()
                            .max_w(px(260.0))
                            .truncate()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(12.0))
                            .text_color(palette.fg_subtle)
                            .child(format!("{} → .local.env", secrets.join(", "))),
                    )
                }),
        )
        .children(render_destination_summary(wizard, palette, cx))
        .into_any_element()
}

/// The card of the Preview step that says where the collection goes.
fn render_destination_summary(
    wizard: &Entity<ImportWizard>,
    palette: &Palette,
    cx: &App,
) -> Option<AnyElement> {
    let state = wizard.read(cx);
    let destination = state.resolved(cx)?;
    let collection = state.collection_name(cx);
    // Environments on their own add no folder, and go where the block said.
    if state.environments_only() && !matches!(destination.target, Target::Create(_)) {
        return None;
    }
    let (icon, text) = match &destination.target {
        Target::Create(path) => {
            let home = dirs::home_dir();
            let parent = path.parent().unwrap_or(path);
            (
                IconName::FolderPlus,
                t!(
                    "import.summary.create",
                    name = folder_name_of(path).as_str(),
                    path = crate::state::format::shorten_path(parent, home.as_deref()).as_str()
                )
                .into_owned(),
            )
        }
        Target::Open(path) => (
            IconName::FolderTree,
            t!(
                "import.summary.add",
                collection = collection.as_str(),
                workspace = folder_name_of(path).as_str()
            )
            .into_owned(),
        ),
        Target::Current => {
            let root = state.view.upgrade().and_then(|view| {
                view.read(cx)
                    .state
                    .workspace
                    .as_ref()
                    .map(|workspace| workspace.root().to_path_buf())
            })?;
            (
                IconName::FolderTree,
                t!(
                    "import.summary.add",
                    collection = collection.as_str(),
                    workspace = folder_name_of(&root).as_str()
                )
                .into_owned(),
            )
        }
    };
    Some(
        h_flex()
            .flex_none()
            .gap(px(10.0))
            .px(px(12.0))
            .py(px(10.0))
            .rounded(px(RADIUS_MD))
            .bg(palette.raised)
            .child(Icon::new(icon).size(px(15.0)).text_color(palette.fg_muted))
            .child(div().flex_1().min_w_0().truncate().child(text))
            .child(
                div()
                    .flex_none()
                    .text_size(px(12.0))
                    .text_color(palette.fg_subtle)
                    .child(t!("import.summary.nothing_written")),
            )
            .into_any_element(),
    )
}

/// The last component of `path`.
fn folder_name_of(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// One row of the preview tree. A folder's row opens and collapses it, a request's row toggles
/// it; the checkbox of a folder selects everything under it.
fn render_row(
    index: usize,
    row: &PreviewRow,
    wizard: &WeakEntity<ImportWizard>,
    palette: &Palette,
    mono: &SharedString,
) -> AnyElement {
    let id = row.id.clone();
    let tree_row = TreeRow::new(("openapi-row", index), row.name.clone()).depth(row.depth);
    match &row.kind {
        RowKind::Folder {
            open,
            check,
            selected,
            total,
        } => {
            let check_id = id.clone();
            let check_weak = wizard.clone();
            let checkbox = TriCheckbox::new(("openapi-row-check", index), check_state(*check))
                .on_click(move |_, _, cx| {
                    let _ = check_weak.update(cx, |wizard, cx| {
                        wizard.state.toggle_folder(&check_id);
                        cx.notify();
                    });
                });
            let weak = wizard.clone();
            tree_row
                .folder(*open)
                .leading(
                    h_flex().gap(px(6.0)).child(checkbox).child(
                        Icon::new(if *open {
                            IconName::FolderOpen
                        } else {
                            IconName::Folder
                        })
                        .size(px(14.0))
                        .text_color(palette.fg_muted),
                    ),
                )
                .suffix(
                    div()
                        .pr(px(8.0))
                        .font_family(mono.clone())
                        .text_size(px(11.0))
                        .text_color(palette.fg_subtle)
                        .child(format!(
                            "{}/{}",
                            format_integer(*selected as u64),
                            format_integer(*total as u64)
                        )),
                )
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |wizard, cx| {
                        wizard.state.toggle_open(&id);
                        cx.notify();
                    });
                })
                .into_any_element()
        }
        RowKind::Request {
            method,
            checked,
            path,
            issue,
        } => {
            let check_id = id.clone();
            let check_weak = wizard.clone();
            let state = if *checked {
                CheckState::Checked
            } else {
                CheckState::Unchecked
            };
            let checkbox =
                TriCheckbox::new(("openapi-row-check", index), state).on_click(move |_, _, cx| {
                    let _ = check_weak.update(cx, |wizard, cx| {
                        wizard.state.toggle_request(&check_id);
                        cx.notify();
                    });
                });
            let weak = wizard.clone();
            tree_row
                .leading(
                    h_flex()
                        .gap(px(6.0))
                        .child(checkbox)
                        .child(MethodBadge::label(method.clone())),
                )
                .when(!path.is_empty() || issue.is_some(), |row| {
                    let note = issue.as_ref().map(|issue| {
                        let color = match issue.level() {
                            IssueLevel::Note => palette.success,
                            IssueLevel::Warning => palette.warning,
                            IssueLevel::Unsupported => palette.danger,
                        };
                        div()
                            .flex_none()
                            .text_size(px(11.5))
                            .text_color(color)
                            .child(issue_text(issue))
                    });
                    row.suffix(
                        h_flex().gap(px(10.0)).pr(px(8.0)).children(note).child(
                            div()
                                .w(px(ROW_PATH_WIDTH))
                                .flex_none()
                                .truncate()
                                .text_right()
                                .font_family(mono.clone())
                                .text_size(px(11.0))
                                .text_color(palette.fg_subtle)
                                .child(path.clone()),
                        ),
                    )
                })
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |wizard, cx| {
                        wizard.state.toggle_request(&id);
                        cx.notify();
                    });
                })
                .into_any_element()
        }
    }
}

fn check_state(check: Check) -> CheckState {
    match check {
        Check::All => CheckState::Checked,
        Check::Some => CheckState::Mixed,
        Check::None => CheckState::Unchecked,
    }
}

/// The Done step: what was imported, and the warnings.
fn render_done(wizard: &Entity<ImportWizard>, palette: &Palette, cx: &App) -> AnyElement {
    let Some(done) = &wizard.read(cx).done else {
        return div().into_any_element();
    };
    v_flex()
        .min_h_full()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .child(
            Icon::new(IconName::CircleCheck)
                .size(px(32.0))
                .text_color(palette.success),
        )
        .child(
            div()
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_center()
                .child(match done.updated {
                    Some((added, changed, removed)) => t!(
                        "openapi.done.updated",
                        name = done.name.clone(),
                        added = format_integer(added as u64),
                        changed = format_integer(changed as u64),
                        removed = format_integer(removed as u64)
                    )
                    .into_owned(),
                    None if done.count == 0 => t!("import.done.environments").into_owned(),
                    None if done.created => plural("import.done.created", done.count),
                    None => t!(
                        &telaio_app::language::plural_key("openapi.done.title", done.count),
                        count = format_integer(done.count as u64),
                        name = done.name.clone()
                    )
                    .into_owned(),
                }),
        )
        .child(
            div()
                .text_color(palette.fg_muted)
                .text_center()
                .child(match &done.workspace {
                    Some(workspace) => t!(
                        "import.done.in_workspace",
                        collection = done.name.as_str(),
                        workspace = workspace.as_str()
                    ),
                    None if done.count == 0 => t!("import.done.environments_note"),
                    None => t!("openapi.done.note"),
                }),
        )
        .when_some(done.report.clone(), |column, report| {
            column.child(
                h_flex()
                    .mt(px(4.0))
                    .gap(px(10.0))
                    .px(px(14.0))
                    .py(px(10.0))
                    .rounded(px(RADIUS_MD))
                    .bg(palette.warning_subtle)
                    .text_color(palette.warning)
                    .child(Icon::new(IconName::TriangleAlert).size(px(15.0)))
                    .child(plural("import.report.review", done.review))
                    .child(
                        div()
                            .id("import-view-report")
                            .cursor_pointer()
                            .text_color(palette.accent_text)
                            .hover(|style| style.underline())
                            .child(t!("import.report.view"))
                            .on_click(move |_, _, cx| cx.open_with_system(&report)),
                    ),
            )
        })
        // The report lists them, with the requests they belong to.
        .when(
            !done.warnings.is_empty() && done.report.is_none(),
            |column| {
                column.child(
                    v_flex()
                        .w_full()
                        .mt(px(8.0))
                        .gap(px(4.0))
                        .px(px(12.0))
                        .py(px(10.0))
                        .rounded(px(RADIUS_MD))
                        .bg(palette.warning_subtle)
                        .text_size(px(12.0))
                        .child(
                            div()
                                .text_color(palette.warning)
                                .child(t!("openapi.done.warnings")),
                        )
                        .children(done.warnings.iter().map(|warning| {
                            div()
                                .text_color(palette.fg_muted)
                                .child(format!("- {warning}"))
                        })),
                )
            },
        )
        .into_any_element()
}
