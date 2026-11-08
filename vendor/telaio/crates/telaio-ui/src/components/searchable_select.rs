//! [`SearchableSelect`]: a select box over a list of plain options, with an optional search box
//! for long lists (a font picker). It is gpui-component's `Select` restyled as the design
//! system's select (see [`super::select_trigger`]) and with a list that keeps one height while
//! the user filters it.
//!
//! A `Select` needs a state entity, so the caller creates it once with [`select_state`] (when a
//! dialog opens, say) and keeps it. Each option has a stable string value apart from its label,
//! which is what the caller stores and receives back. The choices arrive through [`on_pick`], or
//! by subscribing to the state's `SelectEvent` directly. [`SearchableSelect::new`] renders the
//! state, [`set_options`] swaps its options and [`pick`] changes the selection from outside.
//!
//! The list is gpui-component's: it scrolls with a scrollbar, sits under the box when it fits
//! and above it when it does not, and with the search box gives keyboard access (type to filter,
//! arrows, Enter).

use gpui_kit::component::Icon;
use gpui_kit::component::IndexPath;
use gpui_kit::component::searchable_list::{
    SearchableListDelegate, SearchableListItem, SearchableVec,
};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::PaletteExt;
use crate::metrics::{
    CONTROL_HEIGHT, RADIUS_MD, SELECT_MENU_MAX_HEIGHT, SELECT_MONO_TEXT_SIZE, SELECT_SEARCH_ROWS,
    SELECT_WIDTH,
};

/// One choice of a select.
#[derive(Clone)]
pub struct SelectOption {
    value: SharedString,
    label: SharedString,
    disabled: bool,
}

impl SelectOption {
    /// An option identified by `value` and shown as `label`.
    pub fn new(value: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            disabled: false,
        }
    }

    /// Whether the option is listed but cannot be picked.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// An empty row that pads a filtered list; `index` keeps the values apart.
    fn filler(index: usize, label: impl Into<SharedString>) -> Self {
        Self {
            value: format!("\u{0}filler-{index}").into(),
            label: label.into(),
            disabled: true,
        }
    }
}

impl SearchableListItem for SelectOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &SharedString {
        &self.value
    }

    fn disabled(&self) -> bool {
        self.disabled
    }
}

/// What the list shows at one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    /// The `n`th match of the search.
    Match(usize),
    /// The "No results" message, in place of the first row of an empty result.
    NoResults,
    /// The `n`th empty filler row.
    Filler(usize),
}

/// The slot at `row` of a list with `matches` matches padded to `rows` rows, or `None` past its
/// end.
fn slot(row: usize, matches: usize, rows: usize) -> Option<Slot> {
    if row >= row_count(matches, rows) {
        None
    } else if row < matches {
        Some(Slot::Match(row))
    } else if matches == 0 && row == 0 && rows > 0 {
        Some(Slot::NoResults)
    } else {
        Some(Slot::Filler(row - matches))
    }
}

/// How many rows a list with `matches` matches has when padded to `rows`.
fn row_count(matches: usize, rows: usize) -> usize {
    matches.max(rows)
}

/// The options of a select, with empty filler rows after the matches so that a filtered list is
/// as tall as the full one. gpui-component places the list under the box when it fits and above
/// it when it does not, deciding again on every change of the list: a list that shrinks while the
/// user types would jump from above the box to below it. A constant height keeps it in place.
/// With no match at all, the first filler reads "No results".
pub struct PaddedOptions {
    options: SearchableVec<SelectOption>,
    fillers: Vec<SelectOption>,
    no_results: SelectOption,
    matches: usize,
}

impl PaddedOptions {
    /// `min_rows` is 0 for a select without search, which needs no padding.
    fn new(options: Vec<SelectOption>, min_rows: usize) -> Self {
        let matches = options.len();
        Self {
            options: SearchableVec::new(options),
            fillers: (0..min_rows).map(|i| SelectOption::filler(i, "")).collect(),
            no_results: SelectOption::filler(min_rows, t!("telaio.select.no_results")),
            matches,
        }
    }
}

impl SearchableListDelegate for PaddedOptions {
    type Item = SelectOption;

    fn items_count(&self, _section: usize) -> usize {
        row_count(self.matches, self.fillers.len())
    }

    fn item(&self, ix: IndexPath) -> Option<&SelectOption> {
        match slot(ix.row, self.matches, self.fillers.len())? {
            Slot::Match(_) => self.options.item(ix),
            Slot::NoResults => Some(&self.no_results),
            Slot::Filler(n) => self.fillers.get(n),
        }
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        SelectOption: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.options.position(value)
    }

    fn perform_search(&mut self, query: &str, window: &mut Window, cx: &mut App) -> Task<()> {
        let task = self.options.perform_search(query, window, cx);
        self.matches = self.options.items_count(0);
        task
    }
}

/// The entity behind one select. Create it once with [`select_state`] and keep it.
pub type SelectHandle = Entity<SelectState<PaddedOptions>>;

/// Creates the state of a select over `options`, showing the option whose value is `selected`
/// (none when it is `None` or not listed). With `searchable`, the list has a search box and
/// keeps [`SELECT_SEARCH_ROWS`] rows while filtered; use it for long lists.
pub fn select_state(
    options: Vec<SelectOption>,
    selected: Option<&str>,
    searchable: bool,
    window: &mut Window,
    cx: &mut App,
) -> SelectHandle {
    let rows = if searchable { SELECT_SEARCH_ROWS } else { 0 };
    let selected = selected.map(|value| SharedString::from(value.to_string()));
    cx.new(|cx| {
        let mut state = SelectState::new(PaddedOptions::new(options, rows), None, window, cx)
            .searchable(searchable);
        if let Some(value) = &selected {
            state.set_selected_value(value, window, cx);
        }
        state
    })
}

/// Runs `handler` with the value of every option the user confirms in `select`, from the view
/// `cx` belongs to. Keep the returned subscription alive as long as the select is.
pub fn on_pick<V: 'static>(
    select: &SelectHandle,
    window: &mut Window,
    cx: &mut Context<V>,
    handler: impl Fn(&mut V, &str, &mut Window, &mut Context<V>) + 'static,
) -> Subscription {
    cx.subscribe_in(
        select,
        window,
        move |view, _, event: &SelectEvent<PaddedOptions>, window, cx| {
            if let SelectEvent::Confirm(Some(value)) = event {
                handler(view, value, window, cx);
            }
        },
    )
}

/// Replaces the options of `select` (new labels after a language change, say). A searchable
/// select keeps its padding, so give a long list only when its options really changed.
pub fn set_options(
    select: &SelectHandle,
    options: Vec<SelectOption>,
    searchable: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let rows = if searchable { SELECT_SEARCH_ROWS } else { 0 };
    select.update(cx, |state, cx| {
        state.set_items(PaddedOptions::new(options, rows), window, cx)
    });
}

/// Selects the option with `value`, unless it is already the selected one. It emits no event.
pub fn pick(select: &SelectHandle, value: &str, window: &mut Window, cx: &mut App) {
    let value: SharedString = value.to_string().into();
    select.update(cx, |state, cx| {
        if state.selected_value() != Some(&value) {
            state.set_selected_value(&value, window, cx);
        }
    });
}

/// A select box styled as the design system's select ([`super::select_trigger`]): the same
/// height, border, background, hover and chevrons icon, plus an `accent` border while it has the
/// focus or its list is open.
#[derive(IntoElement)]
pub struct SearchableSelect {
    id: ElementId,
    select: SelectHandle,
    width: f32,
    mono: Option<SharedString>,
    disabled: bool,
}

impl SearchableSelect {
    /// The box for `select`. `id` must be unique in the window.
    pub fn new(id: impl Into<ElementId>, select: &SelectHandle) -> Self {
        Self {
            id: id.into(),
            select: select.clone(),
            width: SELECT_WIDTH,
            mono: None,
            disabled: false,
        }
    }

    /// The box's width in pixels, [`SELECT_WIDTH`] by default.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the label in the monospace `family`, at the editors' size.
    pub fn mono(mut self, family: impl Into<SharedString>) -> Self {
        self.mono = Some(family.into());
        self
    }

    /// Whether the box ignores clicks.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for SearchableSelect {
    /// gpui-component's `Select` has no hover style of its own and, with `appearance(false)`, no
    /// focus look either, so this tracks the pointer like `HoverInput` (only entering and leaving
    /// renders again) and reads the focus on every render.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let box_id = SharedString::from(format!("{}-box", self.id));
        let hovered = window.use_keyed_state(box_id.clone(), cx, |_, _| false);
        // The open list and its search box live inside the select's focus tree, so this also
        // holds while the list is open.
        let active = self
            .select
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        let fill = if *hovered.read(cx) && !self.disabled {
            palette.hover_over(palette.raised)
        } else {
            palette.raised
        };
        let mut field = Select::new(&self.select)
            .id(self.id)
            .appearance(false)
            .disabled(self.disabled)
            .icon(
                Icon::new(gpui_kit::assets::IconName::ChevronsUpDown).text_color(palette.fg_subtle),
            )
            .search_placeholder(t!("telaio.select.search"))
            .menu_max_h(px(SELECT_MENU_MAX_HEIGHT))
            .h(px(CONTROL_HEIGHT))
            .px(px(10.0))
            .rounded(px(RADIUS_MD - 1.0))
            .border_1()
            .border_color(if active {
                palette.accent
            } else {
                palette.border_strong
            })
            .bg(fill)
            .text_color(palette.fg);
        if let Some(family) = self.mono {
            field = field
                .font_family(family)
                .text_size(px(SELECT_MONO_TEXT_SIZE));
        }
        div()
            .id(box_id)
            .w(px(self.width))
            .h(px(CONTROL_HEIGHT))
            .on_hover(move |is_hovered, _, cx| {
                hovered.update(cx, |hovered, cx| {
                    if *hovered != *is_hovered {
                        *hovered = *is_hovered;
                        cx.notify();
                    }
                });
            })
            .child(field)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::IndexPath;
    use gpui_kit::component::searchable_list::{SearchableListDelegate, SearchableListItem};

    use super::{PaddedOptions, SelectOption, Slot, row_count, slot};

    fn options(n: usize) -> Vec<SelectOption> {
        (0..n)
            .map(|i| SelectOption::new(format!("v{i}"), format!("Label {i}")))
            .collect()
    }

    fn padded(n: usize, rows: usize, matches: usize) -> PaddedOptions {
        let mut padded = PaddedOptions::new(options(n), rows);
        padded.matches = matches;
        padded
    }

    #[test]
    fn a_full_list_has_no_padding() {
        assert_eq!(row_count(40, 12), 40);
        assert_eq!(slot(39, 40, 12), Some(Slot::Match(39)));
        assert_eq!(slot(40, 40, 12), None);
    }

    #[test]
    fn a_filtered_list_keeps_its_rows() {
        assert_eq!(row_count(3, 12), 12);
        assert_eq!(slot(2, 3, 12), Some(Slot::Match(2)));
        assert_eq!(slot(3, 3, 12), Some(Slot::Filler(0)));
        assert_eq!(slot(11, 3, 12), Some(Slot::Filler(8)));
        assert_eq!(slot(12, 3, 12), None);
    }

    #[test]
    fn no_match_reads_no_results_then_fillers() {
        assert_eq!(slot(0, 0, 12), Some(Slot::NoResults));
        assert_eq!(slot(1, 0, 12), Some(Slot::Filler(1)));
        assert_eq!(slot(11, 0, 12), Some(Slot::Filler(11)));
        assert_eq!(slot(12, 0, 12), None);
    }

    #[test]
    fn a_select_without_search_is_never_padded() {
        assert_eq!(row_count(3, 0), 3);
        assert_eq!(slot(3, 3, 0), None);
        assert_eq!(slot(0, 0, 0), None);
    }

    #[test]
    fn the_delegate_lists_matches_then_fillers() {
        let padded = padded(3, 5, 3);
        assert_eq!(padded.items_count(0), 5);
        let at = |row: usize| padded.item(IndexPath::default().row(row));
        assert_eq!(at(1).map(|o| o.label.to_string()), Some("Label 1".into()));
        assert!(at(3).is_some_and(|o| o.disabled));
        assert!(at(5).is_none());
    }

    #[test]
    fn the_first_row_of_an_empty_result_is_the_message() {
        let padded = padded(3, 5, 0);
        let first = padded.item(IndexPath::default().row(0));
        assert!(first.is_some_and(|o| o.disabled && !o.label.is_empty()));
        let second = padded.item(IndexPath::default().row(1));
        assert!(second.is_some_and(|o| o.label.is_empty()));
    }

    #[test]
    fn fillers_have_distinct_values_and_are_disabled() {
        let padded = padded(0, 4, 0);
        let mut values: Vec<_> = padded.fillers.iter().map(|f| f.value.clone()).collect();
        values.push(padded.no_results.value.clone());
        values.sort();
        values.dedup();
        assert_eq!(values.len(), 5);
        assert!(padded.fillers.iter().all(SearchableListItem::disabled));
    }

    #[test]
    fn a_disabled_option_is_not_a_filler() {
        let option = SelectOption::new("a", "A").disabled(true);
        assert!(SearchableListItem::disabled(&option));
    }
}
