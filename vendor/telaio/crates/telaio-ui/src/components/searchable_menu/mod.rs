//! [`SearchableMenu`]: a popover with a search box that filters grouped rows, an optional row of
//! action buttons, a per row submenu and a hint bar. In Postino it is the branch menu of the
//! status bar; the component knows nothing about git, it takes plain rows and labels.
//!
//! ```text
//! +----------------------------------------------+
//! | (search) Search branches and actions         |  search input, filters everything
//! | [+ New branch] [Pull] [Push 1] [Log]         |  actions, filtered by the query too
//! | RECENT                                       |  group label
//! | (icon) main                      up1  *  >   |  row: title, subtitle, badge, toggle, submenu
//! |        Bump version                          |
//! | + Create branch "fea"          Ctrl+Enter    |  the row built from the query by the app
//! | Enter Checkout  Right More actions           |  hints
//! +----------------------------------------------+
//! ```
//!
//! # Pieces
//!
//! - [`SearchableMenuState`] is a gpui entity the app creates once and keeps. It holds the rows,
//!   the query, the filtered list and the highlight, and it is what the popover renders. The app
//!   subscribes to its [`SearchableMenuEvent`]s.
//! - [`SearchableMenu`] is the element: a gpui-component `Popover` around a trigger, showing the
//!   state's panel above or below it. Open and close it from the app with
//!   [`SearchableMenuState::open`] and [`SearchableMenuState::close`]; the trigger click, Escape
//!   and a click outside do it from the user's side.
//!
//! ```ignore
//! let menu = cx.new(|cx| {
//!     SearchableMenuState::new(window, cx)
//!         .placeholder("Search branches and actions")
//!         .empty_message("No matches")
//!         .hints([("Enter", "Checkout"), ("Right", "More actions")])
//!         .actions([MenuAction::new("pull", "Pull", IconName::ArrowDown)])
//!         .groups([MenuGroup::new("Local", [MenuRow::new("main", "main", IconName::GitBranch)])])
//!         .submenu(|row| vec![SubmenuEntry::item(SubmenuItem::new("Delete").danger(true))])
//!         .create_row(|query| Some(MenuRow::new("create", format!("Create {query}"), IconName::Plus)))
//! });
//! cx.subscribe(&menu, |this, _, event: &SearchableMenuEvent, cx| { /* ... */ }).detach();
//! // In render:
//! SearchableMenu::new("branches", &menu).trigger(Button::new("branch").label("main"))
//! ```
//!
//! # Behavior
//!
//! - The query is a case insensitive substring match on each row's `search_text` and each
//!   action's label. A group with no match disappears with its label. The matched part of a
//!   row's title is drawn in the accent color, weight 600. The filtered list is computed when the
//!   query or the data changes, never in `render`.
//! - Keyboard (the search box keeps focus): Up and Down move the highlight over the visible
//!   rows, skipping labels, wrapping around; Enter runs the highlighted row
//!   ([`SearchableMenuEvent::Activate`]); Ctrl+Enter runs the secondary action
//!   ([`SearchableMenuEvent::Secondary`], with the query, only when it is not empty); Right opens
//!   the highlighted row's submenu when the caret is at the end of the query; Left or Escape
//!   closes the submenu; Escape with no submenu closes the menu. The mouse highlights the row
//!   under it; the chevron opens the submenu.
//! - The menu closes itself after `Activate`, `Secondary`, `Action` and `SubmenuItem`. `Toggle`
//!   keeps it open. `Dismiss` is the user closing it without choosing.
//! - The submenu is gpui-component's `PopupMenu`, so it has its own keyboard navigation, and it
//!   opens to the right of the row.
//!
//! # Long lists
//!
//! The rows are drawn by gpui's variable height `list` (labels are 28 px, rows 42 px, so
//! `uniform_list` does not fit): only the visible rows plus a small overdraw are built, so a few
//! thousand rows cost the same to draw as a dozen. Filtering is one pass over the pre-lowercased
//! search texts when the query changes. Nothing is capped or grouped under "N more".
//!
//! # Keys
//!
//! The panel uses the key contexts `List` and `PopupMenu`, whose bindings (Up, Down, Enter,
//! Ctrl+Enter, Escape, Left, Right) gpui-component already registers in `gpui_kit::init`.

mod logic;
mod panel;
mod state;
mod view;

pub use logic::{Entry, highlight_range};
pub use state::{
    MenuAction, MenuGroup, MenuRow, SearchableMenuEvent, SearchableMenuState, SubmenuEntry,
    SubmenuItem,
};
pub use view::SearchableMenu;
