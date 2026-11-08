//! [`AppView`]: the single top-level `gpui` view. It owns the plain
//! [`AppState`](crate::state::AppState), the `gpui`-specific bits that have to live alongside it
//! (the sidebar's tree state), and lays out the title bar, sidebar and main area. Other `views/`
//! modules add methods to [`AppView`] (`impl AppView` blocks split across files) so each panel's
//! rendering code lives next to what it renders, while the overall layout stays here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::tree::TreeState;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_core::{Body, Method};
use brasa_format::snippet::SnippetLanguage;
use brasa_runner::{RunResult, ScriptEngine, SendOptions};
use brasa_script::QuickJsEngine;
use brasa_workspace::{Workspace, WorkspaceError};

use crate::actions::{
    CloseActiveTab, DeleteSelectedEntry, ImportOpenApi, MoveEnvironmentDown, MoveEnvironmentUp,
    NextTab, OpenCommandPalette, OpenSettings, OpenShortcuts, OpenWorkspace, PreviousTab,
    RenameSelectedEntry, SaveActiveTab, SelectEnvironment1, SelectEnvironment2, SelectEnvironment3,
    SelectEnvironment4, SelectEnvironment5, SelectEnvironment6, SelectEnvironment7,
    SelectEnvironment8, SelectEnvironment9, SelectNoEnvironment, SendActiveTab, ToggleCollections,
    ToggleEnvironments, ToggleLoadTests, ToggleSourceControl,
};
use crate::state::about::AboutInfo;
use crate::state::debug_open::{self, DebugOpenTarget};
use crate::state::script_completion::JsonShape;
use crate::state::settings::{Settings, SettingsCategory};
use crate::state::ui_tabs::{self, RequestTab, ResponseTab};
use crate::state::workspace_log::{log_workspace_error, tree_counts};
use crate::state::{self, AppState};
use crate::theme::PaletteExt;
use crate::theme::metrics::{
    ICON_BUTTON_LG, RADIUS_MD, SIDEBAR_WIDTH, SIDEBAR_WIDTH_MAX, SIDEBAR_WIDTH_MIN,
    STATUS_BAR_HEIGHT, RADIUS_SM,
};
use crate::views::components::{
    DocumentTab, DocumentTabs, InlineMessage, InlineMessageKind, MethodBadge,
};
use crate::views::request_editor::RequestEditorEntities;
use crate::views::response_view::ResponseEditorEntities;
use crate::views::send::SendingTask;
use crate::views::settings::SettingsSelects;
use telaio_ui::components::open_shortcuts_dialog;

use super::sidebar;

/// Something to do once the workspace being opened is ready (see
/// [`AppView::when_workspace_opens`]).
type AfterOpen = Box<dyn FnOnce(&mut AppView, &mut Window, &mut Context<AppView>)>;

/// A workspace folder being scanned on the background executor by
/// [`AppView::open_workspace_at`].
pub(crate) struct OpeningWorkspace {
    /// The folder being opened, as given.
    pub(crate) root: PathBuf,
    /// What waits for it: a request to open in a tab, the startup debug hooks. Dropped if the
    /// open fails.
    after: Vec<AfterOpen>,
    /// Delivers the scan's result. Opening another folder replaces this, which drops it and so
    /// discards the result.
    _task: Task<()>,
}

/// How long [`AppView::show_toast`]'s pill stays up before it dismisses itself.
const TOAST_DURATION: Duration = Duration::from_millis(2600);

/// The main window view: title bar, resizable sidebar, and main area (open tabs plus the
/// request editor and response viewer).
pub struct AppView {
    /// The window-level focus this view tracks (`.track_focus` in [`Render::render`]), focused
    /// at startup by [`Self::new`] and returned to whenever a field's own Escape blurs it
    /// (`views/components/url_bar.rs`'s `on_escape`, GitHub #19). `on_action` (`Ctrl ,`, `Ctrl
    /// S`, `Ctrl 1..9`, ...) only dispatches to a view while one of its own focus handles (or a
    /// descendant's) is focused, so with nothing tracked here every shortcut went dead the moment
    /// focus left an `Input` with nowhere else to land, including at a fresh startup where
    /// nothing is ever focused at all.
    pub(crate) focus_handle: FocusHandle,
    /// Plain application state: workspace, open tabs, active environment, session environment.
    pub(crate) state: AppState,
    /// The sidebar collection tree's interaction state (selection, expansion, scrolling). This
    /// is `gpui` state, so it lives here rather than in [`AppState`].
    pub(crate) tree_state: Entity<TreeState>,
    /// The id of the tree entry that was open as a tab the last time we checked, so a new
    /// selection opens a tab exactly once instead of reloading the file from disk on every
    /// re-render (which would discard in-progress edits).
    pub(crate) last_selected_request: Option<String>,
    /// While an entry is dragged in the sidebar: the folder under the pointer during the current
    /// move event, collected by the tree's handlers (`views/sidebar.rs`).
    pub(crate) sidebar_drop_hover: Option<String>,
    /// While an entry is dragged in the sidebar: the folder it would move to (`""` for the
    /// root), highlighted in the tree. `None` when dropping would change nothing.
    pub(crate) sidebar_drop_target: Option<String>,
    /// The sidebar entries picked with Ctrl+click (Cmd+click on macOS), moved together when one
    /// of them is dragged.
    pub(crate) sidebar_selection: state::tree_selection::TreeSelection,
    /// The folder row last pressed and whether it was expanded before the press, so a drag that
    /// starts there can undo the tree's toggle (`views/sidebar.rs`).
    pub(crate) sidebar_folder_press: Option<(String, bool)>,
    /// The autocompletion menu of a request editor field, while it is open
    /// (`views/completion.rs`).
    pub(crate) completion: Option<crate::views::completion::ActiveCompletion>,
    /// The message of the last operation that failed, shown as a banner until the next
    /// successful operation clears it. `gpui` has no blocking error dialogs in this app by
    /// design (`AGENTS.md`: no native blocking dialogs), so failures show inline instead.
    pub(crate) workspace_error: Option<String>,
    /// The `gpui` entities behind the request editor's editable fields. See
    /// `views/request_editor.rs`.
    pub(crate) request_editor: RequestEditorEntities,
    /// The response viewer's read-only body editor entity. See `views/response_view.rs`.
    pub(crate) response_editor: ResponseEditorEntities,
    /// Whether the method selector shows the inline custom-method `Input` instead of the dropdown.
    /// See `views/request_editor.rs`'s `begin_editing_custom_method`/`commit_custom_method`.
    pub(crate) editing_method: bool,
    /// Which request editor tab (Params, Headers, ...) is active. Shared across every open tab
    /// for simplicity: switching tabs keeps the same editor tab selected, which matches how most
    /// tabbed editors behave.
    pub(crate) active_request_tab: RequestTab,
    /// Which response viewer tab (Body, Headers, ...) is active.
    pub(crate) active_response_tab: ResponseTab,
    /// Whether the response Body tab shows the raw body instead of the pretty-printed one.
    pub(crate) response_raw: bool,
    /// The request currently being sent, if any. See `views/send.rs`.
    pub(crate) sending: Option<SendingTask>,
    /// The last [`RunResult`] for each tab id that has been sent at least once.
    pub(crate) responses: HashMap<String, RunResult>,
    /// Which response is stored per tab, so the Body tab can tell a new response from a re-render.
    pub(crate) response_generations: crate::state::response_render::ResponseGenerations,
    /// The keys of each tab's last response body, for the post script's `res.json()`
    /// completion. Built on the background thread that sent the request; absent when the body
    /// is not JSON or is too big.
    pub(crate) response_shapes: HashMap<String, Arc<JsonShape>>,
    /// The Console tab's timeline of each tab's last response, built on the background thread
    /// that sent the request (#194).
    pub(crate) response_consoles: HashMap<String, Arc<state::console::ConsoleTimeline>>,
    /// What the user changed in the Console tab (filter, open tree nodes) of the shown response.
    pub(crate) console_view: state::console::ConsoleView,
    /// The Console tab's text filter.
    pub(crate) console_filter_input: Entity<InputState>,
    /// The Console's network tree focus, taken by a click on a row, for the arrow keys and
    /// Ctrl+C (#197).
    pub(crate) console_tree_focus: FocusHandle,
    /// Set when an arrow key moved the tree's selection: the selected row scrolls into view at
    /// its next prepaint.
    pub(crate) console_reveal: std::rc::Rc<std::cell::Cell<bool>>,
    /// The script engine every send uses. `Arc` so it is cheap to clone into each background
    /// send task; `QuickJsEngine` needs no per-run state, a fresh QuickJS runtime is created for
    /// every script run (see `brasa-script`).
    pub(crate) script_engine: Arc<dyn ScriptEngine>,
    /// The options every send and load test run uses (timeout, redirects, TLS verification). The
    /// invalid TLS certificate mode comes from Settings, "Requests"; the rest are the defaults.
    pub(crate) send_options: SendOptions,
    /// The category the Settings modal shows (`views/settings.rs`).
    pub(crate) settings_category: SettingsCategory,
    /// The Settings selects (language, fonts, TLS, ...), created each time the modal opens. See
    /// `views/settings/selects.rs`.
    pub(crate) settings_selects: Option<SettingsSelects>,
    /// Version, platform and paths shown by Settings, "About", read once at startup.
    pub(crate) about_info: AboutInfo,
    /// The UI state `BRASA_OPEN` requested at startup, if any (`state::debug_open`). `None` on
    /// a normal launch.
    pub(crate) debug_open: Option<DebugOpenTarget>,
    /// The `InputState` backing the components gallery's `UrlBar` demo
    /// (`views/components/gallery.rs`). Created lazily by [`Self::apply_debug_open`], never on a
    /// normal launch.
    pub(crate) gallery_url_input: Option<Entity<InputState>>,
    /// The sidebar filter's live text input. Subscribed once in [`Self::new`]; edits call
    /// [`Self::on_sidebar_filter_changed`].
    pub(crate) sidebar_filter_input: Entity<InputState>,
    /// The Source Control panel's commit message (`views/git/sync.rs`).
    pub(crate) commit_input: Entity<TextareaState>,
    /// The sidebar tree's per-folder expand state, captured right before the filter went from
    /// empty to non-empty, so clearing the filter can restore it instead of resetting every
    /// folder to expanded. `None` while the filter is empty.
    pub(crate) sidebar_filter_pre_expansion: Option<HashMap<String, bool>>,
    /// The collections an OpenAPI import created in the workspace, read with the tree, for the
    /// command palette's "Update from OpenAPI" items (GitHub #203).
    pub(crate) openapi_collections: Vec<brasa_workspace::OpenApiCollection>,
    /// The Code snippet dialog's currently selected language. Kept here (rather than local to the
    /// dialog) because it must survive the dialog's `content` closure being re-invoked on every
    /// repaint: see `views/snippet_dialog.rs`'s module doc comment.
    pub(crate) snippet_language: SnippetLanguage,
    /// The `gpui` entities behind whichever load test tab is active: its config panel's numeric
    /// inputs and target picker. See `views/load_test/config_panel.rs`.
    pub(crate) load_test_entities: crate::views::load_test::LoadTestEntities,
    /// The running (or just-finished) `brasa_load::LoadRun` behind every load test tab that has
    /// been started at least once, keyed by tab id. Unlike [`Self::request_editor`]/
    /// [`Self::response_editor`] (only ever built for the active tab), a load test keeps running
    /// in the background while another tab is active, so this is a map, not a single slot. See
    /// `views/load_test/run.rs`.
    pub(crate) load_runs: HashMap<String, crate::views::load_test::run::LoadRunHandle>,
    /// The load test export in progress, if any, see `views/load_test/export.rs`.
    pub(crate) export_job: Option<crate::views::load_test::export::ExportJob>,
    /// The welcome tab's scroll position, shared by its area and its scrollbars.
    pub(crate) welcome_scroll: ScrollHandle,
    /// The What's new tab's notes, parsed the first time it opens, and its scroll position.
    pub(crate) whats_new_text: Option<Entity<gpui_kit::component::text::TextViewState>>,
    pub(crate) whats_new_scroll: ScrollHandle,
    /// The scroll position of the response Headers, Tests and Console tabs, shared by the area
    /// and its scrollbar.
    pub(crate) response_scroll: ScrollHandle,
    /// The latency distribution bar under the pointer, for its crosshair and tooltip. Only the
    /// active tab shows a distribution, so one slot is enough.
    pub(crate) latency_hover: Option<usize>,
    /// The background computations of every load test tab's results
    /// (`views/load_test/results_data.rs`).
    pub(crate) load_result_tasks: crate::views::load_test::results_data::LoadResultTaskMap,
    /// The activity rail's section and collapsed state (`views/navigation.rs`).
    pub(crate) nav: state::navigation::NavState,
    /// The Environments panel's rows, loaded when the workspace or an environment changes
    /// ([`Self::refresh_environments`]) so render never reads files. The environment picker and
    /// the command palette list these names too.
    pub(crate) env_rows: Vec<state::env_panel::EnvRow>,
    /// Whether the color picker of the active environment tab is open (#217,
    /// `views/env_color_picker.rs`).
    pub(crate) env_color_picker_open: bool,
    /// The status bar's git branch (or short hash) of the open workspace, read when it opens and
    /// when the window gets the focus back ([`Self::refresh_git_branch`]), so render never reads
    /// `.git/HEAD`.
    pub(crate) git_branch: Option<String>,
    /// The Git integration's state: availability, repository and status (`views/git`).
    pub(crate) git: state::git::GitState,
    /// The git commands running in the background (`views/git`).
    pub(crate) git_tasks: crate::views::git::GitTasks,
    /// The choice of the "Check out with changes" dialog (design 6f).
    pub(crate) checkout_choice: crate::views::git::CheckoutChoice,
    /// The status bar's branch menu (`views/git/branch_menu.rs`).
    pub(crate) branch_menu: Entity<telaio_ui::components::SearchableMenuState>,
    /// What the branch menu's submenu and create row need to know, updated with its rows.
    pub(crate) branch_menu_context: std::rc::Rc<std::cell::RefCell<crate::views::git::MenuContext>>,
    /// The `gpui` entities behind every open environment editor tab, keyed by tab id, created
    /// when the tab is first shown. See `views/env_editor.rs`.
    pub(crate) env_editors: HashMap<String, crate::views::env_editor::EnvEditorEntities>,
    /// The Environments panel's inline "new environment" name input, `Some` while it is shown.
    pub(crate) new_env_input: Option<Entity<InputState>>,
    /// The Load tests panel's target tree (`views/load_panel.rs`), rebuilt by
    /// [`Self::refresh_tree`].
    pub(crate) load_targets: state::load_panel::TargetTree,
    /// The recently opened workspaces, most recent first, for the title bar's workspace
    /// switcher. Read from the config file at startup and after opening a workspace
    /// ([`Self::open_workspace_at`]), so render never reads it.
    pub(crate) recent_workspaces: Vec<PathBuf>,
    /// The open workspace's absolute path, the form [`Self::recent_workspaces`] holds, so the
    /// switcher can tick the open one without canonicalizing a path on every render.
    pub(crate) workspace_path: Option<PathBuf>,
    /// The workspace folder being scanned in the background, `None` once it is open or failed
    /// to open. The sidebar shows it as loading meanwhile.
    pub(crate) opening: Option<OpeningWorkspace>,
    /// How many times [`Render::render`] ran, logged at Trace. A count that keeps growing while
    /// nobody touches the app means something keeps calling `cx.notify()`.
    render_count: u64,
    /// The brief confirmation pill shown over the bottom right of the window ("Saved"), if any.
    /// See [`Self::show_toast`].
    pub(crate) toast: Option<Toast>,
    /// Bumped by every [`Self::show_toast`], so a toast's own delayed dismiss only clears it
    /// when its generation still matches: an older toast's timer firing late never dismisses a
    /// newer one shown in the meantime.
    toast_generation: u64,
}

/// A brief, self-dismissing confirmation message, native shapes and gpui's own
/// `Animation`/`AnimationExt` (the same technique as the welcome tab's entrance), not the stock
/// `Notification` widget: its own layout did not account for this app's title bar and did not
/// line up with it. See [`AppView::show_toast`] and `views::toast::render_toast`.
pub(crate) struct Toast {
    pub(crate) message: SharedString,
    generation: u64,
}

impl AppView {
    /// Creates a fresh view, opening `initial_workspace` right away if one was given (the CLI
    /// argument or the remembered last workspace, see `main.rs`), and the welcome tab when
    /// `show_welcome` says so (`state::welcome::show_at_startup`), after that workspace, then the
    /// What's new tab when `show_whats_new` says so (`state::whats_new::opens_at_startup`).
    /// `settings` is whatever `main.rs`
    /// loaded from `settings.toml` (or its defaults) before opening the window; the theme and
    /// fonts it describes are already applied by the time this runs, this just remembers it so
    /// the Settings view (`views/settings.rs`) has a value to show and change. `window` is only
    /// needed for [`Self::apply_debug_open`] (opening a dialog, or here creating the gallery's
    /// `InputState`, needs it); every other debug hook ignores it.
    pub fn new(
        initial_workspace: Option<PathBuf>,
        initial_request: Option<String>,
        show_welcome: bool,
        show_whats_new: bool,
        settings: Settings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tree_state = cx.new(|cx| TreeState::new(cx));
        let sidebar_filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("shell.sidebar.filter")));
        let console_filter_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("response.console.filter")));
        cx.subscribe(
            &console_filter_input,
            |view, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let query = input.read(cx).value().to_string();
                    view.console_view.set_query(&query);
                    cx.notify();
                }
            },
        )
        .detach();
        let branch_menu_context = std::rc::Rc::default();
        let branch_menu = crate::views::git::new_branch_menu(
            std::rc::Rc::clone(&branch_menu_context),
            window,
            cx,
        );
        let commit_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("git.commit.placeholder"))
                .auto_grow(4, 10)
        });
        // The commit buttons are enabled by the message, so its edits repaint the panel.
        cx.subscribe(&commit_input, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(
            &sidebar_filter_input,
            |view, _entity, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.on_sidebar_filter_changed(cx);
                }
            },
        )
        .detach();
        let focus_handle = cx.focus_handle();
        // The modal backdrop is painted by `render` from `has_active_dialog`, but opening or
        // closing a dialog notifies only gpui-component's private window state, not this view.
        // Opening one moves focus into the dialog, out of this view; closing it restores focus
        // back in. Re-rendering on both keeps the backdrop in step, with no work while idle.
        cx.on_focus_out(&focus_handle, window, |_, _, _, cx| cx.notify())
            .detach();
        cx.on_focus_in(&focus_handle, window, |_, _, cx| cx.notify())
            .detach();
        // A dialog gives the focus back to whatever had it when the dialog opened
        // (`Root::open_dialog` keeps a weak handle). Opened from a menu, that is the menu, gone
        // by the time the dialog closes, which left nothing focused and every window shortcut
        // dead until a click (GitHub #96). Whenever nothing has the focus and no dialog is open,
        // this view takes it, as a click on the window would. With a dialog open the focus stays
        // where it is, so shortcuts never act behind a modal.
        //
        // gpui draws no new frame after a focus-lost listener moves the focus, and a notify from
        // inside the listener is dropped with the frame being drawn, so the refresh is deferred.
        // It matters: a dialog closed with Enter restores the menu's handle 250 ms later
        // (`Root::defer_close_dialog`), once more focusing nothing, and only a frame drawn with
        // this view focused in between lets gpui see that second loss and call this again.
        cx.on_focus_lost(window, |view, window, cx| {
            if !window.has_active_dialog(cx) {
                view.focus_handle.focus(window, cx);
                cx.defer_in(window, |_, window, _| window.refresh());
            }
        })
        .detach();
        // Brasa does not watch the workspace folder: coming back to the window is when files
        // changed elsewhere, such as an `.env` edited in another editor or a branch switched in a
        // terminal, are read again. An event, never a timer, so an idle window does no work.
        cx.observe_window_activation(window, |view, window, cx| {
            if window.is_window_active() {
                view.refresh_environments();
                view.refresh_git_branch();
                view.refresh_git_status(cx);
                view.maybe_auto_fetch(window, cx);
                cx.notify();
            }
        })
        .detach();

        let send_options = SendOptions {
            invalid_certificates: settings.invalid_tls_certificates.to_http(),
            system_certificates: settings.system_certificates,
            max_response_size: settings.max_response_bytes(),
            ..SendOptions::default()
        };
        let mut view = Self {
            focus_handle: focus_handle.clone(),
            state: AppState {
                settings,
                ..AppState::new()
            },
            tree_state,
            last_selected_request: None,
            sidebar_drop_hover: None,
            sidebar_drop_target: None,
            sidebar_selection: Default::default(),
            sidebar_folder_press: None,
            completion: None,
            workspace_error: None,
            request_editor: RequestEditorEntities::default(),
            response_editor: ResponseEditorEntities::default(),
            editing_method: false,
            active_request_tab: RequestTab::default(),
            active_response_tab: ResponseTab::default(),
            response_raw: false,
            sending: None,
            responses: HashMap::new(),
            response_generations: Default::default(),
            response_shapes: HashMap::new(),
            response_consoles: HashMap::new(),
            console_view: Default::default(),
            console_filter_input,
            console_tree_focus: cx.focus_handle(),
            console_reveal: Default::default(),
            script_engine: Arc::new(QuickJsEngine),
            send_options,
            settings_category: SettingsCategory::default(),
            settings_selects: None,
            about_info: AboutInfo::detect(),
            debug_open: None,
            gallery_url_input: None,
            sidebar_filter_input,
            commit_input,
            sidebar_filter_pre_expansion: None,
            openapi_collections: Vec::new(),
            snippet_language: SnippetLanguage::Curl,
            load_test_entities: crate::views::load_test::LoadTestEntities::default(),
            load_runs: HashMap::new(),
            latency_hover: None,
            welcome_scroll: ScrollHandle::new(),
            whats_new_text: None,
            whats_new_scroll: ScrollHandle::new(),
            response_scroll: ScrollHandle::new(),
            export_job: None,
            load_result_tasks: HashMap::new(),
            nav: state::navigation::NavState::default(),
            env_rows: Vec::new(),
            env_color_picker_open: false,
            git_branch: None,
            git: Default::default(),
            git_tasks: Default::default(),
            checkout_choice: Default::default(),
            branch_menu,
            branch_menu_context,
            env_editors: HashMap::new(),
            new_env_input: None,
            load_targets: state::load_panel::TargetTree::default(),
            recent_workspaces: state::config::recent_workspaces(),
            workspace_path: None,
            opening: None,
            render_count: 0,
            toast: None,
            toast_generation: 0,
        };
        // Once per launch: `git --version`, then the workspace's repository once it opens.
        view.detect_git(cx);
        if let Some(root) = initial_workspace {
            view.open_workspace_at(&root, window, cx);
            // A file given on the command line: open its request in a tab, without sending.
            if let Some(id) = initial_request {
                view.when_workspace_opens(window, cx, move |view, _, cx| {
                    view.open_request(id, cx);
                });
            }
        }
        if show_welcome {
            view.when_workspace_opens(window, cx, |view, _, cx| view.open_welcome_tab(cx));
        }
        if show_whats_new {
            view.when_workspace_opens(window, cx, |view, _, cx| view.open_whats_new_tab(cx));
        }
        // Before `apply_debug_open`: a `BRASA_OPEN=settings` launch defers opening the dialog
        // (see that method), and `Root::open_dialog` captures whatever is focused at that later
        // point as the handle to restore once the dialog closes, so this must already be in
        // effect for that capture to see it.
        focus_handle.focus(window, cx);
        // Both hooks act on the workspace (send one of its requests, open an environment), so
        // they wait for the scan.
        view.when_workspace_opens(window, cx, |view, window, cx| {
            view.apply_debug_autosend(cx);
            view.apply_debug_open(window, cx);
        });
        view.schedule_startup_update_check(cx);
        view
    }

    /// `BRASA_OPEN=<target>` opens a specific UI state at startup, to
    /// screenshot a UI state. A no-op when unset or unrecognized, same convention as
    /// [`Self::apply_debug_autosend`]: not a supported feature, not surfaced in any menu.
    /// Recognized targets are listed on [`crate::state::debug_open::DebugOpenTarget`].
    fn apply_debug_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // `BRASA_IMPORT=<paths>` opens the import wizard on those files, as a drop on the window
        // does. Native file pickers and drops are out of reach of `xdotool`.
        if let Some(paths) = std::env::var_os("BRASA_IMPORT") {
            let paths: Vec<PathBuf> = std::env::split_paths(&paths).collect();
            let view = cx.weak_entity();
            window.defer(cx, move |window, cx| {
                super::openapi_import::open_wizard_with_files(view, paths, window, cx);
            });
        }
        let Ok(value) = std::env::var("BRASA_OPEN") else {
            return;
        };
        if let Some(name) = debug_open::parse_env(&value) {
            self.nav.click(state::navigation::NavSection::Environments);
            self.open_environment_tab(name.to_string(), cx);
            return;
        }
        let Some(target) = debug_open::parse(&value) else {
            return;
        };
        self.debug_open = Some(target);
        match target {
            DebugOpenTarget::Components => {
                self.gallery_url_input = Some(cx.new(|cx| {
                    InputState::new(window, cx).default_value("{{baseUrl}}/users/{{missing}}")
                }));
            }
            DebugOpenTarget::Settings
            | DebugOpenTarget::SettingsRequests
            | DebugOpenTarget::SettingsAdvanced
            | DebugOpenTarget::SettingsAbout => {
                if target == DebugOpenTarget::SettingsAbout {
                    self.settings_category = SettingsCategory::About;
                }
                if target == DebugOpenTarget::SettingsRequests {
                    self.settings_category = SettingsCategory::Requests;
                }
                if target == DebugOpenTarget::SettingsAdvanced {
                    self.settings_category = SettingsCategory::Advanced;
                }
                // Opening a dialog needs the window's `Root` (`WindowExt::open_dialog` panics
                // otherwise, "window first layer should be a gpui_component::Root"), which does
                // not exist yet this early: `AppView::new` (this hook's caller) still runs inside
                // `gpui`'s own `open_window`, before it assigns the view this method builds as
                // the window's root. Deferring one tick, with `window.defer`, runs after that
                // assignment, same as everywhere else in `gpui` that must wait for it (see
                // `open_window`'s own use of `window.defer` for `appearance_changed`).
                let weak = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = weak.update(cx, |view, cx| view.open_settings(window, cx));
                });
            }
            DebugOpenTarget::Palette => {
                let weak = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = weak.update(cx, |view, cx| view.open_command_palette(window, cx));
                });
            }
            DebugOpenTarget::Snippet => {
                let weak = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = weak.update(cx, |view, cx| view.open_snippet_dialog(window, cx));
                });
            }
            DebugOpenTarget::LoadTest => {
                self.open_load_test_tab_for_first_request(cx);
            }
        }
        cx.notify();
    }

    /// Hidden debug hooks for end-to-end checks, since there is no tool to simulate clicks in this
    /// app. Both are no-ops when unset, so they never affect a normal launch; they exist for `make
    /// run`/manual testing, not as a supported feature, so neither is surfaced in any menu or
    /// documented outside this comment.
    ///
    /// - `BRASA_ENV=<name>` selects an environment before sending, matching a name under
    ///   `environments/` in the opened workspace.
    /// - `BRASA_AUTOSEND=<request id>` opens that request (a workspace-relative id, for
    ///   example `auth/login.brasa`) and sends it immediately at startup.
    fn apply_debug_autosend(&mut self, cx: &mut Context<Self>) {
        if self.state.workspace.is_none() {
            return;
        }
        if let Ok(env_name) = std::env::var("BRASA_ENV") {
            self.state.set_active_environment(Some(env_name));
        }
        if let Ok(request_id) = std::env::var("BRASA_AUTOSEND") {
            self.open_request(request_id, cx);
            self.send_active_tab(cx);
        }
    }

    /// Opens files the OS asked the app to open (macOS "Open with" and double click). A file
    /// inside the open workspace opens in a tab; any other file switches to its parent folder
    /// first, through the same path as "Open folder...", then opens in a tab. A directory opens as
    /// the workspace.
    pub(crate) fn open_external_files(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            if path.is_dir() {
                log::info!("opening a folder requested by the OS");
                self.open_workspace_at(&path, window, cx);
                continue;
            }
            if !path.is_file() {
                log::warn!("the OS asked to open a file that does not exist");
                continue;
            }
            // The folder being opened counts as the current one: a second file from the same
            // folder joins that open instead of starting it again.
            let current = match self.opening.as_ref() {
                Some(opening) => Some(opening.root.clone()),
                None => self
                    .state
                    .workspace
                    .as_ref()
                    .map(|workspace| workspace.root().to_path_buf()),
            };
            let target = state::launch::target_for_file(&path, current.as_deref());
            if current.as_deref() != Some(target.workspace.as_path()) {
                self.open_workspace_at(&target.workspace, window, cx);
            }
            if let Some(id) = target.request_id {
                self.when_workspace_opens(window, cx, move |view, _, cx| {
                    view.open_request(id, cx);
                });
            }
        }
    }

    /// Opens `root` as the workspace. The folder is scanned on the background executor, so a big
    /// one never freezes the window, and [`Self::finish_opening_workspace`] swaps it in. Opening
    /// another folder before this one is ready discards this one. Used at startup, by the title
    /// bar's workspace switcher ("Open folder..." and picking a recent workspace), `Ctrl O`, and
    /// for folders and files the OS hands over.
    pub(crate) fn open_workspace_at(
        &mut self,
        root: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scan_root = root.to_path_buf();
        let task = cx.spawn_in(window, async move |this, cx| {
            // Timed here, not up to `finish_opening_workspace`: at startup that also waits for
            // the window's first frames.
            let (result, elapsed) = cx
                .background_spawn(async move {
                    let started = Instant::now();
                    (Workspace::open(scan_root), started.elapsed())
                })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                view.finish_opening_workspace(result, elapsed, window, cx);
            });
        });
        self.opening = Some(OpeningWorkspace {
            root: root.to_path_buf(),
            after: Vec::new(),
            _task: task,
        });
        cx.notify();
    }

    /// Runs `after` once the workspace being opened is ready, or right away when none is being
    /// opened. It is dropped if that open fails, so a request id never opens in the previous
    /// workspace instead.
    pub(crate) fn when_workspace_opens(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        after: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        match self.opening.as_mut() {
            Some(opening) => opening.after.push(Box::new(after)),
            None => after(self, window, cx),
        }
    }

    /// Swaps in the workspace [`Self::open_workspace_at`] scanned, remembers it for next launch,
    /// clears the sidebar filter (it belonged to the previous workspace), refreshes the sidebar
    /// tree, then runs what was waiting for it. A failed open keeps the previous workspace and
    /// shows the error.
    fn finish_opening_workspace(
        &mut self,
        result: Result<Workspace, WorkspaceError>,
        elapsed: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(OpeningWorkspace { root, after, .. }) = self.opening.take() else {
            return;
        };
        let root = root.as_path();
        let opened = result.is_ok();
        match result {
            Ok(workspace) => {
                self.state.set_workspace(workspace);
                self.log_opened_workspace(root, elapsed);
                self.workspace_error = None;
                // Remember an absolute path: a relative one (as given on the command line, for
                // example) would resolve against whatever directory the app happens to be
                // launched from next time, not necessarily the same folder. `dunce` avoids the
                // verbatim `\\?\C:\...` form `std` returns on Windows.
                let absolute = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
                state::config::record_workspace(&absolute);
                self.recent_workspaces = state::config::recent_workspaces();
                self.workspace_path = Some(absolute.clone());
                self.restore_last_environment(&absolute);
                self.refresh_environments();
                self.refresh_git_branch();
                self.discover_git_repo(cx);
            }
            Err(error) => {
                log_workspace_error(&format!("open workspace {}", root.display()), &error);
                self.workspace_error = Some(error.to_string());
                // Nothing open and nothing to show, as when the folder remembered at startup
                // can no longer be opened: the welcome tab offers a way forward.
                if self.state.workspace.is_none() && self.state.tabs.open_tabs().is_empty() {
                    self.open_welcome_tab(cx);
                }
            }
        }
        self.last_selected_request = None;
        self.sidebar_selection.clear();
        self.new_env_input = None;
        self.env_editors.clear();
        self.sidebar_filter_pre_expansion = None;
        self.sidebar_filter_input.update(cx, |state, cx| {
            state.set_value(String::new(), window, cx);
        });
        // The live tree belongs to the previous workspace: drop it so no expand flag carries
        // over and every folder of the new one starts collapsed.
        self.tree_state
            .update(cx, |state, cx| state.set_items(Vec::new(), cx));
        self.refresh_tree(cx);
        cx.notify();
        if opened {
            for after in after {
                after(self, window, cx);
            }
        }
    }

    /// Re-selects the environment that was active when the workspace at `root` was last used,
    /// unless it has been deleted or renamed since.
    fn restore_last_environment(&mut self, root: &Path) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let Some(name) = state::config::last_environment(root) else {
            return;
        };
        let exists = workspace
            .list_environments()
            .is_ok_and(|names| names.contains(&name));
        if exists {
            log::debug!("restored active environment: {name}");
            self.state.set_active_environment(Some(name));
        }
    }

    /// Logs a successfully opened workspace: where it is, what it holds, how long the scan took,
    /// and which request files do not parse.
    fn log_opened_workspace(&self, root: &Path, elapsed: Duration) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let counts = tree_counts(workspace.tree());
        let environments = workspace.list_environments().map_or(0, |names| names.len());
        log::info!(
            "opened workspace {} in {} ms: {} requests, {} folders, {} environments",
            root.display(),
            elapsed.as_millis(),
            counts.requests,
            counts.folders,
            environments,
        );
        for id in counts.broken {
            log::warn!("{id} does not parse, it cannot be opened until fixed");
        }
    }

    /// Reads the workspace from disk again and shows it: the collections tree and the
    /// environments, with the active one's variables. For changes Brasa did not make itself
    /// (a pull, a checkout, a git discard). `after` names the cause in the log.
    pub(crate) fn reload_workspace_from_disk(&mut self, after: &str, cx: &mut Context<Self>) {
        if let Some(workspace) = self.state.workspace.as_mut()
            && let Err(error) = workspace.rescan()
        {
            log_workspace_error(&format!("rescanning the workspace after {after}"), &error);
        }
        self.refresh_tree(cx);
        self.refresh_environments();
    }

    /// The sidebar's refresh button and the palette's "Refresh collections" (#219): shows the
    /// changes made outside Brasa (a pull in a terminal, files copied in by hand).
    pub(crate) fn refresh_collections(&mut self, cx: &mut Context<Self>) {
        if self.state.workspace.is_none() {
            return;
        }
        log::info!("refreshing the workspace from disk");
        self.reload_workspace_from_disk("a refresh", cx);
        cx.notify();
    }

    /// Rebuilds the sidebar tree from the workspace's current on-disk state, respecting the
    /// sidebar filter's current text. Call after any operation that changes the workspace (save,
    /// create, rename, delete).
    pub(crate) fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        self.refresh_tree_with(cx, |_| {});
    }

    /// Like [`Self::refresh_tree`], but `edit` first adjusts the folders' expand flags (reveal a
    /// new entry, follow a renamed folder). Folders keep the state they had; new ones are collapsed.
    pub(crate) fn refresh_tree_with(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut HashMap<String, bool>),
    ) {
        let query = self.sidebar_filter_input.read(cx).value().to_string();
        // While the filter is on, the live tree is forced open: the real flags are the ones
        // captured when the filter started.
        let mut expansion = match self.sidebar_filter_pre_expansion.take() {
            Some(captured) => captured,
            None => self.snapshot_tree_expansion(cx),
        };
        edit(&mut expansion);
        let items = self.tree_items_for_query(&query, &expansion);
        if !query.trim().is_empty() {
            self.sidebar_filter_pre_expansion = Some(expansion);
        }
        self.tree_state
            .update(cx, |state, cx| state.set_items(items, cx));
        let nodes = self
            .state
            .workspace
            .as_ref()
            .map_or(&[][..], |workspace| workspace.tree());
        self.load_targets.rebuild(nodes);
        if !self.sidebar_selection.is_empty() {
            let existing = state::tree_selection::ids_in(nodes);
            self.sidebar_selection
                .retain_existing(|id| existing.contains(id));
        }
        // The tree changes after Brasa writes a file (or a rescan), which also changes git's
        // status; the method badges of the Source Control rows come from the tree.
        self.openapi_collections = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.openapi_collections())
            .unwrap_or_default();
        self.rebuild_git_rows();
        self.refresh_git_status(cx);
    }

    /// Called on every edit to [`Self::sidebar_filter_input`]. Narrows the tree while the filter is
    /// non-empty (capturing the current expand state once, on the empty-to-non-empty transition);
    /// restores that captured expand state when the filter goes back to empty.
    pub(crate) fn on_sidebar_filter_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.sidebar_filter_input.read(cx).value().to_string();
        let items = if query.trim().is_empty() {
            let expansion = match self.sidebar_filter_pre_expansion.take() {
                Some(captured) => captured,
                None => self.snapshot_tree_expansion(cx),
            };
            self.tree_items_for_query(&query, &expansion)
        } else {
            if self.sidebar_filter_pre_expansion.is_none() {
                self.sidebar_filter_pre_expansion = Some(self.snapshot_tree_expansion(cx));
            }
            self.tree_items_for_query(&query, &HashMap::new())
        };
        self.tree_state
            .update(cx, |state, cx| state.set_items(items, cx));
    }

    /// Builds the sidebar's `TreeItem`s for the open workspace: the ordinary tree with the
    /// folders' `expansion` when `query` is empty, the filtered tree otherwise. Empty (no
    /// workspace open) when there is nothing to show.
    fn tree_items_for_query(
        &self,
        query: &str,
        expansion: &HashMap<String, bool>,
    ) -> Vec<gpui_kit::component::tree::TreeItem> {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return Vec::new();
        };
        if query.trim().is_empty() {
            sidebar::build_tree_items(workspace.tree(), expansion)
        } else {
            sidebar::build_filtered_tree_items(workspace.tree(), query)
        }
    }

    /// Snapshots every folder's current expand flag from the live sidebar tree, so
    /// [`Self::on_sidebar_filter_changed`] can restore it once the filter is cleared.
    fn snapshot_tree_expansion(&self, cx: &Context<Self>) -> HashMap<String, bool> {
        let tree_state = self.tree_state.read(cx);
        let mut map = HashMap::new();
        let mut index = 0;
        while let Some(entry) = tree_state.entry(index) {
            if entry.is_folder() {
                map.insert(entry.item().id.to_string(), entry.is_expanded());
            }
            index += 1;
        }
        map
    }

    /// Expands the folders above `id` when some are collapsed, so a request opened from
    /// elsewhere (the command line, a script, a new tab) is visible in the sidebar.
    fn reveal_in_tree(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.sidebar_filter_input.read(cx).value().trim().is_empty() {
            return;
        }
        let expansion = self.snapshot_tree_expansion(cx);
        if state::tree_expansion::is_hidden(&expansion, id) {
            self.refresh_tree_with(cx, |expansion| state::tree_expansion::reveal(expansion, id));
        }
    }

    /// Loads `id` from the workspace and opens it as a tab (or activates it, if already open).
    pub(crate) fn open_request(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        match workspace.load_request(&id) {
            Ok(request) => {
                // Only a genuinely new tab gets the `Body`-tab heuristic below: re-selecting an
                // already open one keeps whatever tab the user last looked at, same as before.
                let is_new_tab = self.state.tabs.index_of(&id).is_none();
                let query_len = request.query.len();
                let has_body = !matches!(request.body, Body::None);
                if is_new_tab {
                    log::debug!("opened tab {id}");
                }
                self.state.tabs.open(id.clone(), request);
                self.workspace_error = None;
                self.reveal_in_tree(&id, cx);
                if is_new_tab {
                    self.active_request_tab =
                        ui_tabs::tab_to_show_on_open(self.active_request_tab, query_len, has_body);
                }
            }
            Err(error) => {
                log_workspace_error(&format!("open {id}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Closes the tab at `index`. Stops any load test still running in it first: dropping its
    /// `LoadRunHandle` (see `views/load_test/run.rs`) cancels the periodic refresh task and
    /// detaches the run's supervisor thread, which exits shortly after observing the stop signal.
    pub(crate) fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(tab_id) = self
            .state
            .tabs
            .open_tabs()
            .get(index)
            .map(|tab| tab.id.clone())
        {
            log::debug!("closed tab {tab_id}");
            self.stop_load_test(&tab_id);
            self.env_editors.remove(&tab_id);
        }
        self.state.tabs.close(index);
        cx.notify();
    }

    /// Closes every tab at `indices`, highest index first, so closing one never shifts the
    /// position of another still to be closed. Each goes through [`Self::request_close_tab`], so
    /// a dirty environment tab among them still asks before it is actually closed (see that
    /// method's doc comment): the ones after it in the list still close normally. Used by the
    /// open tabs bar's context menu.
    fn close_tabs(&mut self, mut indices: Vec<usize>, window: &mut Window, cx: &mut Context<Self>) {
        indices.sort_unstable_by(|a, b| b.cmp(a));
        indices.dedup();
        for index in indices {
            self.request_close_tab(index, window, cx);
        }
    }

    /// Saves the active tab's request to disk with `brasa-format`'s canonical serialization, via
    /// [`brasa_workspace::Workspace::save_request`]. Bound to `Ctrl+S` / `Cmd+S`. A no-op when
    /// the active tab is a load test: dirty state and saving only apply to requests, so `Ctrl+S` on
    /// that tab does nothing harmful.
    pub(crate) fn save_active_tab(&mut self, cx: &mut Context<Self>) {
        let Some(active_index) = self.state.tabs.active_index() else {
            return;
        };
        let Some(tab) = self.state.tabs.active() else {
            return;
        };
        if tab.environment().is_some() {
            self.save_env_tab(active_index, cx);
            return;
        }
        let id = tab.id.clone();
        let Some(request) = tab.request().cloned() else {
            return;
        };

        let Some(workspace) = self.state.workspace.as_mut() else {
            return;
        };
        match workspace.save_request(&id, &request) {
            Ok(()) => {
                log::debug!("saved {id}");
                self.state.tabs.mark_saved(active_index);
                self.workspace_error = None;
                self.refresh_tree(cx);
                self.show_toast(t!("common.saved_toast"), cx);
            }
            Err(error) => {
                log_workspace_error(&format!("save {id}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Shows `message` as the toast, replacing any shown already, and schedules its own
    /// dismissal after [`TOAST_DURATION`]. The one-shot delayed task is the only thing that
    /// repaints the window while the toast is up: it fires once, clears the toast if a newer
    /// one has not replaced it, and is done (`AGENTS.md`, Performance).
    pub(crate) fn show_toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.toast_generation += 1;
        let generation = self.toast_generation;
        self.toast = Some(Toast {
            message: message.into(),
            generation,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_DURATION).await;
            let _ = this.update(cx, |view, cx| {
                if view
                    .toast
                    .as_ref()
                    .is_some_and(|toast| toast.generation == generation)
                {
                    view.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Creates a new, blank request named `name` and with the method `method` inside the folder
    /// `parent` (workspace root when `None`), then opens it as a tab.
    pub(crate) fn create_request(
        &mut self,
        parent: Option<String>,
        name: String,
        method: Method,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.workspace.as_mut() else {
            return;
        };
        match workspace.create_request(parent.as_deref(), &name) {
            Ok(id) => {
                log::debug!("created request {id}");
                self.workspace_error = None;
                self.refresh_tree_with(cx, |expansion| {
                    state::tree_expansion::reveal(expansion, &id);
                });
                self.open_request(id.clone(), cx);
                // `create_request` always writes a blank GET request; the dialog's method
                // picker only needs a second edit when something else was chosen.
                if method != Method::Get {
                    self.edit_active_request(&id, cx, |request| request.method = method);
                }
                return;
            }
            Err(error) => {
                log_workspace_error(&format!("create request {name:?}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Creates a new, empty folder named `name` inside the folder `parent` (workspace root when
    /// `None`).
    pub(crate) fn create_folder(
        &mut self,
        parent: Option<String>,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.workspace.as_mut() else {
            return;
        };
        match workspace.create_folder(parent.as_deref(), &name) {
            Ok(id) => {
                log::debug!("created folder {id}");
                self.workspace_error = None;
                self.refresh_tree_with(cx, |expansion| {
                    state::tree_expansion::reveal(expansion, &id);
                });
            }
            Err(error) => {
                log_workspace_error(&format!("create folder {name:?}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Renames the request or folder `id` to `new_name`, keeping any open tab under it pointing
    /// at the new path.
    pub(crate) fn rename(&mut self, id: String, new_name: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.workspace.as_mut() else {
            return;
        };
        match workspace.rename(&id, &new_name) {
            Ok(new_id) => {
                log::debug!("renamed {id} to {new_id}");
                self.state.tabs.rename_prefix(&id, &new_id);
                self.workspace_error = None;
                self.refresh_tree_with(cx, |expansion| {
                    state::tree_expansion::rename_prefix(expansion, &id, &new_id);
                });
            }
            Err(error) => {
                log_workspace_error(&format!("rename {id}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Moves the requests and folders `ids` into `folder` (`""` for the workspace root), keeping
    /// the open tabs, unsaved edits and last responses of every request they hold and the expand
    /// state of a moved folder. The selection follows them. One that cannot move (a name already
    /// in `folder`) stays where it is and is reported, without stopping the others.
    pub(crate) fn move_entries(
        &mut self,
        ids: Vec<String>,
        folder: String,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.workspace.as_mut() else {
            return;
        };
        let folder_id = (!folder.is_empty()).then_some(folder.as_str());
        let mut moved = Vec::new();
        let mut errors = Vec::new();
        for id in ids {
            match workspace.move_entry(&id, folder_id) {
                Ok(new_id) => {
                    log::debug!("moved {id} to {new_id}");
                    moved.push((id, new_id));
                }
                Err(error) => {
                    log_workspace_error(&format!("move {id}"), &error);
                    errors.push(error.to_string());
                }
            }
        }
        let mut selected = self
            .tree_state
            .read(cx)
            .selected_item()
            .map(|item| item.id.to_string());
        for (id, new_id) in &moved {
            self.state.tabs.rename_prefix(id, new_id);
            self.responses = std::mem::take(&mut self.responses)
                .into_iter()
                .map(|(key, response)| (state::tree_drop::moved_id(&key, id, new_id), response))
                .collect();
            self.response_consoles = std::mem::take(&mut self.response_consoles)
                .into_iter()
                .map(|(key, timeline)| (state::tree_drop::moved_id(&key, id, new_id), timeline))
                .collect();
            self.sidebar_selection.follow_move(id, new_id);
            selected = selected.map(|selected| state::tree_drop::moved_id(&selected, id, new_id));
            self.last_selected_request = self
                .last_selected_request
                .take()
                .map(|request| state::tree_drop::moved_id(&request, id, new_id));
        }
        self.workspace_error = (!errors.is_empty()).then(|| errors.join("\n"));
        self.refresh_tree_with(cx, |expansion| {
            for (id, new_id) in &moved {
                state::tree_expansion::rename_prefix(expansion, id, new_id);
                state::tree_expansion::reveal(expansion, new_id);
            }
        });
        // Selected without reopening anything: the tabs already follow the new ids.
        if let Some(selected) = selected {
            if selected.ends_with(".brasa") {
                self.last_selected_request = Some(selected.clone());
            }
            let selected = SharedString::from(selected);
            self.tree_state.update(cx, |tree, cx| {
                let index = tree.index_of(&selected);
                tree.set_selected_index(index, cx);
            });
        }
        cx.notify();
    }

    /// Copies the request `id` next to it (`Login copy`, see
    /// [`brasa_workspace::Workspace::duplicate_request`]) and opens the copy in a tab. The copy
    /// is the saved file: unsaved edits in an open tab stay in that tab.
    pub(crate) fn duplicate(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.workspace.as_mut() else {
            return;
        };
        match workspace.duplicate_request(&id, &t!("shell.sidebar.copy_suffix")) {
            Ok(new_id) => {
                log::debug!("duplicated {id} as {new_id}");
                self.workspace_error = None;
                self.refresh_tree_with(cx, |expansion| {
                    state::tree_expansion::reveal(expansion, &new_id);
                });
                self.open_request(new_id, cx);
                return;
            }
            Err(error) => {
                log_workspace_error(&format!("duplicate {id}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Deletes the request or folder `id`, closing any open tab under it.
    pub(crate) fn delete(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.workspace.as_mut() else {
            return;
        };
        match workspace.delete(&id) {
            Ok(()) => {
                log::debug!("deleted {id}");
                self.state.tabs.close_prefix(&id);
                self.workspace_error = None;
                self.refresh_tree(cx);
            }
            Err(error) => {
                log_workspace_error(&format!("delete {id}"), &error);
                self.workspace_error = Some(error.to_string());
            }
        }
        cx.notify();
    }

    /// Handles the `Ctrl+S` / `Cmd+S` key binding (see `main.rs`'s `bind_keys`).
    fn on_save_action(&mut self, _: &SaveActiveTab, _window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_tab(cx);
    }

    /// Re-applies every translated string that is cached outside `render` after the UI language
    /// changed. Today that means `InputState` placeholders, set once at construction: each view
    /// that owns inputs exposes a `relocalize(&mut self, window, cx)` that calls
    /// `InputState::set_placeholder` again, and this method forwards to them. Called by the
    /// Settings view before it refreshes the windows, with the new locale already applied.
    pub(crate) fn relocalize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.relocalize_sidebar(window, cx);
        self.relocalize_request_editor(window, cx);
        self.relocalize_env_editors(window, cx);
        self.relocalize_git_log(window, cx);
        self.commit_input.update(cx, |state, cx| {
            state.set_placeholder(t!("git.commit.placeholder"), window, cx);
        });
        self.console_filter_input.update(cx, |state, cx| {
            state.set_placeholder(t!("response.console.filter"), window, cx);
        });
        // The command palette and the load test tab cache no translated text: the palette
        // rebuilds its items on every repaint, the load test tab has no input placeholder.
    }

    /// Handles the `Ctrl+Enter` / `Cmd+Enter` key binding (see `main.rs`'s `bind_keys`).
    fn on_send_action(&mut self, _: &SendActiveTab, _window: &mut Window, cx: &mut Context<Self>) {
        // gpui ranks a binding without context above every context binding, so this one runs
        // before the branch menu's own Ctrl+Enter ("Create branch"): let it through.
        if self.branch_menu.read(cx).is_open() {
            cx.propagate();
            return;
        }
        self.send_active_tab(cx);
    }

    /// Handles the `Ctrl ,` / `Cmd ,` key binding (see `main.rs`'s `bind_keys`), and the title
    /// bar's settings gear (`views/title_bar.rs`) and the command palette call
    /// [`Self::open_settings`] directly.
    fn on_open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(window, cx);
    }

    /// Handles the `Ctrl K` / `Cmd K` key binding (see `main.rs`'s `bind_keys`); the title bar's
    /// search trigger (`views/title_bar.rs`) calls [`Self::open_command_palette`] directly.
    fn on_open_command_palette_action(
        &mut self,
        _: &OpenCommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_command_palette(window, cx);
    }

    /// Handles the `Ctrl O` / `Cmd O` key binding (see `main.rs`'s `bind_keys`): the same folder
    /// picker as the workspace switcher's "Open folder..." and the command palette's action.
    fn on_open_workspace_action(
        &mut self,
        _: &OpenWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        sidebar::pick_workspace_folder(cx.weak_entity(), window, cx);
    }

    /// Handles the `Ctrl+Shift+O` / `Cmd+Shift+O` key binding (see `main.rs`'s `bind_keys`): the
    /// OpenAPI import wizard, on its File source.
    fn on_import_openapi_action(
        &mut self,
        _: &ImportOpenApi,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::openapi_import::open_openapi_wizard(
            cx.weak_entity(),
            crate::state::openapi_import::SourceKind::File,
            window,
            cx,
        );
    }

    /// Handles the `F1` / `Cmd+Shift+/` key binding (see `main.rs`'s `bind_keys`).
    fn on_open_shortcuts_action(
        &mut self,
        _: &OpenShortcuts,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_shortcuts(window, cx);
    }

    /// Opens the keyboard shortcuts cheat sheet (telaio's), listing `state::shortcuts`. A no-op
    /// when a dialog is already open.
    pub(crate) fn open_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        open_shortcuts_dialog(state::shortcuts::groups(), window, cx);
    }

    /// Handles the `Ctrl+W` / `Cmd+W` key binding (see `main.rs`'s `bind_keys`). Like the tab's
    /// close button it asks first for an environment tab with unsaved edits, and like a tab
    /// switch it refocuses the view, because the closed tab's focused input vanishes.
    fn on_close_tab_action(
        &mut self,
        _: &CloseActiveTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.state.tabs.active_index() else {
            return;
        };
        let before = self.state.tabs.open_tabs().len();
        self.request_close_tab(index, window, cx);
        if self.state.tabs.open_tabs().len() != before {
            self.refocus_after_tab_switch(window, cx);
        }
    }

    /// Handles the `Ctrl+Tab` key binding (see `main.rs`'s `bind_keys`).
    fn on_next_tab_action(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.state.tabs.activate_next();
        self.refocus_after_tab_switch(window, cx);
    }

    /// Handles the `Ctrl+Shift+Tab` key binding (see `main.rs`'s `bind_keys`).
    fn on_previous_tab_action(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.tabs.activate_previous();
        self.refocus_after_tab_switch(window, cx);
    }

    /// Handles `Delete` while the sidebar tree has the focus (see `main.rs`'s `bind_keys`): asks
    /// to delete the selected request or folder, like its context menu's "Delete".
    fn on_delete_selected_entry(
        &mut self,
        _: &DeleteSelectedEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((id, name)) = self.selected_tree_entry(cx) {
            sidebar::open_delete_confirmation(cx.weak_entity(), id, name, window, cx);
        }
    }

    /// Handles `F2` while the sidebar tree has the focus (see `main.rs`'s `bind_keys`): opens the
    /// rename dialog of the selected request or folder, like its context menu's "Rename".
    fn on_rename_selected_entry(
        &mut self,
        _: &RenameSelectedEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((id, name)) = self.selected_tree_entry(cx) {
            sidebar::open_rename_dialog(cx.weak_entity(), id, name, window, cx);
        }
    }

    /// The id and label of the entry selected in the sidebar tree, if any.
    fn selected_tree_entry(&self, cx: &App) -> Option<(String, String)> {
        let item = self.tree_state.read(cx).selected_item()?;
        Some((item.id.to_string(), item.label.to_string()))
    }

    /// The tab switch replaces the editor's inputs, so a focused one would vanish and leave
    /// nothing focused, which kills every window-level shortcut (including the next `Ctrl+Tab`)
    /// until the next click. Focusing the view itself keeps them alive.
    fn refocus_after_tab_switch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Sets the active environment (`None` for "No environment") and remembers it for the open
    /// workspace, so reopening it restores the choice. Used by the environment picker's clicks
    /// and by [`Self::select_environment_by_shortcut`].
    pub(crate) fn select_environment(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        log::debug!("active environment: {}", name.as_deref().unwrap_or("none"));
        if let Some(workspace) = self.state.workspace.as_ref() {
            state::config::record_environment(workspace.root(), name.as_deref());
        }
        self.state.set_active_environment(name);
        cx.notify();
    }

    /// Selects the `one_based_index`-th environment (1..=9) of the open workspace, for the `Ctrl
    /// 1..9` / `Cmd 1..9` shortcuts. A no-op when there is no workspace open or fewer than
    /// `one_based_index` environments.
    pub(crate) fn select_environment_by_shortcut(
        &mut self,
        one_based_index: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.workspace.as_ref() else {
            return;
        };
        let environments = workspace.list_environments().unwrap_or_default();
        if let Some(name) = environments.into_iter().nth(one_based_index - 1) {
            self.select_environment(Some(name), cx);
        }
    }

    /// Handles the `Ctrl 1` / `Cmd 1` key binding.
    fn on_select_environment_1(
        &mut self,
        _: &SelectEnvironment1,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(1, cx);
    }

    /// Handles the `Ctrl 2` / `Cmd 2` key binding.
    fn on_select_environment_2(
        &mut self,
        _: &SelectEnvironment2,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(2, cx);
    }

    /// Handles the `Ctrl 3` / `Cmd 3` key binding.
    fn on_select_environment_3(
        &mut self,
        _: &SelectEnvironment3,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(3, cx);
    }

    /// Handles the `Ctrl 4` / `Cmd 4` key binding.
    fn on_select_environment_4(
        &mut self,
        _: &SelectEnvironment4,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(4, cx);
    }

    /// Handles the `Ctrl 5` / `Cmd 5` key binding.
    fn on_select_environment_5(
        &mut self,
        _: &SelectEnvironment5,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(5, cx);
    }

    /// Handles the `Ctrl 6` / `Cmd 6` key binding.
    fn on_select_environment_6(
        &mut self,
        _: &SelectEnvironment6,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(6, cx);
    }

    /// Handles the `Ctrl 7` / `Cmd 7` key binding.
    fn on_select_environment_7(
        &mut self,
        _: &SelectEnvironment7,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(7, cx);
    }

    /// Handles the `Ctrl 8` / `Cmd 8` key binding.
    fn on_select_environment_8(
        &mut self,
        _: &SelectEnvironment8,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(8, cx);
    }

    /// Handles the `Ctrl 9` / `Cmd 9` key binding.
    fn on_select_environment_9(
        &mut self,
        _: &SelectEnvironment9,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment_by_shortcut(9, cx);
    }

    /// `Alt Up`: moves the environment of the active environment tab one place up (#216).
    fn on_move_environment_up(
        &mut self,
        _: &MoveEnvironmentUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(name) = self
            .state
            .tabs
            .active_environment_name()
            .map(str::to_string)
        {
            self.move_environment(&name, true, cx);
        }
    }

    /// `Alt Down`: moves the environment of the active environment tab one place down.
    fn on_move_environment_down(
        &mut self,
        _: &MoveEnvironmentDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(name) = self
            .state
            .tabs
            .active_environment_name()
            .map(str::to_string)
        {
            self.move_environment(&name, false, cx);
        }
    }

    /// Handles the `Ctrl 0` / `Cmd 0` key binding ("No environment").
    fn on_select_no_environment(
        &mut self,
        _: &SelectNoEnvironment,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_environment(None, cx);
    }

    /// Handles a rail shortcut like a click on that rail button. A no-op without a workspace,
    /// when the rail is hidden.
    fn toggle_rail_section(
        &mut self,
        section: state::navigation::NavSection,
        cx: &mut Context<Self>,
    ) {
        if self.state.workspace.is_some() {
            self.click_rail(section, cx);
        }
    }

    /// Handles the `Alt 1` / `Ctrl 1` (macOS) key binding.
    fn on_toggle_collections(
        &mut self,
        _: &ToggleCollections,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_rail_section(state::navigation::NavSection::Collections, cx);
    }

    /// Handles the `Alt 2` / `Ctrl 2` (macOS) key binding.
    fn on_toggle_source_control(
        &mut self,
        _: &ToggleSourceControl,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_rail_section(state::navigation::NavSection::SourceControl, cx);
    }

    /// Handles the `Alt 3` / `Ctrl 3` (macOS) key binding.
    fn on_toggle_environments(
        &mut self,
        _: &ToggleEnvironments,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_rail_section(state::navigation::NavSection::Environments, cx);
    }

    /// Handles the `Alt 4` / `Ctrl 4` (macOS) key binding.
    fn on_toggle_load_tests(
        &mut self,
        _: &ToggleLoadTests,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_rail_section(state::navigation::NavSection::LoadTests, cx);
    }

    /// Renders the resizable sidebar and main area below the title bar.
    fn render_body(
        &mut self,
        weak: WeakEntity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // `ResizablePanelGroup` (`h_resizable`) sizes itself with `size_full()`, which only
        // resolves correctly if ITS OWN parent hands it a determinate height. The top-level
        // `Render` impl's `v_flex` used to add this `child(body)` with no `flex_1`/`min_h_0` of its
        // own, so `body` took its content's natural (unconstrained) height instead of "the window
        // height minus the title bar and status bar" - overflowing the window and pushing the
        // status bar (and, one level down, the sidebar footer, whose own already-correct
        // `flex_1`/`min_h_0` chain never got a real height to shrink within either) out of the
        // visible area. `min_h_0` is what actually allows this to shrink below its content size
        // instead of just growing; `flex_1` alone is not enough in a vertical flex chain.
        // With no workspace open nor being opened, the rail and the sidebar have nothing to show:
        // the main area takes the whole width, for the welcome tab (GitHub #90).
        let has_workspace = self.state.workspace.is_some() || self.opening.is_some();
        let sidebar_visible = has_workspace && self.nav.sidebar_visible();
        let sidebar = if sidebar_visible {
            self.render_side_panel(weak.clone(), cx)
        } else {
            div().into_any_element()
        };
        h_flex()
            .flex_1()
            .min_h_0()
            .when(has_workspace, |this| this.child(self.render_rail(cx)))
            .child(
                div().flex_1().min_w_0().h_full().child(
                    h_resizable("brasa-layout")
                        .child(
                            resizable_panel()
                                .visible(sidebar_visible)
                                .size(px(SIDEBAR_WIDTH))
                                .size_range(px(SIDEBAR_WIDTH_MIN)..px(SIDEBAR_WIDTH_MAX))
                                .flex_none()
                                .child(sidebar),
                        )
                        .child(resizable_panel().child(self.render_main_area(weak, window, cx))),
                ),
            )
            .into_any_element()
    }

    /// Renders the main area: the open-tabs bar on top, the request editor and response viewer
    /// split below it.
    fn render_main_area(
        &mut self,
        weak: WeakEntity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.debug_open == Some(DebugOpenTarget::Components) {
            return self.render_components_gallery(window, cx);
        }

        if self.state.tabs.open_tabs().is_empty()
            && self.state.workspace.is_none()
            && self.opening.is_none()
        {
            return self.render_no_workspace(weak, cx);
        }

        let tabs_bar = self.render_tabs_bar(weak, cx);
        let welcome = self
            .state
            .tabs
            .active()
            .and_then(|tab| tab.welcome())
            .cloned();
        let is_load_test = self
            .state
            .tabs
            .active()
            .is_some_and(|tab| tab.load_test().is_some());
        let is_environment = self
            .state
            .tabs
            .active()
            .is_some_and(|tab| tab.environment().is_some());
        let is_diff = self
            .state
            .tabs
            .active()
            .is_some_and(|tab| tab.diff().is_some());

        let is_log = self
            .state
            .tabs
            .active()
            .is_some_and(|tab| tab.log().is_some());

        let content = if let Some(welcome) = welcome {
            self.render_welcome(&welcome, cx)
        } else if self.state.tabs.is_whats_new_active() {
            self.render_whats_new(cx)
        } else if is_load_test {
            self.render_load_test_tab(window, cx)
        } else if is_environment {
            self.render_env_tab(window, cx)
        } else if is_diff {
            self.render_diff_tab(window, cx)
        } else if is_log {
            self.render_log_tab(window, cx)
        } else {
            v_resizable("brasa-main")
                .child(
                    resizable_panel()
                        // 1.1: 1 initial split.
                        .size(px(462.0))
                        .child(self.render_request_editor(window, cx)),
                )
                .child(
                    resizable_panel()
                        .size(px(420.0))
                        .child(self.render_response_view(window, cx)),
                )
                .into_any_element()
        };

        v_flex()
            .size_full()
            .child(tabs_bar)
            // `flex_1().min_h_0()` here, not just on `content` itself: a plain flex child does
            // not grow to fill remaining space or shrink below its content's natural height on
            // its own (the same reasoning `render_body`'s own `div().flex_1().min_h_0()` already
            // documents for `h_resizable`). The non-load-test `content` (`v_resizable`) happens
            // to manage its own sizing regardless, but the load test tab's dashboard is a plain
            // `overflow_y_scroll()` `v_flex`, which needs a genuinely bounded parent height for
            // that scroll to actually clip instead of letting the pane grow past the window and
            // paint its scrolled content over the title bar. `overflow_hidden()` here too: once
            // scrolled, the dashboard's own content mask was still letting scrolled-past rows
            // paint above this pane (verified by screenshot while fixing this same issue), so
            // this outer clip is the actual backstop that keeps them inside the tab.
            .child(div().flex_1().min_h_0().overflow_hidden().child(content))
            .into_any_element()
    }

    /// Renders the open-tabs bar: a [`DocumentTab`] per open request, with a dirty marker and a
    /// close button, and a trailing "+" that opens the same new-request dialog as the sidebar
    /// header.
    fn render_tabs_bar(&mut self, weak: WeakEntity<Self>, cx: &mut Context<Self>) -> AnyElement {
        if self.state.tabs.open_tabs().is_empty() {
            return div().into_any_element();
        }

        let active_index = self.state.tabs.active_index();
        let reorder_weak = weak.clone();
        let mut tabs = DocumentTabs::new("open-tabs").on_reorder(move |from, to, _, cx| {
            let _ = reorder_weak.update(cx, |view, cx| {
                log::debug!("moved tab {from} to {to}");
                view.state.tabs.move_tab(from, to);
                cx.notify();
            });
        });
        let total_tabs = self.state.tabs.open_tabs().len();
        for (index, tab) in self.state.tabs.open_tabs().iter().enumerate() {
            let select_weak = weak.clone();
            let close_weak = weak.clone();
            let context_weak = weak.clone();
            let full_id = match &tab.kind {
                state::TabKind::Environment(edit) => state::env_edit::status_path(&edit.name),
                state::TabKind::Welcome(_) => t!("welcome.tab").into_owned(),
                state::TabKind::Diff(diff) => diff.path.clone(),
                state::TabKind::Log(_) => t!("git_log.tab").into_owned(),
                state::TabKind::WhatsNew => t!("whats_new.tab").into_owned(),
                state::TabKind::Request(_) | state::TabKind::LoadTest(_) => tab.id.clone(),
            };
            let mut doc_tab = match &tab.kind {
                state::TabKind::Request(request) => {
                    let method = request.method.clone();
                    DocumentTab::new(state::format::tab_label(&tab.id).to_string())
                        .prefix(move || MethodBadge::inline(method.clone()))
                }
                state::TabKind::Environment(edit) => {
                    DocumentTab::new(state::env_edit::tab_label(&edit.name))
                        .icon(gpui_kit::assets::IconName::Variable)
                }
                state::TabKind::LoadTest(load_test) => {
                    DocumentTab::new(crate::views::load_test::tab_label(load_test))
                        .icon(gpui_kit::assets::IconName::Gauge)
                }
                state::TabKind::Diff(diff) => DocumentTab::new(diff.label().to_string())
                    .icon(gpui_kit::assets::IconName::GitCompare)
                    .detail(diff.target.detail()),
                state::TabKind::Log(log) => {
                    let log_tab = DocumentTab::new(t!("git_log.tab"))
                        .icon(gpui_kit::assets::IconName::GitGraph);
                    match log.filter.branch.clone() {
                        Some(branch) => log_tab.detail(branch),
                        None => log_tab,
                    }
                }
                state::TabKind::Welcome(_) => DocumentTab::new(t!("welcome.tab"))
                    .icon(gpui_kit::assets::IconName::House)
                    .icon_color(cx.palette().accent_text),
                state::TabKind::WhatsNew => DocumentTab::new(t!("whats_new.tab"))
                    .icon(gpui_kit::assets::IconName::Sparkles)
                    .icon_color(cx.palette().accent_text),
            };
            doc_tab = doc_tab
                .dirty(tab.dirty)
                .selected(active_index == Some(index))
                .tooltip(full_id)
                .on_click(move |_, cx| {
                    let _ = select_weak.update(cx, |view, cx| {
                        view.state.tabs.set_active(index);
                        cx.notify();
                    });
                })
                .on_close(move |window, cx| {
                    let _ =
                        close_weak.update(cx, |view, cx| view.request_close_tab(index, window, cx));
                })
                .context_menu(move |menu, window, cx| {
                    build_tab_context_menu(context_weak.clone(), index, total_tabs, menu, window, cx)
                });
            tabs = tabs.item(doc_tab);
        }

        // A new request needs a workspace: next to the welcome tab alone there is none.
        if self.state.workspace.is_none() {
            return tabs.into_any_element();
        }
        // A large hover area around a small icon (an `IconButton`'s large size also scales its
        // icon up); the bar pads it off the last tab.
        let palette = cx.palette();
        let tooltip = SharedString::from(t!("common.new_request"));
        tabs = tabs.suffix(
            div()
                .id("open-tabs-new-request")
                .size(px(ICON_BUTTON_LG))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .rounded(px(RADIUS_SM))
                .text_color(palette.fg_muted)
                .hover(|style| style.bg(palette.hover))
                .active(|style| style.bg(palette.pressed))
                .child(Icon::new(IconName::Plus).small())
                .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                .on_click(move |_, window, cx| {
                    sidebar::open_new_request_dialog(weak.clone(), None, window, cx);
                }),
        );
        tabs.into_any_element()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_count += 1;
        log::trace!("render #{}", self.render_count);

        // Follow the sidebar's selection: opening a request tab happens here, once per actual
        // selection change, rather than from a click handler on every tree row. Guarding on
        // `last_selected_request` keeps a re-render from reloading the file (and discarding
        // in-progress edits) just because the same row is still selected.
        let selected_id = self
            .tree_state
            .read(cx)
            .selected_item()
            .map(|item| item.id.to_string());
        if let Some(id) = selected_id
            && id.ends_with(".brasa")
            && self.last_selected_request.as_deref() != Some(id.as_str())
        {
            self.last_selected_request = Some(id.clone());
            self.open_request(id, cx);
        }

        let weak = cx.weak_entity();
        let background = cx.theme().background;

        let title_bar = self.render_title_bar(weak.clone(), window, cx);
        let error_banner = self.workspace_error.clone().map(|message| {
            let dismiss_weak = weak.clone();
            div().px_3().pt_2().child(
                InlineMessage::new(InlineMessageKind::Danger, message).action(
                    t!("common.dismiss"),
                    move |_, cx| {
                        let _ = dismiss_weak.update(cx, |view, cx| {
                            view.workspace_error = None;
                            cx.notify();
                        });
                    },
                ),
            )
        });
        let body = self.render_body(weak, window, cx);
        let status_bar = self.render_status_bar(cx);
        let completion = self.render_completion(cx);

        v_flex()
            .size_full()
            .bg(background)
            .track_focus(&self.focus_handle)
            // A file dropped anywhere opens the import wizard (design Import Flow, 4e).
            .on_drop({
                // Not a listener: the wizard reads `AppView` while it opens.
                let view = cx.weak_entity();
                move |paths: &ExternalPaths, window, cx| {
                    let paths = paths.paths().to_vec();
                    if !paths.is_empty() {
                        super::openapi_import::open_wizard_with_files(
                            view.clone(),
                            paths,
                            window,
                            cx,
                        );
                    }
                }
            })
            .on_action(cx.listener(Self::on_save_action))
            .on_action(cx.listener(Self::on_copy_load_test_summary_action))
            .on_action(cx.listener(Self::on_send_action))
            .on_action(cx.listener(Self::on_open_settings_action))
            .on_action(cx.listener(Self::on_open_command_palette_action))
            .on_action(cx.listener(Self::on_open_workspace_action))
            .on_action(cx.listener(Self::on_import_openapi_action))
            .on_action(cx.listener(Self::on_open_shortcuts_action))
            .on_action(cx.listener(Self::on_close_tab_action))
            .on_action(cx.listener(Self::on_next_tab_action))
            .on_action(cx.listener(Self::on_previous_tab_action))
            .on_action(cx.listener(Self::on_delete_selected_entry))
            .on_action(cx.listener(Self::on_rename_selected_entry))
            .on_action(cx.listener(Self::on_select_environment_1))
            .on_action(cx.listener(Self::on_select_environment_2))
            .on_action(cx.listener(Self::on_select_environment_3))
            .on_action(cx.listener(Self::on_select_environment_4))
            .on_action(cx.listener(Self::on_select_environment_5))
            .on_action(cx.listener(Self::on_select_environment_6))
            .on_action(cx.listener(Self::on_select_environment_7))
            .on_action(cx.listener(Self::on_select_environment_8))
            .on_action(cx.listener(Self::on_select_environment_9))
            .on_action(cx.listener(Self::on_select_no_environment))
            .on_action(cx.listener(Self::on_move_environment_up))
            .on_action(cx.listener(Self::on_move_environment_down))
            .on_action(cx.listener(Self::on_toggle_collections))
            .on_action(cx.listener(Self::on_toggle_source_control))
            .on_action(cx.listener(Self::on_toggle_environments))
            .on_action(cx.listener(Self::on_toggle_load_tests))
            .on_action(cx.listener(Self::on_show_completion))
            .on_action(cx.listener(Self::on_next_completion))
            .on_action(cx.listener(Self::on_previous_completion))
            .on_action(cx.listener(Self::on_confirm_completion))
            .on_action(cx.listener(Self::on_dismiss_completion))
            .on_action(cx.listener(Self::on_delete_line))
            .on_action(cx.listener(Self::on_undo_request_edit))
            .on_action(cx.listener(Self::on_redo_request_edit))
            .child(title_bar)
            .children(error_banner)
            .child(body)
            .child(status_bar)
            .children(completion)
            // The modal backdrop, see `Palette::scrim`. Paints only, so clicks still reach the
            // dialog's own (transparent) backdrop above it, which closes the dialog.
            .relative()
            .when(window.has_active_dialog(cx), |this| {
                this.child(div().absolute().inset_0().bg(cx.palette().scrim))
            })
            .children(self.toast.as_ref().map(|toast| render_toast(toast, cx)))
    }
}

/// Renders the active toast: a small pill, bottom right, clear of the status bar. Fades and
/// rises into place once when it first paints, keyed to the toast's own generation so showing a
/// new one always replays the entrance, the same technique as the welcome tab's
/// (`views/welcome.rs`'s `fade_rise`). No further frames once it settles.
/// The open tab's right-click menu: Close, then (only the ones that would actually close
/// something) Close others, Close tabs to the right, Close tabs to the left, then Close all.
/// `index` and `total` are read when the row renders, not when the menu opens, matching how the
/// sidebar's own context menu already works: fast enough that the tab list cannot have changed
/// in between on a single click.
fn build_tab_context_menu(
    weak: WeakEntity<AppView>,
    index: usize,
    total: usize,
    mut menu: PopupMenu,
    _window: &mut Window,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let close_weak = weak.clone();
    menu = menu.item(
        PopupMenuItem::new(t!("shell.tabs.close")).on_click(move |_, window, cx| {
            let _ = close_weak.update(cx, |view, cx| view.request_close_tab(index, window, cx));
        }),
    );

    let has_others = total > 1;
    let has_right = index + 1 < total;
    let has_left = index > 0;
    if has_others || has_right || has_left {
        menu = menu.separator();
    }
    if has_others {
        let weak = weak.clone();
        menu = menu.item(
            PopupMenuItem::new(t!("shell.tabs.close_others")).on_click(move |_, window, cx| {
                let indices: Vec<usize> = (0..total).filter(|&i| i != index).collect();
                let _ = weak.update(cx, |view, cx| view.close_tabs(indices, window, cx));
            }),
        );
    }
    if has_right {
        let weak = weak.clone();
        menu = menu.item(
            PopupMenuItem::new(t!("shell.tabs.close_right")).on_click(move |_, window, cx| {
                let indices: Vec<usize> = (index + 1..total).collect();
                let _ = weak.update(cx, |view, cx| view.close_tabs(indices, window, cx));
            }),
        );
    }
    if has_left {
        let weak = weak.clone();
        menu = menu.item(
            PopupMenuItem::new(t!("shell.tabs.close_left")).on_click(move |_, window, cx| {
                let indices: Vec<usize> = (0..index).collect();
                let _ = weak.update(cx, |view, cx| view.close_tabs(indices, window, cx));
            }),
        );
    }
    if total > 1 {
        menu = menu.separator();
        menu = menu.item(
            PopupMenuItem::new(t!("shell.tabs.close_all")).on_click(move |_, window, cx| {
                let indices: Vec<usize> = (0..total).collect();
                let _ = weak.update(cx, |view, cx| view.close_tabs(indices, window, cx));
            }),
        );
    }
    menu
}

fn render_toast(toast: &Toast, cx: &App) -> AnyElement {
    let palette = cx.palette();
    div()
        .absolute()
        .right(px(16.0))
        .bottom(px(16.0 + STATUS_BAR_HEIGHT))
        .child(
            h_flex()
                .relative()
                .items_center()
                .gap(px(8.0))
                .h(px(36.0))
                .px(px(14.0))
                .rounded(px(RADIUS_MD))
                .bg(palette.overlay)
                .border_1()
                .border_color(palette.border)
                .shadow(palette.shadow.clone())
                .text_size(px(13.0))
                .text_color(palette.fg)
                .child(
                    Icon::new(IconName::Check)
                        .small()
                        .text_color(palette.success),
                )
                .child(toast.message.clone())
                .with_animation(
                    ("toast-pill", toast.generation),
                    Animation::new(Duration::from_millis(260)).with_easing(ease_in_out),
                    |this, delta| this.opacity(delta).top(px((1.0 - delta) * 8.0)),
                ),
        )
        .into_any_element()
}
