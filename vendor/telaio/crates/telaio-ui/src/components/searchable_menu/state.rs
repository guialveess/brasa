//! The data types and the state entity of the searchable menu. The drawing is in `panel.rs`.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::logic::{
    Entry, content_height, filter_entries, filter_indices, first_selectable, fold, move_highlight,
};
use crate::PaletteExt;
use crate::metrics::{
    SEARCHABLE_MENU_LIST_MAX_HEIGHT, SEARCHABLE_MENU_SUBMENU_ROW_HEIGHT,
    SEARCHABLE_MENU_SUBMENU_WIDTH,
};

/// Vertical padding of the row area (4 px above, 6 px below), part of the list's height.
pub(super) const LIST_PADDING_Y: f32 = 10.0;

/// One row of a group.
#[derive(Clone)]
pub struct MenuRow {
    /// Identifies the row in the events. Unique within the menu.
    pub id: SharedString,
    /// First line, monospace. The matched part of the query is highlighted in it.
    pub title: SharedString,
    /// Second line, `fg_subtle`.
    pub subtitle: Option<SharedString>,
    /// The icon in the leading column.
    pub icon: IconName,
    /// Color of the icon; `fg_subtle` when `None`.
    pub icon_color: Option<Hsla>,
    /// Color of the title; `fg` when `None`.
    pub title_color: Option<Hsla>,
    /// Short text before the chevron, monospace in the accent color (a sync state such as "up 1").
    pub badge: Option<SharedString>,
    /// A trailing toggle (a favourite star): `Some(on)`. Drawn while the row is highlighted or
    /// on; clicking it emits [`SearchableMenuEvent::Toggle`] and the app updates the data.
    pub toggle: Option<bool>,
    /// The text the query is matched against, usually the title and the subtitle.
    pub search_text: SharedString,
}

impl MenuRow {
    /// A row with an `id`, a `title` (also its search text) and an `icon`.
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        icon: IconName,
    ) -> Self {
        let title = title.into();
        Self {
            id: id.into(),
            search_text: title.clone(),
            title,
            subtitle: None,
            icon,
            icon_color: None,
            title_color: None,
            badge: None,
            toggle: None,
        }
    }

    /// Sets the second line, and adds it to the search text.
    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        let subtitle = subtitle.into();
        self.search_text = format!("{} {}", self.title, subtitle).into();
        self.subtitle = Some(subtitle);
        self
    }

    /// Sets the color of the icon.
    pub fn icon_color(mut self, color: Hsla) -> Self {
        self.icon_color = Some(color);
        self
    }

    /// Sets the color of the title.
    pub fn title_color(mut self, color: Hsla) -> Self {
        self.title_color = Some(color);
        self
    }

    /// Sets the trailing badge text.
    pub fn badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// Adds the trailing toggle, `on` or off.
    pub fn toggle(mut self, on: bool) -> Self {
        self.toggle = Some(on);
        self
    }

    /// Replaces the text the query is matched against.
    pub fn search_text(mut self, text: impl Into<SharedString>) -> Self {
        self.search_text = text.into();
        self
    }
}

/// A labelled group of rows.
#[derive(Clone)]
pub struct MenuGroup {
    /// Shown above the rows, as given (sentence case).
    pub label: SharedString,
    /// The rows. A group that ends up with none (or none matching) shows nothing.
    pub rows: Vec<MenuRow>,
}

impl MenuGroup {
    /// A group with a `label` and its `rows`.
    pub fn new(label: impl Into<SharedString>, rows: impl IntoIterator<Item = MenuRow>) -> Self {
        Self {
            label: label.into(),
            rows: rows.into_iter().collect(),
        }
    }
}

/// A button of the action row. The buttons share the width equally.
#[derive(Clone)]
pub struct MenuAction {
    /// Identifies the action in [`SearchableMenuEvent::Action`].
    pub id: SharedString,
    /// The label, also what the query is matched against.
    pub label: SharedString,
    /// The icon before the label.
    pub icon: IconName,
    /// A small monospace text in the accent color after the label (a count).
    pub badge: Option<SharedString>,
    /// Shown but not clickable.
    pub disabled: bool,
}

impl MenuAction {
    /// An action with an `id`, a `label` and an `icon`.
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: IconName,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon,
            badge: None,
            disabled: false,
        }
    }

    /// Sets the badge text.
    pub fn badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    /// Disables the action.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// An item of a row's submenu.
#[derive(Clone)]
pub struct SubmenuItem {
    /// The label.
    pub label: SharedString,
    /// An icon before the label.
    pub icon: Option<IconName>,
    /// Drawn in the danger color.
    pub danger: bool,
    /// Shown but not clickable.
    pub disabled: bool,
}

impl SubmenuItem {
    /// An item with a `label`.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            danger: false,
            disabled: false,
        }
    }

    /// Sets the icon.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Draws the item in the danger color.
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// Disables the item.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// One entry of a submenu: an item or a separator line. The index in
/// [`SearchableMenuEvent::SubmenuItem`] counts the items only, not the separators.
#[derive(Clone)]
pub enum SubmenuEntry {
    /// A clickable item.
    Item(SubmenuItem),
    /// A separator line.
    Separator,
}

impl SubmenuEntry {
    /// An item entry.
    pub fn item(item: SubmenuItem) -> Self {
        Self::Item(item)
    }
}

/// What a [`SearchableMenuState`] reports to the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchableMenuEvent {
    /// A row (or the create row) was chosen with Enter or a click. Carries the row's id. The
    /// menu has closed.
    Activate(SharedString),
    /// Ctrl+Enter with a non-empty query. Carries the query. The menu has closed.
    Secondary(SharedString),
    /// An action button was clicked. Carries the action's id. The menu has closed.
    Action(SharedString),
    /// A submenu item was chosen: the row's id and the item's index among the items. The menu
    /// has closed.
    SubmenuItem(SharedString, usize),
    /// A row's toggle was clicked. Carries the row's id. The menu stays open; the app flips the
    /// data with [`SearchableMenuState::set_groups`].
    Toggle(SharedString),
    /// The user closed the menu without choosing (Escape or a click outside).
    Dismiss,
}

/// The submenu builder: the entries for a row.
type SubmenuBuilder = Rc<dyn Fn(&MenuRow) -> Vec<SubmenuEntry>>;
/// The create row builder: the row for a query, if any.
type CreateBuilder = Rc<dyn Fn(&str) -> Option<MenuRow>>;

/// The open submenu of a row.
pub(super) struct OpenSubmenu {
    /// The entry index of the row it belongs to.
    pub(super) entry: usize,
    pub(super) menu: Entity<PopupMenu>,
    _dismiss: Subscription,
}

/// The state of a [`super::SearchableMenu`]. See the module documentation.
pub struct SearchableMenuState {
    pub(super) input: Entity<InputState>,
    pub(super) groups: Vec<MenuGroup>,
    /// The folded search text of every row, parallel to `groups`.
    search: Vec<Vec<String>>,
    pub(super) actions: Vec<MenuAction>,
    action_search: Vec<String>,
    placeholder: SharedString,
    pub(super) empty_message: SharedString,
    pub(super) hints: Vec<(SharedString, SharedString)>,
    submenu_builder: Option<SubmenuBuilder>,
    create_builder: Option<CreateBuilder>,

    /// The query as typed.
    pub(super) query: SharedString,
    /// What is shown, rebuilt when the query or the data changes.
    pub(super) entries: Vec<Entry>,
    pub(super) visible_actions: Vec<usize>,
    pub(super) create: Option<MenuRow>,
    pub(super) highlight: Option<usize>,
    pub(super) list: ListState,
    pub(super) list_height: f32,
    pub(super) open: bool,
    pub(super) submenu: Option<OpenSubmenu>,
    /// Where the submenu was drawn last, so a click on it is not a click outside the menu.
    pub(super) submenu_bounds: Rc<std::cell::Cell<Bounds<Pixels>>>,
    _input_subscription: Subscription,
}

impl EventEmitter<SearchableMenuEvent> for SearchableMenuState {}

impl SearchableMenuState {
    /// An empty menu. Fill it with the builder methods.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this: &mut Self, _, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refilter(cx);
                }
            },
        );
        Self {
            input,
            groups: Vec::new(),
            search: Vec::new(),
            actions: Vec::new(),
            action_search: Vec::new(),
            placeholder: SharedString::default(),
            empty_message: SharedString::default(),
            hints: Vec::new(),
            submenu_builder: None,
            create_builder: None,
            query: SharedString::default(),
            entries: Vec::new(),
            visible_actions: Vec::new(),
            create: None,
            highlight: None,
            list: ListState::new(0, ListAlignment::Top, px(200.0)),
            list_height: 0.0,
            open: false,
            submenu: None,
            submenu_bounds: Rc::default(),
            _input_subscription: subscription,
        }
    }

    /// Sets the search box placeholder (applied each time the menu opens).
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Sets the text shown when nothing matches.
    pub fn empty_message(mut self, message: impl Into<SharedString>) -> Self {
        self.empty_message = message.into();
        self
    }

    /// Sets the key hints of the footer, as `(key, meaning)` pairs.
    pub fn hints<K, V>(mut self, hints: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<SharedString>,
        V: Into<SharedString>,
    {
        self.hints = hints
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        self
    }

    /// Sets the buttons of the action row.
    pub fn actions(mut self, actions: impl IntoIterator<Item = MenuAction>) -> Self {
        self.set_actions_inner(actions.into_iter().collect());
        self
    }

    /// Sets the groups of rows.
    pub fn groups(mut self, groups: impl IntoIterator<Item = MenuGroup>) -> Self {
        self.set_groups_inner(groups.into_iter().collect());
        self
    }

    /// Sets the submenu builder. Rows get a chevron and a submenu with the returned entries;
    /// without a builder rows have none.
    pub fn submenu(mut self, build: impl Fn(&MenuRow) -> Vec<SubmenuEntry> + 'static) -> Self {
        self.submenu_builder = Some(Rc::new(build));
        self
    }

    /// Sets the create row builder. It runs when the query changes and returns the row shown last
    /// for a non-empty query (or `None` for no row). Choosing it emits
    /// [`SearchableMenuEvent::Activate`] with that row's id.
    pub fn create_row(mut self, build: impl Fn(&str) -> Option<MenuRow> + 'static) -> Self {
        self.create_builder = Some(Rc::new(build));
        self
    }

    /// Replaces the groups, for data that changes while the menu is open. Keeps the query and
    /// the highlighted row (by id) when it is still shown.
    pub fn set_groups(&mut self, groups: Vec<MenuGroup>, cx: &mut Context<Self>) {
        let kept = self.highlighted_id();
        let scroll = self.list.logical_scroll_top();
        self.set_groups_inner(groups);
        self.refilter(cx);
        self.restore_highlight(kept);
        // Keep the place in a long list, as when a toggle changes one row.
        if !self.entries.is_empty() {
            self.list.scroll_to(ListOffset {
                item_ix: scroll.item_ix.min(self.entries.len() - 1),
                offset_in_item: scroll.offset_in_item,
            });
        }
        cx.notify();
    }

    /// Replaces the action buttons.
    pub fn set_actions(&mut self, actions: Vec<MenuAction>, cx: &mut Context<Self>) {
        self.set_actions_inner(actions);
        self.visible_actions = filter_indices(&self.action_search, &fold(&self.query));
        cx.notify();
    }

    /// Whether the menu is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The current query.
    pub fn query(&self) -> &SharedString {
        &self.query
    }

    /// Opens the menu with an empty query and the first row highlighted, and focuses the search
    /// box. The popover shows on the next render of the [`super::SearchableMenu`].
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = self.placeholder.clone();
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder, window, cx);
            input.set_value("", window, cx);
        });
        self.submenu = None;
        self.open = true;
        self.refilter(cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    /// Closes the menu from the app. Emits no event.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            self.submenu = None;
            self.submenu_bounds.set(Bounds::default());
            cx.notify();
        }
    }

    /// The user closed the menu without choosing.
    pub(super) fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.close(cx);
            cx.emit(SearchableMenuEvent::Dismiss);
        }
    }

    /// Closes the menu and reports what was chosen.
    pub(super) fn finish(&mut self, event: SearchableMenuEvent, cx: &mut Context<Self>) {
        self.close(cx);
        cx.emit(event);
    }

    fn set_groups_inner(&mut self, groups: Vec<MenuGroup>) {
        self.search = groups
            .iter()
            .map(|group| {
                group
                    .rows
                    .iter()
                    .map(|row| fold(&row.search_text))
                    .collect()
            })
            .collect();
        self.groups = groups;
        self.submenu = None;
    }

    fn set_actions_inner(&mut self, actions: Vec<MenuAction>) {
        self.action_search = actions.iter().map(|action| fold(&action.label)).collect();
        self.visible_actions = filter_indices(&self.action_search, "");
        self.actions = actions;
    }

    /// Rebuilds what is shown for the current query. Runs when the query or the data changes.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        self.query = self.input.read(cx).value();
        let folded = fold(&self.query);
        self.entries = filter_entries(&self.search, &folded);
        self.visible_actions = filter_indices(&self.action_search, &folded);
        self.create = match (&self.create_builder, self.query.is_empty()) {
            (Some(build), false) => build(&self.query),
            _ => None,
        };
        if self.create.is_some() {
            self.entries.push(Entry::Create);
        }
        let height = content_height(&self.entries) + LIST_PADDING_Y;
        self.list_height = height.min(SEARCHABLE_MENU_LIST_MAX_HEIGHT);
        self.list.reset(self.entries.len());
        self.list.scroll_to(ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });
        self.highlight = first_selectable(&self.entries);
        self.submenu = None;
        cx.notify();
    }

    /// The id of the highlighted row, if any.
    fn highlighted_id(&self) -> Option<SharedString> {
        self.highlight
            .and_then(|ix| self.row_at(ix))
            .map(|row| row.id.clone())
    }

    fn restore_highlight(&mut self, id: Option<SharedString>) {
        let Some(id) = id else { return };
        if let Some(ix) =
            (0..self.entries.len()).find(|&ix| self.row_at(ix).is_some_and(|row| row.id == id))
        {
            self.highlight = Some(ix);
        }
    }

    /// The row an entry stands for, including the create row.
    pub(super) fn row_at(&self, ix: usize) -> Option<&MenuRow> {
        match self.entries.get(ix)? {
            Entry::Row { group, row } => self.groups.get(*group)?.rows.get(*row),
            Entry::Create => self.create.as_ref(),
            Entry::Label(_) => None,
        }
    }

    /// Whether rows have a submenu.
    pub(super) fn has_submenus(&self) -> bool {
        self.submenu_builder.is_some()
    }

    /// Moves the highlight `delta` rows (negative is up) and scrolls it into view.
    pub(super) fn move_by(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(ix) = move_highlight(&self.entries, self.highlight, delta) {
            self.highlight = Some(ix);
            self.list.scroll_to_reveal_item(ix);
        }
        cx.notify();
    }

    /// Highlights the entry under the mouse.
    pub(super) fn hover(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.highlight == Some(ix) || !self.entries.get(ix).is_some_and(|e| e.is_selectable()) {
            return;
        }
        self.highlight = Some(ix);
        if self.submenu.as_ref().is_some_and(|open| open.entry != ix) {
            self.close_submenu(true, window, cx);
        }
        cx.notify();
    }

    /// Runs the primary action of an entry: choose its row.
    pub(super) fn activate(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(id) = self.row_at(ix).map(|row| row.id.clone()) {
            self.finish(SearchableMenuEvent::Activate(id), cx);
        }
    }

    /// Enter and Ctrl+Enter.
    pub(super) fn confirm(&mut self, secondary: bool, cx: &mut Context<Self>) {
        if secondary {
            if !self.query.is_empty() {
                let query = self.query.clone();
                self.finish(SearchableMenuEvent::Secondary(query), cx);
            }
        } else if let Some(ix) = self.highlight {
            self.activate(ix, cx);
        }
    }

    /// Opens the submenu of the entry at `ix` and moves the focus into it.
    pub(super) fn open_submenu(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(build) = self.submenu_builder.clone() else {
            return;
        };
        let Some(row) = self
            .row_at(ix)
            .filter(|_| self.entries[ix] != Entry::Create)
            .cloned()
        else {
            return;
        };
        let entries = build(&row);
        if !entries.iter().any(|e| matches!(e, SubmenuEntry::Item(_))) {
            return;
        }
        let weak = cx.entity().downgrade();
        let menu = PopupMenu::build(window, cx, |mut menu, _window, _cx| {
            menu = menu
                .min_w(px(SEARCHABLE_MENU_SUBMENU_WIDTH))
                .max_w(px(SEARCHABLE_MENU_SUBMENU_WIDTH));
            let mut index = 0;
            for entry in entries {
                match entry {
                    SubmenuEntry::Separator => menu = menu.separator(),
                    SubmenuEntry::Item(item) => {
                        menu = menu.item(submenu_item(item, index, row.id.clone(), weak.clone()));
                        index += 1;
                    }
                }
            }
            menu
        });
        let dismiss = cx.subscribe_in(
            &menu,
            window,
            |this: &mut Self, _, _: &DismissEvent, window, cx| {
                this.close_submenu(true, window, cx);
            },
        );
        menu.update(cx, |menu, cx| menu.focus_handle(cx).focus(window, cx));
        self.highlight = Some(ix);
        self.submenu = Some(OpenSubmenu {
            entry: ix,
            menu,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    /// Closes the submenu, giving the focus back to the search box when `refocus`.
    pub(super) fn close_submenu(
        &mut self,
        refocus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.submenu.take().is_some() {
            self.submenu_bounds.set(Bounds::default());
            if refocus && self.open {
                self.input.update(cx, |input, cx| input.focus(window, cx));
            }
            cx.notify();
        }
    }

    /// Whether the filtered list shows nothing at all.
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One item of a row's submenu, drawn by us so it can be red or dimmed. Clicking it reports the
/// choice to the menu state.
fn submenu_item(
    item: SubmenuItem,
    index: usize,
    row_id: SharedString,
    state: WeakEntity<SearchableMenuState>,
) -> PopupMenuItem {
    let SubmenuItem {
        label,
        icon,
        danger,
        disabled,
    } = item;
    PopupMenuItem::element(move |_window, cx| {
        let palette = cx.palette();
        let color = if disabled {
            palette.fg_subtle
        } else if danger {
            palette.danger
        } else {
            palette.fg
        };
        let icon_color = if danger && !disabled {
            palette.danger
        } else {
            palette.fg_muted
        };
        h_flex()
            .flex_1()
            .min_w_0()
            .h(px(SEARCHABLE_MENU_SUBMENU_ROW_HEIGHT))
            .items_center()
            .gap(px(8.0))
            .text_color(color)
            .child(div().w(px(14.0)).flex_none().children(
                icon.map(|icon| Icon::new(icon).with_size(px(13.0)).text_color(icon_color)),
            ))
            .child(div().flex_1().min_w_0().truncate().child(label.clone()))
    })
    .disabled(disabled)
    .on_click(move |_, _, cx| {
        let event = SearchableMenuEvent::SubmenuItem(row_id.clone(), index);
        _ = state.update(cx, |state, cx| state.finish(event, cx));
    })
}
