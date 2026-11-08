//! The "Destination" block of the import wizard's Options step (GitHub #208, Claude Design
//! "Import Flow", 3a to 3c): the open workspace, another recent one, a new one or a picked folder.
//! The choice and the checks live in `state/import_destination.rs`; nothing is written until the
//! wizard's last step.

use std::fs;
use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_workspace::{NewWorkspaceContent, Workspace, WorkspaceError, create_workspace};
use telaio_ui::components::text_field::FIELD_HEIGHT;
use telaio_ui::components::{labelled_checkbox, labelled_radio};

use crate::state::format::{elide_path, shorten_path};
use crate::state::import_destination::{
    Choice, Destination, FolderCheck, Target, default_location,
};
use crate::state::locale::plural;
use crate::state::workspace_log::log_workspace_error;
use crate::theme::Palette;
use crate::theme::metrics::RADIUS_MD;
use crate::views::components::{
    InlineMessage, InlineMessageKind, NeutralButtonExt, SecondaryButton, SectionLabel, text_field,
};

use super::root::AppView;

/// Width of the workspace name field, from the design.
const NAME_WIDTH: f32 = 200.0;
/// Height of a pill of the first use layout.
const PILL_HEIGHT: f32 = 28.0;
/// Longest path, in characters, in a field of the block.
const PATH_MAX_CHARS: usize = 44;
/// Longest path, in characters, next to a radio of the block.
const ROW_PATH_MAX_CHARS: usize = 32;

/// The block's entity: the plain state, the name input and the Git option of a new workspace.
pub(crate) struct DestinationBlock {
    state: Destination,
    /// The new workspace's name.
    name: Entity<InputState>,
    /// The last name the block filled in. Another value means the user typed one.
    suggested: String,
    /// The git Brasa detected.
    git: Option<telaio_git::Git>,
    /// Whether to run `git init` in a new workspace. On by default when git is available.
    init_git: bool,
    /// Whether the new workspace's location already is inside a Git repository.
    inside_repo: bool,
    /// The folder the import reads from, offered as the workspace (design, section 6).
    source_folder: Option<PathBuf>,
}

/// What the wizard runs with once the user confirms.
#[derive(Clone)]
pub(crate) struct ResolvedDestination {
    /// Where the import goes.
    pub(crate) target: Target,
    /// The git to initialize a created workspace with.
    pub(crate) init_git: Option<telaio_git::Git>,
}

impl DestinationBlock {
    /// Builds the block, reading the workspace and the recent list from `view`.
    pub(crate) fn build(
        view: &WeakEntity<AppView>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let (current, recent, git) = view.upgrade().map_or((None, Vec::new(), None), |view| {
            let view = view.read(cx);
            (
                view.state
                    .workspace
                    .as_ref()
                    .map(|workspace| workspace.root().to_path_buf()),
                view.recent_workspaces.clone(),
                view.git.availability.git().cloned(),
            )
        });
        let location = default_location();
        let inside_repo = is_inside_repo(&location);
        let name = cx.new(|cx| InputState::new(window, cx));
        let block = cx.new(|_| DestinationBlock {
            state: Destination::new(current, &recent, location),
            name: name.clone(),
            suggested: String::new(),
            init_git: git.is_some(),
            git,
            inside_repo,
            source_folder: None,
        });
        let weak = block.downgrade();
        window
            .subscribe(&name, cx, move |_, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let _ = weak.update(cx, |block, cx| block.refresh(cx));
                }
            })
            .detach();
        block
    }

    /// Fills the name in from the source's title, unless the user typed one already.
    pub(crate) fn suggest_name(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let typed = self.name.read(cx).value().to_string();
        if !typed.is_empty() && typed != self.suggested {
            return;
        }
        self.suggested = name.clone();
        self.name
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.refresh(cx);
    }

    /// The folder the import reads from.
    pub(crate) fn source_folder(&self) -> Option<&PathBuf> {
        self.source_folder.as_ref()
    }

    /// Sets the folder the import reads from, `None` when it reads from something else.
    pub(crate) fn set_source_folder(&mut self, folder: Option<PathBuf>, cx: &mut Context<Self>) {
        self.source_folder = folder;
        cx.notify();
    }

    /// The resolved destination, `None` while something is missing.
    pub(crate) fn resolved(&self, cx: &App) -> Option<ResolvedDestination> {
        let target = self.state.target(&self.name.read(cx).value())?;
        let init_git = (matches!(target, Target::Create(_)) && self.init_git && !self.inside_repo)
            .then(|| self.git.clone())
            .flatten();
        Some(ResolvedDestination { target, init_git })
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        self.state.refresh(&name);
        cx.notify();
    }

    fn select(&mut self, choice: Choice, cx: &mut Context<Self>) {
        self.state.choice = choice;
        self.refresh(cx);
    }
}

/// Opens the native folder picker, then hands the folder to `apply`.
fn pick_folder(
    block: WeakEntity<DestinationBlock>,
    apply: impl FnOnce(&mut DestinationBlock, PathBuf) + 'static,
    cx: &mut App,
) {
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
        let _ = block.update(cx, |block, cx| {
            apply(block, path);
            block.refresh(cx);
        });
    })
    .detach();
}

/// The location picker of a new workspace.
fn pick_location(block: WeakEntity<DestinationBlock>, cx: &mut App) {
    pick_folder(
        block,
        |block, path| {
            block.inside_repo = is_inside_repo(&path);
            block.state.location = path;
        },
        cx,
    );
}

/// The picker of a folder to use as it is.
fn pick_existing(block: WeakEntity<DestinationBlock>, cx: &mut App) {
    pick_folder(
        block,
        |block, path| {
            block.state.existing = Some(path);
            block.state.choice = Choice::Existing;
        },
        cx,
    );
}

/// Renders the block. `collection` is the collection folder's name, for the "added as" line, empty
/// when the import writes only environments.
pub(crate) fn render_destination(
    block: &Entity<DestinationBlock>,
    collection: &str,
    palette: &Palette,
    cx: &App,
) -> AnyElement {
    let state = &block.read(cx).state;
    let content = if state.current.is_some() {
        render_with_workspace(block, collection, palette, cx)
    } else {
        render_first_use(block, palette, cx)
    };
    v_flex()
        .gap(px(10.0))
        .px(px(16.0))
        .py(px(14.0))
        .rounded(px(10.0))
        .border_1()
        .border_color(palette.border)
        .bg(palette.surface)
        .child(SectionLabel::new(t!("import.dest.title")))
        .child(content)
        .children(render_use_source(block, cx))
        .children(render_notice(block, palette, cx))
        .into_any_element()
}

/// The button that makes the folder the import reads from the workspace, instead of a new one.
/// Clicking it answers the notice about its files.
fn render_use_source(block: &Entity<DestinationBlock>, cx: &App) -> Option<AnyElement> {
    let this = block.read(cx);
    let folder = this.source_folder.clone()?;
    if this.state.choice == Choice::Existing && this.state.existing.as_ref() == Some(&folder) {
        return None;
    }
    let weak = block.downgrade();
    Some(
        h_flex()
            .child(
                SecondaryButton::new("import-dest-use-source", t!("import.dest.use_source"))
                    .on_click(move |_, _, cx| {
                        let folder = folder.clone();
                        let _ = weak.update(cx, |block, cx| {
                            block.state.choice = Choice::Existing;
                            block.state.existing = Some(folder);
                            block.state.accept("");
                            block.refresh(cx);
                        });
                    }),
            )
            .into_any_element(),
    )
}

/// Design 3a: no workspace is open, so a new one is proposed.
fn render_first_use(block: &Entity<DestinationBlock>, palette: &Palette, cx: &App) -> AnyElement {
    let state = &block.read(cx).state;
    let is_new = state.choice == Choice::New;
    let new_block = block.downgrade();
    let open_block = block.downgrade();
    v_flex()
        .gap(px(10.0))
        .child(
            h_flex()
                .items_start()
                .gap(px(10.0))
                .px(px(12.0))
                .py(px(9.0))
                .rounded(px(8.0))
                .bg(palette.accent_subtle)
                .child(
                    Icon::new(IconName::Info)
                        .size(px(15.0))
                        .text_color(palette.accent_text),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .line_height(relative(1.45))
                        .child(t!("import.dest.no_workspace")),
                ),
        )
        .child(
            h_flex()
                .gap(px(6.0))
                .child(
                    pill(
                        "import-dest-new",
                        IconName::FolderPlus,
                        t!("import.dest.new").into(),
                        is_new,
                        palette,
                    )
                    .on_click(move |_, _, cx| {
                        let _ = new_block.update(cx, |block, cx| block.select(Choice::New, cx));
                    }),
                )
                .child(
                    pill(
                        "import-dest-open",
                        IconName::FolderOpen,
                        t!("import.dest.open_existing").into(),
                        !is_new,
                        palette,
                    )
                    .on_click(move |_, _, cx| pick_existing(open_block.clone(), cx)),
                ),
        )
        .child(if is_new {
            render_new_fields(block, palette, cx)
        } else {
            render_existing_folder(block, palette, cx)
        })
        .into_any_element()
}

/// Design 3b: the open workspace is the default, another one can be picked.
fn render_with_workspace(
    block: &Entity<DestinationBlock>,
    collection: &str,
    palette: &Palette,
    cx: &App,
) -> AnyElement {
    let state = &block.read(cx).state;
    let home = dirs::home_dir();
    let current = state.current.as_deref().unwrap_or(Path::new(""));
    let shown = |path: &Path| elide_path(&shorten_path(path, home.as_deref()), ROW_PATH_MAX_CHARS);
    let mono = cx.theme().mono_font_family.clone();
    let radio = |id: &'static str, label: String, choice: Choice| {
        let weak = block.downgrade();
        labelled_radio(id, state.choice == choice, label, palette).on_change(move |_, _, cx| {
            let _ = weak.update(cx, |block, cx| block.select(choice, cx));
        })
    };
    let side_text = |text: String| {
        div()
            .flex_none()
            .font_family(mono.clone())
            .text_size(px(12.0))
            .text_color(palette.fg_subtle)
            .child(text)
    };
    let row = || h_flex().gap(px(10.0)).py(px(5.0));
    let workspace_name = folder_name(current);
    let recent_label = recent_label_for(state);
    let recent_menu = !state.recent.is_empty();
    let menu_block = block.downgrade();
    let recent = state.recent.clone();
    v_flex()
        .gap(px(2.0))
        .child(
            row()
                .child(radio(
                    "import-dest-current",
                    t!("import.dest.current", name = workspace_name.as_str()).into_owned(),
                    Choice::Current,
                ))
                .child(div().flex_1())
                .child(side_text(shown(current))),
        )
        .when(recent_menu, |rows| {
            rows.child(
                row()
                    .child(radio(
                        "import-dest-recent",
                        t!("import.dest.recent").into_owned(),
                        Choice::Recent,
                    ))
                    .child(div().flex_1())
                    .child(
                        Button::new("import-dest-recent-menu")
                            .neutral_bordered(cx, palette.bg, palette.border, palette.fg_muted)
                            .h(px(26.0))
                            .px(px(10.0))
                            .rounded(px(6.0))
                            .border_1()
                            .child(recent_label)
                            .dropdown_caret(true)
                            .dropdown_menu(move |mut menu, _, _| {
                                for (index, path) in recent.iter().enumerate() {
                                    let weak = menu_block.clone();
                                    menu =
                                        menu.item(PopupMenuItem::new(folder_name(path)).on_click(
                                            move |_, _, cx| {
                                                let _ = weak.update(cx, |block, cx| {
                                                    block.state.recent_index = index;
                                                    block.select(Choice::Recent, cx);
                                                });
                                            },
                                        ));
                                }
                                menu
                            }),
                    ),
            )
        })
        .child(row().child(radio(
            "import-dest-new",
            t!("import.dest.new").into_owned(),
            Choice::New,
        )))
        .child(row().child(radio(
            "import-dest-existing",
            t!("import.dest.open_existing").into_owned(),
            Choice::Existing,
        )))
        .when(state.choice == Choice::New, |rows| {
            rows.child(
                div()
                    .pt(px(6.0))
                    .child(render_new_fields(block, palette, cx)),
            )
        })
        .when(state.choice == Choice::Existing, |rows| {
            rows.child(
                div()
                    .pt(px(6.0))
                    .child(render_existing_folder(block, palette, cx)),
            )
        })
        .when(
            matches!(state.choice, Choice::Current | Choice::Recent) && !collection.is_empty(),
            |rows| {
                let workspace = match state.choice {
                    Choice::Recent => recent_label_for(state),
                    _ => workspace_name.clone(),
                };
                rows.child(
                    h_flex()
                        .mt(px(6.0))
                        .gap(px(10.0))
                        .px(px(12.0))
                        .py(px(9.0))
                        .rounded(px(8.0))
                        .bg(palette.raised)
                        .text_color(palette.fg_muted)
                        .child(Icon::new(IconName::FolderTree).size(px(15.0)))
                        .child(div().flex_1().min_w_0().child(t!(
                            "import.dest.adds_folder",
                            collection = collection,
                            workspace = workspace.as_str()
                        ))),
                )
            },
        )
        .into_any_element()
}

/// The name of the selected recent workspace.
fn recent_label_for(state: &Destination) -> String {
    state
        .recent
        .get(state.recent_index)
        .map(|path| folder_name(path))
        .unwrap_or_default()
}

/// The last component of `path`.
fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// A pill button of the first use layout, styled like the wizard's source tabs.
fn pill(
    id: &'static str,
    icon: IconName,
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
        .h(px(PILL_HEIGHT))
        .gap(px(6.0))
        .px(px(12.0))
        .rounded(px(RADIUS_MD))
        .border_1()
        .border_color(if selected {
            palette.accent
        } else {
            palette.border
        })
        .bg(rest)
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
}

/// A read only field showing a path, with a button that changes it.
fn path_field(path: String, button: impl IntoElement, palette: &Palette, cx: &App) -> AnyElement {
    h_flex()
        .gap(px(8.0))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .h(px(FIELD_HEIGHT))
                .gap(px(8.0))
                .px(px(10.0))
                .rounded(px(RADIUS_MD))
                .bg(palette.bg)
                .border_1()
                .border_color(palette.border)
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(px(12.0))
                .child(
                    Icon::new(IconName::Folder)
                        .size(px(14.0))
                        .text_color(palette.fg_muted),
                )
                .child(div().min_w_0().truncate().child(path)),
        )
        .child(button)
        .into_any_element()
}

fn field_label(text: SharedString, palette: &Palette) -> Div {
    div()
        .text_size(px(12.0))
        .text_color(palette.fg_muted)
        .child(text)
}

/// The name, the folder it creates, the note and the Git option of a new workspace.
fn render_new_fields(block: &Entity<DestinationBlock>, palette: &Palette, cx: &App) -> AnyElement {
    let this = block.read(cx);
    let home = dirs::home_dir();
    let name = this.name.read(cx).value().to_string();
    let folder = this
        .state
        .folder(&name)
        .unwrap_or_else(|| this.state.location.clone());
    let shown = elide_path(&shorten_path(&folder, home.as_deref()), PATH_MAX_CHARS);
    let pick = block.downgrade();
    v_flex()
        .gap(px(10.0))
        .child(
            h_flex()
                .items_start()
                .gap(px(10.0))
                .child(
                    v_flex()
                        .w(px(NAME_WIDTH))
                        .flex_none()
                        .gap(px(6.0))
                        .child(field_label(t!("import.dest.name").into(), palette))
                        .child(text_field(&this.name, cx)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(px(6.0))
                        .child(field_label(t!("import.dest.will_create").into(), palette))
                        .child(path_field(
                            shown,
                            SecondaryButton::new("import-dest-location", t!("import.dest.choose"))
                                .height(FIELD_HEIGHT)
                                .on_click(move |_, _, cx| pick_location(pick.clone(), cx)),
                            palette,
                            cx,
                        )),
                ),
        )
        .child(
            div()
                .text_size(px(12.0))
                .line_height(relative(1.5))
                .text_color(palette.fg_subtle)
                .child(t!("import.dest.note")),
        )
        .when(this.git.is_some() && !this.inside_repo, |column| {
            let weak = block.downgrade();
            column.child(
                labelled_checkbox(
                    "import-dest-git",
                    this.init_git,
                    t!("shell.new_workspace.init_git"),
                    palette,
                )
                .on_change(move |checked, _, cx| {
                    let checked = *checked;
                    let _ = weak.update(cx, |block, cx| {
                        block.init_git = checked;
                        cx.notify();
                    });
                }),
            )
        })
        .into_any_element()
}

/// The picked folder and the button that changes it.
fn render_existing_folder(
    block: &Entity<DestinationBlock>,
    palette: &Palette,
    cx: &App,
) -> AnyElement {
    let state = &block.read(cx).state;
    let home = dirs::home_dir();
    let shown = state.existing.as_deref().map_or_else(String::new, |path| {
        elide_path(&shorten_path(path, home.as_deref()), PATH_MAX_CHARS)
    });
    let pick = block.downgrade();
    v_flex()
        .gap(px(6.0))
        .child(field_label(t!("import.dest.folder").into(), palette))
        .child(path_field(
            shown,
            SecondaryButton::new("import-dest-existing-pick", t!("import.dest.choose"))
                .height(FIELD_HEIGHT)
                .on_click(move |_, _, cx| pick_existing(pick.clone(), cx)),
            palette,
            cx,
        ))
        .into_any_element()
}

/// Design 3c: what the check of the folder found, with its two ways out.
fn render_notice(
    block: &Entity<DestinationBlock>,
    _palette: &Palette,
    cx: &App,
) -> Option<AnyElement> {
    let this = block.read(cx);
    let state = &this.state;
    let name = this.name.read(cx).value().to_string();
    let home = dirs::home_dir();
    let choose_other = {
        let weak = block.downgrade();
        let choice = state.choice;
        SecondaryButton::new("import-dest-notice-other", t!("import.dest.choose_other")).on_click(
            move |_, _, cx| {
                if choice == Choice::New {
                    pick_location(weak.clone(), cx);
                } else {
                    pick_existing(weak.clone(), cx);
                }
            },
        )
    };
    let accept = |label: SharedString| {
        let weak = block.downgrade();
        let name = name.clone();
        SecondaryButton::new("import-dest-notice-accept", label).on_click(move |_, _, cx| {
            let name = name.clone();
            let _ = weak.update(cx, |block, cx| {
                block.state.accept(&name);
                cx.notify();
            });
        })
    };
    let (kind, message, first): (InlineMessageKind, String, AnyElement) = match &state.check {
        FolderCheck::Ready => return None,
        FolderCheck::NameTaken { suggested } => {
            let folder = state.folder(&name).unwrap_or_default();
            let weak = block.downgrade();
            let value = suggested.clone();
            (
                InlineMessageKind::Info,
                t!(
                    "import.dest.name_taken",
                    path = shorten_path(&folder, home.as_deref()).as_str(),
                    suggested = suggested.as_str()
                )
                .into_owned(),
                SecondaryButton::new(
                    "import-dest-notice-suggested",
                    t!("import.dest.use_suggested", suggested = suggested.as_str()),
                )
                .on_click(move |_, window, cx| {
                    let value = value.clone();
                    let _ = weak.update(cx, |block, cx| {
                        block.suggested = value.clone();
                        block
                            .name
                            .update(cx, |state, cx| state.set_value(value, window, cx));
                        block.refresh(cx);
                    });
                })
                .into_any_element(),
            )
        }
        FolderCheck::IsWorkspace => {
            if state.accepted == state.folder(&name) {
                return None;
            }
            (
                InlineMessageKind::Info,
                t!("import.dest.is_workspace").into_owned(),
                accept(t!("import.dest.add_there").into()).into_any_element(),
            )
        }
        FolderCheck::HasFiles { count } => {
            if state.accepted == state.folder(&name) {
                return None;
            }
            (
                InlineMessageKind::Warning,
                plural("import.dest.has_files", *count),
                accept(t!("import.dest.use_anyway").into()).into_any_element(),
            )
        }
        FolderCheck::NotWritable => {
            let weak = block.downgrade();
            (
                InlineMessageKind::Danger,
                t!("import.dest.not_writable").into_owned(),
                SecondaryButton::new(
                    "import-dest-notice-default",
                    t!("import.dest.back_to_default"),
                )
                .on_click(move |_, _, cx| {
                    let _ = weak.update(cx, |block, cx| {
                        block.state.location = default_location();
                        block.inside_repo = is_inside_repo(&block.state.location);
                        block.select(Choice::New, cx);
                    });
                })
                .into_any_element(),
            )
        }
    };
    Some(
        v_flex()
            .gap(px(8.0))
            .child(InlineMessage::new(kind, message))
            .child(h_flex().gap(px(8.0)).child(first).child(choose_other))
            .into_any_element(),
    )
}

/// Whether `location`, or a folder above it, is a Git work tree.
fn is_inside_repo(location: &Path) -> bool {
    location
        .ancestors()
        .any(|folder| folder.join(".git").exists())
}

/// Runs `import` in the workspace of `destination` when it is not the open one: creates a new
/// workspace first and removes what was created if anything fails. The caller opens the folder
/// afterwards.
pub(crate) fn import_outside<R>(
    destination: &ResolvedDestination,
    import: impl FnOnce(&mut Workspace) -> Result<R, WorkspaceError>,
) -> Result<R, WorkspaceError> {
    let path = match &destination.target {
        Target::Current => {
            return Err(WorkspaceError::AlreadyExists(
                "the open workspace is not an outside destination".to_string(),
            ));
        }
        Target::Open(path) => {
            let mut workspace = Workspace::open(path)?;
            return import(&mut workspace);
        }
        Target::Create(path) => path,
    };
    let existed = path.exists();
    let result = (|| {
        create_workspace(path, NewWorkspaceContent::Empty)?;
        if let Some(git) = &destination.init_git {
            // A failure keeps the workspace, without git, like the "New workspace" dialog.
            if let Err(error) = crate::views::git::init_with_gitignore(git, path) {
                log::warn!("git init in the new workspace failed: {error}");
            }
        }
        let mut workspace = Workspace::open(path)?;
        import(&mut workspace)
    })();
    if let Err(error) = &result {
        log_workspace_error("import into a new workspace", error);
        remove_created(path, existed);
    }
    result
}

/// Undoes a workspace the wizard created at `path`. `existed` tells that the folder was already
/// there, empty: only what is inside goes then.
fn remove_created(path: &Path, existed: bool) {
    let removed = if existed {
        fs::read_dir(path).and_then(|entries| {
            entries.flatten().try_for_each(|entry| {
                if entry.path().is_dir() {
                    fs::remove_dir_all(entry.path())
                } else {
                    fs::remove_file(entry.path())
                }
            })
        })
    } else {
        fs::remove_dir_all(path)
    };
    if let Err(error) = removed {
        log::warn!("could not undo the new workspace: {error}");
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    // Not a glob: `gpui_kit::*` brings an attribute named `test` that would replace the real one.
    use std::fs;
    use std::path::{Path, PathBuf};

    use brasa_workspace::WorkspaceError;

    use super::{ResolvedDestination, Target, import_outside};

    fn create_at(path: &Path) -> ResolvedDestination {
        ResolvedDestination {
            target: Target::Create(path.to_path_buf()),
            init_git: None,
        }
    }

    fn failure() -> WorkspaceError {
        WorkspaceError::NotEmpty(PathBuf::from("forced"))
    }

    #[test]
    fn a_new_workspace_is_created_for_the_import() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("Brasa/acme");
        let root = import_outside(&create_at(&path), |workspace| {
            Ok(workspace.root().to_path_buf())
        })
        .expect("import");
        assert!(root.ends_with("acme"));
        assert!(path.join(".gitignore").is_file());
    }

    #[test]
    fn a_failed_import_removes_the_new_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("acme");
        let result = import_outside(&create_at(&path), |workspace| {
            fs::write(workspace.root().join("partial.brasa"), "GET http://x\n").expect("write");
            Err::<(), _>(failure())
        });
        assert!(result.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn a_failed_import_keeps_a_folder_that_was_already_there_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("acme");
        fs::create_dir(&path).expect("mkdir");
        let result = import_outside(&create_at(&path), |_| Err::<(), _>(failure()));
        assert!(result.is_err());
        assert!(path.is_dir());
        assert_eq!(fs::read_dir(&path).expect("read").count(), 0);
    }

    #[test]
    fn a_failed_import_never_touches_an_existing_workspace() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("mine.brasa"), "GET http://x\n").expect("write");
        let destination = ResolvedDestination {
            target: Target::Open(dir.path().to_path_buf()),
            init_git: None,
        };
        let result = import_outside(&destination, |_| Err::<(), _>(failure()));
        assert!(result.is_err());
        assert!(dir.path().join("mine.brasa").is_file());
    }
}
