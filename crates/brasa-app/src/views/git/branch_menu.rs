//! The branch menu of the status bar (#173, design 7b): telaio's searchable menu filled with the
//! branches of `state::git_branches`, the New branch, Pull, Push and Log actions, a submenu per
//! branch and the "Create branch" row. The rows are rebuilt when the branches or the starred ones
//! change, never in render.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use telaio_ui::components::{
    MenuAction, MenuGroup, MenuRow, SearchableMenuEvent, SearchableMenuState, SubmenuEntry,
    SubmenuItem,
};

use crate::state::git_branches::{BranchRow, branch_groups, local_names};
use crate::theme::PaletteExt;

use super::super::root::AppView;
use super::branches::CheckoutTarget;

/// The prefix of a branch row's id; the rest is the branch name.
const ROW_PREFIX: &str = "branch:";
/// The id of the "Create branch" row.
const CREATE_ROW: &str = "create";

/// What the submenu closure needs to know, shared with the menu and updated on every rebuild.
#[derive(Default)]
pub(crate) struct MenuContext {
    /// The checked out branch.
    current: String,
    /// The local branch names, so "Create branch" is not offered for one that exists.
    locals: Vec<String>,
    /// The remote branch names, whose submenu cannot rename or delete.
    remotes: Vec<String>,
}

/// The items of a branch's submenu, in order. The menu reports the index of the chosen one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmenuAction {
    Checkout,
    NewBranchFrom,
    CompareWithCurrent,
    ShowLog,
    Rename,
    Delete,
}

const SUBMENU: [SubmenuAction; 6] = [
    SubmenuAction::Checkout,
    SubmenuAction::NewBranchFrom,
    SubmenuAction::CompareWithCurrent,
    SubmenuAction::ShowLog,
    SubmenuAction::Rename,
    SubmenuAction::Delete,
];

/// Builds the menu's state, once per window.
pub(crate) fn new_branch_menu(
    context: Rc<RefCell<MenuContext>>,
    window: &mut Window,
    cx: &mut Context<AppView>,
) -> Entity<SearchableMenuState> {
    let submenu_context = context.clone();
    let create_context = context;
    let menu = cx.new(|cx| {
        SearchableMenuState::new(window, cx)
            .placeholder(t!("git.menu.search"))
            .empty_message(t!("git.menu.empty"))
            .hints([
                (
                    "\u{21b5}".to_string(),
                    t!("git.menu.hint_checkout").into_owned(),
                ),
                (
                    crate::state::palette::send_shortcut_label(),
                    t!("git.menu.hint_create").into_owned(),
                ),
            ])
            .submenu(move |row| submenu(&submenu_context.borrow(), row))
            .create_row(move |query| create_row(&create_context.borrow(), query))
    });
    cx.subscribe_in(
        &menu,
        window,
        |view, _, event: &SearchableMenuEvent, window, cx| {
            view.on_branch_menu_event(event, window, cx);
        },
    )
    .detach();
    menu
}

/// The submenu of `row`.
fn submenu(context: &MenuContext, row: &MenuRow) -> Vec<SubmenuEntry> {
    let Some(name) = row.id.strip_prefix(ROW_PREFIX) else {
        return Vec::new();
    };
    let remote = context.remotes.iter().any(|remote| remote == name);
    let current = name == context.current;
    let item = |action: SubmenuAction| {
        let (label, icon) = match action {
            SubmenuAction::Checkout => (t!("git.menu.checkout").into_owned(), IconName::GitBranch),
            SubmenuAction::NewBranchFrom => (
                t!("git.menu.new_from", name = name).into_owned(),
                IconName::Plus,
            ),
            SubmenuAction::CompareWithCurrent => (
                t!("git.menu.compare", name = context.current.as_str()).into_owned(),
                IconName::GitCompare,
            ),
            SubmenuAction::ShowLog => (t!("git.menu.show_log").into_owned(), IconName::GitGraph),
            SubmenuAction::Rename => (t!("git.menu.rename").into_owned(), IconName::Pencil),
            SubmenuAction::Delete => (t!("common.delete").into_owned(), IconName::Trash),
        };
        let disabled = match action {
            SubmenuAction::Checkout | SubmenuAction::CompareWithCurrent => current,
            SubmenuAction::Rename => remote,
            SubmenuAction::Delete => remote || current,
            SubmenuAction::NewBranchFrom | SubmenuAction::ShowLog => false,
        };
        SubmenuEntry::item(
            SubmenuItem::new(label)
                .icon(icon)
                .danger(action == SubmenuAction::Delete)
                .disabled(disabled),
        )
    };
    vec![
        item(SubmenuAction::Checkout),
        item(SubmenuAction::NewBranchFrom),
        SubmenuEntry::Separator,
        item(SubmenuAction::CompareWithCurrent),
        item(SubmenuAction::ShowLog),
        SubmenuEntry::Separator,
        item(SubmenuAction::Rename),
        item(SubmenuAction::Delete),
    ]
}

/// "Create branch <query>", unless a local branch has that name.
fn create_row(context: &MenuContext, query: &str) -> Option<MenuRow> {
    let name = query.trim();
    if name.is_empty() || context.locals.iter().any(|local| local == name) {
        return None;
    }
    Some(
        MenuRow::new(
            CREATE_ROW,
            t!("git.menu.create", name = name).into_owned(),
            IconName::Plus,
        )
        .search_text("")
        .subtitle(t!("git.menu.create_from", name = context.current.as_str()).into_owned())
        .badge(crate::state::palette::send_shortcut_label()),
    )
}

impl AppView {
    /// Rebuilds the menu's rows and actions from the branches read last.
    pub(crate) fn rebuild_branch_menu(&mut self, cx: &mut Context<Self>) {
        let Some(list) = self.git.branches.clone() else {
            return;
        };
        let palette = cx.palette();
        let now = chrono::Utc::now().timestamp();
        let groups = branch_groups(
            &list.branches,
            &list.recent,
            &self.state.settings.git_favorite_branches,
            now,
        );
        {
            let mut context = self.branch_menu_context.borrow_mut();
            context.current = self.git.branch_label().unwrap_or_default();
            context.locals = local_names(&list.branches)
                .into_iter()
                .map(str::to_string)
                .collect();
            context.remotes = groups
                .iter()
                .flat_map(|group| group.rows.iter())
                .filter(|row| row.remote)
                .map(|row| row.name.clone())
                .collect();
        }
        let row = |branch: &BranchRow| {
            let icon = if branch.current {
                IconName::Check
            } else if branch.remote {
                IconName::Cloud
            } else {
                IconName::GitBranch
            };
            let mut row = MenuRow::new(
                format!("{ROW_PREFIX}{}", branch.name),
                branch.name.clone(),
                icon,
            )
            .subtitle(branch.subtitle.clone())
            .toggle(branch.favorite);
            if branch.current {
                row = row
                    .icon_color(palette.accent_text)
                    .title_color(palette.accent_text);
            }
            if !branch.sync.is_empty() {
                row = row.badge(branch.sync.clone());
            }
            row
        };
        let groups: Vec<MenuGroup> = groups
            .iter()
            .map(|group| MenuGroup::new(group.kind.label(), group.rows.iter().map(row)))
            .collect();
        let ahead = self.git.status().map_or(0, |(_, status)| status.ahead);
        let mut push = MenuAction::new("push", t!("git.menu.push"), IconName::ArrowUp);
        if ahead > 0 {
            push = push.badge(ahead.to_string());
        }
        let actions = [
            MenuAction::new("new", t!("git.menu.new_branch"), IconName::Plus),
            MenuAction::new("pull", t!("git.menu.pull"), IconName::ArrowDown),
            push,
            MenuAction::new("log", t!("git.menu.log"), IconName::GitGraph),
        ];
        self.branch_menu.update(cx, |menu, cx| {
            menu.set_groups(groups, cx);
            menu.set_actions(actions.into(), cx);
        });
    }

    /// Opens the branch menu from the keyboard (the palette's "Checkout branch").
    pub(crate) fn open_branch_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.git.repo().is_none() {
            return;
        }
        self.load_branches(cx);
        self.branch_menu
            .update(cx, |menu, cx| menu.open(window, cx));
        cx.notify();
    }

    /// What the menu reports.
    fn on_branch_menu_event(
        &mut self,
        event: &SearchableMenuEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SearchableMenuEvent::Activate(id) if id.as_ref() == CREATE_ROW => {
                let query = self.branch_menu.read(cx).query().trim().to_string();
                self.create_branch(query, None, cx);
            }
            SearchableMenuEvent::Activate(id) => {
                if let Some(name) = id.strip_prefix(ROW_PREFIX)
                    && Some(name) != self.git.branch_label().as_deref()
                {
                    self.request_checkout(CheckoutTarget::Branch(name.to_string()), window, cx);
                }
            }
            SearchableMenuEvent::Secondary(query) => {
                let name = query.trim().to_string();
                let exists = self.branch_menu_context.borrow().locals.contains(&name);
                if exists {
                    // The branch is already there: go to it instead of failing to create it.
                    if Some(name.as_str()) != self.git.branch_label().as_deref() {
                        self.request_checkout(CheckoutTarget::Branch(name), window, cx);
                    }
                } else if !name.is_empty() {
                    self.create_branch(name, None, cx);
                }
            }
            SearchableMenuEvent::Action(id) => match id.as_ref() {
                "new" => self.new_branch_dialog(None, String::new(), window, cx),
                "pull" => self.pull(false, window, cx),
                "push" => self.start_push(true, window, cx),
                "log" => self.open_git_log(None, window, cx),
                _ => {}
            },
            SearchableMenuEvent::SubmenuItem(id, index) => {
                let (Some(name), Some(action)) = (id.strip_prefix(ROW_PREFIX), SUBMENU.get(*index))
                else {
                    return;
                };
                let name = name.to_string();
                match action {
                    SubmenuAction::Checkout => {
                        self.request_checkout(CheckoutTarget::Branch(name), window, cx);
                    }
                    SubmenuAction::NewBranchFrom => {
                        self.new_branch_dialog(Some(name), String::new(), window, cx);
                    }
                    SubmenuAction::CompareWithCurrent => {
                        let current = self.git.branch_label().unwrap_or_default();
                        self.open_git_log(Some(format!("{current}..{name}")), window, cx);
                    }
                    SubmenuAction::ShowLog => self.open_git_log(Some(name), window, cx),
                    SubmenuAction::Rename => self.rename_branch_dialog(name, window, cx),
                    SubmenuAction::Delete => self.delete_branch_dialog(name, window, cx),
                }
            }
            SearchableMenuEvent::Toggle(id) => {
                if let Some(name) = id.strip_prefix(ROW_PREFIX) {
                    self.toggle_favorite_branch(name.to_string(), cx);
                }
            }
            SearchableMenuEvent::Dismiss => {}
        }
    }
}
