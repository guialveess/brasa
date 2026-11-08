//! The "New workspace" dialog (GitHub #88): a name, a location with a "Change..." button that
//! opens the native folder picker, and the full path that will be created. Create makes the
//! folder with `brasa_workspace::create_workspace`, with the example request and environment,
//! and opens it as the workspace. Opened from the workspace switcher menu, the command palette
//! and the welcome tab. An import creates its workspace inside the wizard instead
//! (`views/import_destination.rs`).
//!
//! An in-app dialog, never a native "save" one: those are meant for files and behave differently
//! between platforms when they create a folder.

use std::path::{Path, PathBuf};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_workspace::{
    EXAMPLE_ENVIRONMENT, EXAMPLE_REQUEST_ID, GITIGNORE, NewWorkspaceContent, WorkspaceError,
    create_workspace,
};
use telaio_ui::components::labelled_checkbox;

use crate::state;
use crate::state::workspace_log::log_workspace_error;
use crate::theme::PaletteExt;
use crate::theme::metrics::RADIUS_MD;
use crate::views::components::{
    InlineMessage, InlineMessageKind, ModalPlacement, PrimaryButton, SecondaryButton,
    framed_dialog, modal_body, modal_footer, modal_header, text_field,
};
use telaio_ui::components::text_field::FIELD_HEIGHT;

use super::root::AppView;

/// Width of the dialog.
const DIALOG_WIDTH: f32 = 480.0;

/// Longest path, in characters, the dialog shows. Longer ones get their middle elided, so the
/// folder being created stays visible at the end.
const PATH_MAX_CHARS: usize = 46;

/// The dialog's state. Owned by the dialog's builder closures, so it ends with the dialog.
struct NewWorkspaceForm {
    /// The folder name the user types.
    name: Entity<InputState>,
    /// The folder the new workspace goes in, the Documents folder until "Change..." picks
    /// another one.
    location: PathBuf,
    /// Why the last Create failed, until the name or the location changes.
    error: Option<String>,
    /// The git Brasa detected, `None` when it is not available (design 7c).
    git: Option<telaio_git::Git>,
    /// Whether to run `git init` in the new folder. On by default when git is available.
    init_git: bool,
    /// Whether the location is already inside a git repository, which hides the checkbox.
    inside_repo: bool,
}

/// Opens the dialog.
pub(crate) fn open_new_workspace_dialog(
    view: WeakEntity<AppView>,
    window: &mut Window,
    cx: &mut App,
) {
    let name = cx.new(|cx| {
        InputState::new(window, cx).placeholder(t!("shell.new_workspace.name_placeholder"))
    });
    let git = view
        .upgrade()
        .and_then(|view| view.read(cx).git.availability.git().cloned());
    let location = state::new_workspace::default_location_for_user();
    let form = cx.new(|_| NewWorkspaceForm {
        name: name.clone(),
        inside_repo: is_inside_repo(&location),
        location,
        error: None,
        init_git: git.is_some(),
        git,
    });
    {
        // Weak, or the subscription would keep the form, and through it the input, alive forever.
        let form = form.downgrade();
        window
            .subscribe(&name, cx, move |_, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let _ = form.update(cx, |form, cx| {
                        if form.error.take().is_some() {
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
    }
    window.open_dialog(cx, move |dialog, window, _| {
        let enter_form = form.clone();
        let enter_view = view.clone();
        // Enter anywhere in the dialog, the name field included, is the dialog's own Confirm
        // action. Returning `false` keeps the dialog open to show why Create failed.
        let dialog =
            dialog.on_ok(move |_, window, cx| confirm(enter_view.clone(), &enter_form, window, cx));
        let form = form.clone();
        let view = view.clone();
        framed_dialog(
            dialog,
            DIALOG_WIDTH,
            None,
            ModalPlacement::DEFAULT,
            window,
            move |_, cx| {
                let can_create = target_of(&form, cx).is_some();
                let create_form = form.clone();
                let create_view = view.clone();
                v_flex()
                    .child(modal_header(t!("shell.new_workspace.title"), cx))
                    .child(modal_body().child(render_form(&form, &view, cx)))
                    .child(
                        modal_footer(cx)
                            .child(
                                SecondaryButton::new("new-workspace-cancel", t!("common.cancel"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                PrimaryButton::new("new-workspace-create", t!("common.create"))
                                    .disabled(!can_create)
                                    .on_click(move |_, window, cx| {
                                        if confirm(create_view.clone(), &create_form, window, cx) {
                                            window.close_dialog(cx);
                                        }
                                    }),
                            ),
                    )
                    .into_any_element()
            },
        )
    });
    // Focus only after `open_dialog`, which captures the focused handle to restore on close
    // (see `views/command_palette.rs`).
    name.update(cx, |state, cx| state.focus(window, cx));
}

/// The folder the form's name creates in its location, `None` while the name is blank.
fn target_of(form: &Entity<NewWorkspaceForm>, cx: &App) -> Option<PathBuf> {
    let form = form.read(cx);
    state::new_workspace::target(&form.location, &form.name.read(cx).value())
}

/// The dialog's body: a short note, the name field, the location with its "Change..." button,
/// the path that will be created and the last error.
fn render_form(
    form: &Entity<NewWorkspaceForm>,
    view: &WeakEntity<AppView>,
    cx: &App,
) -> AnyElement {
    let palette = cx.palette();
    let mono_font = cx.theme().mono_font_family.clone();
    let home = dirs::home_dir();
    let target = target_of(form, cx);
    let state = form.read(cx);
    let shown = |path: &Path| {
        state::format::elide_path(
            &state::format::shorten_path(path, home.as_deref()),
            PATH_MAX_CHARS,
        )
    };
    let location = shown(&state.location);
    let change_form = form.downgrade();
    let note = t!("shell.new_workspace.example_note");

    let label = |text: SharedString| {
        div()
            .text_size(px(12.0))
            .text_color(palette.fg_muted)
            .child(text)
    };

    v_flex()
        .gap(px(14.0))
        .child(div().text_color(palette.fg_muted).child(note))
        .child(
            v_flex()
                .gap(px(6.0))
                .child(label(t!("shell.new_workspace.name").into()))
                .child(text_field(&state.name, cx)),
        )
        .child(
            v_flex()
                .gap(px(6.0))
                .child(label(t!("shell.new_workspace.location").into()))
                .child(
                    h_flex()
                        .gap(px(8.0))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(FIELD_HEIGHT))
                                .flex()
                                .items_center()
                                .px(px(10.0))
                                .rounded(px(RADIUS_MD))
                                .bg(palette.surface)
                                .border_1()
                                .border_color(palette.border)
                                .font_family(mono_font.clone())
                                .text_size(px(12.0))
                                .text_color(palette.fg_muted)
                                .child(div().truncate().child(location)),
                        )
                        .child(
                            SecondaryButton::new(
                                "new-workspace-change",
                                t!("shell.new_workspace.change"),
                            )
                            .height(FIELD_HEIGHT)
                            .on_click(move |_, _, cx| pick_location(change_form.clone(), cx)),
                        ),
                ),
        )
        .children(target.map(|target| {
            h_flex()
                .gap(px(6.0))
                .text_size(px(12.0))
                .text_color(palette.fg_subtle)
                .child(div().flex_none().child(t!("shell.new_workspace.target")))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(mono_font)
                        .child(shown(&target)),
                )
        }))
        .children((!state.inside_repo).then(|| render_git_option(form, view, cx)))
        .children(
            state
                .error
                .clone()
                .map(|error| InlineMessage::new(InlineMessageKind::Danger, error)),
        )
        .into_any_element()
}

/// Opens the native folder picker and, once the user picks a folder, makes it the location.
fn pick_location(form: WeakEntity<NewWorkspaceForm>, cx: &mut App) {
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
        let Some(location) = paths.pop() else {
            return;
        };
        let inside_repo = is_inside_repo(&location);
        let _ = form.update(cx, |form, cx| {
            form.inside_repo = inside_repo;
            form.location = location;
            form.error = None;
            cx.notify();
        });
    })
    .detach();
}

/// Creates the folder and opens it. Returns whether the dialog can close: a target that cannot
/// be created keeps it open with the reason.
fn confirm(
    view: WeakEntity<AppView>,
    form: &Entity<NewWorkspaceForm>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let Some(target) = target_of(form, cx) else {
        return false;
    };
    if let Err(error) = create_workspace(&target, NewWorkspaceContent::Example) {
        log_workspace_error("create workspace", &error);
        // The path is already on screen, in the "Creates" line.
        let message = match error {
            WorkspaceError::NotEmpty(_) => t!("shell.new_workspace.not_empty"),
            error => t!("shell.new_workspace.failed", error = error.to_string()),
        };
        form.update(cx, |form, cx| {
            form.error = Some(message.into_owned());
            cx.notify();
        });
        return false;
    }
    let (git, init_git, inside_repo) = {
        let form = form.read(cx);
        (form.git.clone(), form.init_git, form.inside_repo)
    };
    if let (Some(git), true, false) = (git, init_git, inside_repo) {
        // `git init` takes a few milliseconds, and the workspace must be a repository before it
        // opens, so the Source Control panel finds it. A failure keeps the workspace, without git.
        if let Err(error) = crate::views::git::init_with_gitignore(&git, &target) {
            log::warn!("git init in the new workspace failed: {error}");
        }
    }
    let _ = view.update(cx, |view, cx| {
        view.open_new_workspace(target, window, cx);
    });
    true
}

impl AppView {
    /// Opens the workspace the dialog just created, then selects its environment and opens its
    /// request, so the first Send works without editing anything.
    fn open_new_workspace(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        log::info!("created workspace {}", root.display());
        self.open_workspace_at(&root, window, cx);
        self.when_workspace_opens(window, cx, |view, _, cx| {
            view.select_environment(Some(EXAMPLE_ENVIRONMENT.to_string()), cx);
            view.open_request(EXAMPLE_REQUEST_ID.to_string(), cx);
        });
    }
}

/// Whether `location`, or a folder above it, is a git work tree: a `.git` folder or file there.
/// A few metadata calls, only when the location changes.
fn is_inside_repo(location: &Path) -> bool {
    location
        .ancestors()
        .any(|folder| folder.join(".git").exists())
}

/// The "Initialize a Git repository" checkbox with the `.gitignore` it adds, or, without git, the
/// same checkbox disabled with a link to the Git settings.
fn render_git_option(
    form: &Entity<NewWorkspaceForm>,
    view: &WeakEntity<AppView>,
    cx: &App,
) -> AnyElement {
    let palette = cx.palette();
    let state = form.read(cx);
    let available = state.git.is_some();
    let toggle_form = form.downgrade();
    let checkbox = labelled_checkbox(
        "new-workspace-init-git",
        available && state.init_git,
        t!("shell.new_workspace.init_git"),
        &palette,
    )
    .disabled(!available)
    .on_change(move |checked, _, cx| {
        let checked = *checked;
        let _ = toggle_form.update(cx, |form, cx| {
            form.init_git = checked;
            cx.notify();
        });
    });
    let mut column = v_flex().gap(px(6.0)).child(checkbox);
    if available {
        if state.init_git {
            column = column
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.fg_subtle)
                        .child(t!("shell.new_workspace.init_git_note")),
                )
                .child(
                    div()
                        .px(px(10.0))
                        .py(px(8.0))
                        .rounded(px(RADIUS_MD))
                        .bg(palette.surface)
                        .border_1()
                        .border_color(palette.border)
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_size(px(11.5))
                        .text_color(palette.fg_muted)
                        .children(GITIGNORE.lines().map(|line| div().child(line.to_string()))),
                );
        }
    } else {
        let view = view.clone();
        column = column.child(
            h_flex()
                .gap(px(6.0))
                .text_size(px(12.0))
                .text_color(palette.fg_subtle)
                .child(t!("shell.new_workspace.git_missing"))
                .child(
                    div()
                        .id("new-workspace-git-settings")
                        .cursor_pointer()
                        .text_color(palette.accent_text)
                        .hover(|style| style.underline())
                        .child(t!("git.empty.open_settings"))
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let _ = view.update(cx, |view, cx| view.open_git_settings(window, cx));
                        }),
                ),
        );
    }
    column.into_any_element()
}
