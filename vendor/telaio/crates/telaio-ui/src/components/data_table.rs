//! [`DataTable`], [`DataColumn`] and [`DataCell`]: a read-only table with a header row, for
//! results (in Postino, the load test per-request table and the run comparison).
//!
//! A plain `div` layout colored from [`crate::Palette`], like [`super::KeyValueTable`], since
//! there is no gpui-kit table widget with this look. Rows are given as plain values and the table
//! builds exactly the rows it is given, so an app with a long list passes only the visible ones.
//!
//! Below its least width (the columns' widths, gaps and padding) the table scrolls sideways
//! inside its card instead of pushing the page wider; the wheel keeps scrolling vertically.
//!
//! A column opts in to sorting with [`DataColumn::sortable`]: its header then shows a neutral
//! up-down icon, or an arrow when the table's [`DataTable::sort`] names it, and a click calls the
//! app's `on_sort` with the column index. The table never sorts anything itself: the app orders
//! its rows and tells the table which column is the sorted one.
//!
//! [`VirtualDataTable`] is the same table for lists that do not fit in memory as elements, up to
//! millions of rows: it asks the app for the rows of the visible range only.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::scroll::{ScrollableElement as _, Scrollbar, ScrollbarAxis};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::key_value_table::SortOrder;
use super::scrollbar::overlay;
use crate::fonts::MONO_FAMILY;
use crate::metrics::{
    DATA_TABLE_COL_GAP, DATA_TABLE_FLEX_MIN_WIDTH, DATA_TABLE_NUMERIC_COL_WIDTH,
    DATA_TABLE_PADDING_X, DATA_TABLE_ROW_HEIGHT, RADIUS_LG, TABLE_SORT_ICON_SIZE,
    VIRTUAL_TABLE_COL_GAP, VIRTUAL_TABLE_HEADER_HEIGHT, VIRTUAL_TABLE_PADDING_X,
    VIRTUAL_TABLE_ROW_HEIGHT, VIRTUAL_TABLE_TEXT_SIZE,
};
use crate::{Palette, PaletteExt};

/// The text size of the header row.
const HEADER_TEXT_SIZE: f32 = 11.5;
/// The text size of the body rows.
const BODY_TEXT_SIZE: f32 = 12.5;
/// Space between a sortable header's label and its icon.
const SORT_ICON_GAP: f32 = 4.0;
/// Opacity of the neutral sort icon of a column that is not the sorted one.
const NEUTRAL_SORT_ICON_OPACITY: f32 = 0.5;
/// Opacity of the rule under each body row, over the card.
const ROW_RULE_OPACITY: f32 = 0.55;

/// The sizes one flavor of table is laid out with.
#[derive(Debug, Clone, Copy)]
struct Look {
    header_height: f32,
    row_height: f32,
    padding_x: f32,
    gap: f32,
    body_text: f32,
    /// The least width of a flexible column.
    flex_min: f32,
}

/// The look of [`DataTable`].
const TABLE_LOOK: Look = Look {
    header_height: DATA_TABLE_ROW_HEIGHT,
    row_height: DATA_TABLE_ROW_HEIGHT,
    padding_x: DATA_TABLE_PADDING_X,
    gap: DATA_TABLE_COL_GAP,
    body_text: BODY_TEXT_SIZE,
    flex_min: DATA_TABLE_FLEX_MIN_WIDTH,
};

/// The look of [`VirtualDataTable`]: tighter, and its flexible columns shrink to nothing (the
/// table does not scroll sideways, its cells truncate).
const VIRTUAL_LOOK: Look = Look {
    header_height: VIRTUAL_TABLE_HEADER_HEIGHT,
    row_height: VIRTUAL_TABLE_ROW_HEIGHT,
    padding_x: VIRTUAL_TABLE_PADDING_X,
    gap: VIRTUAL_TABLE_COL_GAP,
    body_text: VIRTUAL_TABLE_TEXT_SIZE,
    flex_min: DATA_TABLE_FLEX_MIN_WIDTH,
};

/// A header click handler, called with the index of the column to sort by.
type SortHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;
/// The column a table is sorted by, with its direction.
pub type DataSort = (usize, SortOrder);

/// How a column's cells are aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataAlign {
    /// Text starts at the left edge (names).
    #[default]
    Left,
    /// Text ends at the right edge (numbers).
    Right,
}

/// How wide a column is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Width {
    /// Takes the space left over, but never less than [`DATA_TABLE_FLEX_MIN_WIDTH`].
    Flex,
    /// A fixed number of pixels.
    Fixed(f32),
}

/// A column of a [`DataTable`] or [`VirtualDataTable`]: its header label, its width, its
/// alignment and whether its header sorts.
#[derive(Debug, Clone)]
pub struct DataColumn {
    label: SharedString,
    width: Width,
    align: DataAlign,
    sortable: bool,
}

impl DataColumn {
    /// A flexible, left aligned column headed `label`.
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            width: Width::Flex,
            align: DataAlign::Left,
            sortable: false,
        }
    }

    /// A numeric column: right aligned, [`DATA_TABLE_NUMERIC_COL_WIDTH`] wide.
    pub fn numeric(label: impl Into<SharedString>) -> Self {
        Self::new(label)
            .width(DATA_TABLE_NUMERIC_COL_WIDTH)
            .align(DataAlign::Right)
    }

    /// Gives the column a fixed width in pixels instead of sharing the leftover space.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Width::Fixed(width);
        self
    }

    /// Sets how the cells and the header are aligned.
    pub fn align(mut self, align: DataAlign) -> Self {
        self.align = align;
        self
    }

    /// Makes the header sortable: it shows a neutral up-down icon (an arrow while it is the sorted
    /// column) and, when the table has an `on_sort` handler, turns `fg` on hover and calls the
    /// handler on click.
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }
}

/// A cell's content: plain text or a caller-supplied element (a badge next to a name).
enum Content {
    Text(SharedString),
    Element(AnyElement),
}

/// One cell of a [`DataTable`] row.
pub struct DataCell {
    content: Content,
    color: Option<Hsla>,
}

impl DataCell {
    /// A cell showing `text`, truncated when it does not fit.
    pub fn text(text: impl Into<SharedString>) -> Self {
        Self {
            content: Content::Text(text.into()),
            color: None,
        }
    }

    /// A cell showing `element` as given.
    pub fn element(element: impl IntoElement) -> Self {
        Self {
            content: Content::Element(element.into_any_element()),
            color: None,
        }
    }

    /// Colors the cell's text, for example a warning or a delta tone; the default is the text
    /// color of the table.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }
}

impl<T: Into<SharedString>> From<T> for DataCell {
    fn from(text: T) -> Self {
        Self::text(text)
    }
}

/// The least width of a table with `columns`: the padding on both sides, the fixed widths, the
/// least width of the flexible columns and the gaps between them.
fn min_width(columns: &[DataColumn], look: Look) -> f32 {
    let widths: f32 = columns
        .iter()
        .map(|column| match column.width {
            Width::Flex => look.flex_min,
            Width::Fixed(width) => width,
        })
        .sum();
    let gaps = columns.len().saturating_sub(1) as f32 * look.gap;
    2.0 * look.padding_x + widths + gaps
}

/// A read-only table in a bordered `raised` card: an optional header row in `fg_subtle`, then one
/// row per [`Self::row`], each line separated by a `border` rule, and an optional totals row
/// ([`Self::total_row`]). Cells are 12.5 px, and the whole table can be set in the monospace font
/// with [`Self::mono`].
#[derive(IntoElement)]
pub struct DataTable {
    id: ElementId,
    columns: Vec<DataColumn>,
    rows: Vec<Vec<DataCell>>,
    total: Option<Vec<DataCell>>,
    header: bool,
    mono: bool,
    sort: Option<DataSort>,
    on_sort: Option<SortHandler>,
}

impl DataTable {
    /// A table with no columns, to give the scrolling area `id`.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            columns: Vec::new(),
            rows: Vec::new(),
            total: None,
            header: true,
            mono: false,
            sort: None,
            on_sort: None,
        }
    }

    /// Adds a column.
    pub fn column(mut self, column: DataColumn) -> Self {
        self.columns.push(column);
        self
    }

    /// Adds a row of cells, one per column in order; a plain `&str` or `String` becomes a text
    /// cell. Missing cells are left empty and extra ones are ignored.
    pub fn row<C: Into<DataCell>>(mut self, cells: impl IntoIterator<Item = C>) -> Self {
        self.rows.push(cells.into_iter().map(Into::into).collect());
        self
    }

    /// Sets the totals row, after every other row: semibold, on a `hover` tint, below a
    /// `border_strong` rule. Like [`Self::row`], one cell per column.
    pub fn total_row<C: Into<DataCell>>(mut self, cells: impl IntoIterator<Item = C>) -> Self {
        self.total = Some(cells.into_iter().map(Into::into).collect());
        self
    }

    /// Shows or hides the header row (shown by default).
    pub fn header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }

    /// Sets the whole table in the monospace font, so numbers line up.
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }

    /// Names the column (by index) the rows are sorted by, and the direction: its header shows an
    /// arrow and the label turns `fg`. `None` (the default) leaves every sortable header neutral.
    pub fn sort(mut self, sort: Option<DataSort>) -> Self {
        self.sort = sort;
        self
    }

    /// Called with the column index when the header of a [`DataColumn::sortable`] column is
    /// clicked. The app reorders its rows and passes the new [`Self::sort`].
    pub fn on_sort(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_sort = Some(Rc::new(handler));
        self
    }
}

/// A row laid out over `columns`: `cells` in order, with the column widths and alignment.
fn line(
    columns: &[DataColumn],
    cells: Vec<Option<AnyElement>>,
    look: Look,
    height: f32,
    text_size: f32,
) -> Div {
    // `w_full`: a virtualized row is laid out on its own, and would otherwise be as wide as
    // its content, giving the flex column a different width on every row.
    let mut row = h_flex()
        .w_full()
        .h(px(height))
        .items_center()
        .gap(px(look.gap))
        .px(px(look.padding_x))
        .text_size(px(text_size));
    for (column, cell) in columns.iter().zip(cells) {
        let mut slot = div();
        slot = match column.width {
            Width::Flex => slot.flex_1().min_w(px(look.flex_min)).overflow_hidden(),
            Width::Fixed(width) => slot.flex_none().w(px(width)),
        };
        if column.align == DataAlign::Right {
            slot = slot.text_right();
        }
        if let Some(element) = cell {
            slot = slot.child(element);
        }
        row = row.child(slot);
    }
    row
}

/// The header cells of `columns`: the plain label, or for a sortable column the label and its
/// sort icon, clickable when `on_sort` is set.
fn header_cells(
    columns: &[DataColumn],
    sort: Option<DataSort>,
    on_sort: Option<&SortHandler>,
    palette: &Palette,
) -> Vec<Option<AnyElement>> {
    columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            if !column.sortable {
                return Some(column.label.clone().into_any_element());
            }
            let sorted = sort.and_then(|(sorted, order)| (sorted == index).then_some(order));
            Some(sortable_header(
                index,
                column,
                sorted,
                on_sort.cloned(),
                palette,
            ))
        })
        .collect()
}

/// The header of a sortable column: the label (in `fg` while it is the sorted one) followed by an
/// 11 px arrow, or by the neutral up-down icon at half opacity. With a handler it is clickable and
/// the label turns `fg` on hover.
fn sortable_header(
    index: usize,
    column: &DataColumn,
    sorted: Option<SortOrder>,
    on_sort: Option<SortHandler>,
    palette: &Palette,
) -> AnyElement {
    let size = px(TABLE_SORT_ICON_SIZE);
    let icon = match sorted {
        Some(SortOrder::Ascending) => {
            Icon::new(gpui_kit::assets::IconName::ArrowUp).with_size(size)
        }
        Some(SortOrder::Descending) => {
            Icon::new(gpui_kit::assets::IconName::ArrowDown).with_size(size)
        }
        None => Icon::new(gpui_kit::assets::IconName::ChevronsUpDown)
            .with_size(size)
            .opacity(NEUTRAL_SORT_ICON_OPACITY),
    };
    let fg = palette.fg;
    h_flex()
        .id(("sort", index))
        .items_center()
        .gap(px(SORT_ICON_GAP))
        .when(column.align == DataAlign::Right, |cell| cell.justify_end())
        .when(sorted.is_some(), |cell| cell.text_color(fg))
        .child(div().min_w_0().truncate().child(column.label.clone()))
        .child(icon)
        .when_some(on_sort, |cell, handler| {
            cell.cursor_pointer()
                .hover(move |style| style.text_color(fg))
                .on_click(move |_, window, cx| handler(index, window, cx))
        })
        .into_any_element()
}

/// The elements of one body row's `cells`, padded with empty cells up to `columns` entries.
fn body_cells(cells: Vec<DataCell>, columns: usize) -> Vec<Option<AnyElement>> {
    let mut cells: Vec<Option<AnyElement>> = cells
        .into_iter()
        .map(|cell| {
            let element = match cell.content {
                Content::Text(text) => div().min_w_0().truncate().child(text).into_any_element(),
                Content::Element(element) => element,
            };
            let element = match cell.color {
                Some(color) => div().text_color(color).child(element).into_any_element(),
                None => element,
            };
            Some(element)
        })
        .collect();
    cells.resize_with(columns, || None);
    cells
}

impl RenderOnce for DataTable {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let Self {
            id,
            columns,
            rows,
            total,
            header,
            mono,
            sort,
            on_sort,
        } = self;
        let table_min_width = min_width(&columns, TABLE_LOOK);

        let header_row = header.then(|| {
            let cells = header_cells(&columns, sort, on_sort.as_ref(), &palette);
            line(
                &columns,
                cells,
                TABLE_LOOK,
                TABLE_LOOK.header_height,
                HEADER_TEXT_SIZE,
            )
            .border_b_1()
            .border_color(palette.border)
            .text_color(palette.fg_subtle)
            .font_weight(FontWeight::MEDIUM)
        });

        // The card's border closes the last row: a border of its own would poke out past the
        // rounded corners, since `overflow_hidden` clips to a plain rectangle.
        let last = rows.len().saturating_sub(1);
        let total_row = total.map(|cells| {
            let cells = body_cells(cells, columns.len());
            line(
                &columns,
                cells,
                TABLE_LOOK,
                TABLE_LOOK.row_height,
                TABLE_LOOK.body_text,
            )
            .border_t_1()
            .border_color(palette.border_strong)
            .bg(palette.hover)
            .font_weight(FontWeight::SEMIBOLD)
        });
        let body = rows.into_iter().enumerate().map(|(index, cells)| {
            let cells = body_cells(cells, columns.len());
            line(
                &columns,
                cells,
                TABLE_LOOK,
                TABLE_LOOK.row_height,
                TABLE_LOOK.body_text,
            )
            // The totals row draws its own, stronger rule above it.
            .when(index < last, |row| row.border_b_1())
            .border_color(palette.border)
        });

        v_flex()
            .border_1()
            .border_color(palette.border)
            .rounded(px(RADIUS_LG))
            .bg(palette.raised)
            .overflow_hidden()
            .when(mono, |this| this.font_family(MONO_FAMILY))
            .child(
                div()
                    .id(id.clone())
                    .w_full()
                    .restrict_scroll_to_axis()
                    .overflow_x_scrollbar()
                    // Keyed by the table's id: by default the scroll state is keyed by this call
                    // site, so every table would share one scroll position.
                    .id(id)
                    .child(
                        v_flex()
                            .min_w(px(table_min_width))
                            .children(header_row)
                            .children(body)
                            .children(total_row),
                    ),
            )
    }
}

/// A callback producing the cells of the rows in a range, one `Vec<DataCell>` per row.
type RowsFn = Rc<dyn Fn(Range<usize>, &mut Window, &mut App) -> Vec<Vec<DataCell>>>;
/// A callback giving the background of a row by its index.
type RowBackgroundFn = Rc<dyn Fn(usize) -> Option<Hsla>>;

/// [`DataTable`] for lists too long to hold as elements: a fixed header over a body that is a gpui
/// `uniform_list`, so only the visible rows are built, from the app's callback. It costs the same
/// with 10 rows or 1,000,000: the table only needs the row count, never the rows themselves.
///
/// It fills the space its parent gives it (the parent must have a definite height, for example a
/// flex column child with `flex_1().min_h_0()`), with the header fixed on top and a vertical
/// scrollbar. Flexible columns shrink and their cells truncate down to their least width; below
/// the table's least width the header and the rows scroll sideways together, like [`DataTable`].
///
/// ```ignore
/// VirtualDataTable::new("requests", rows.len(), move |range, _window, _cx| {
///     range.map(|ix| vec![DataCell::text(rows[ix].id.to_string()), DataCell::text(rows[ix].url.clone())])
///         .collect()
/// })
/// .column(DataColumn::numeric("#").sortable())
/// .column(DataColumn::new("URL").sortable())
/// .sort(Some((0, SortOrder::Ascending)))
/// .on_sort(|column, _window, cx| { /* reorder, then notify */ })
/// .row_background(|ix| errors.contains(&ix).then_some(danger_tint))
/// .empty_message("No request matches the filters")
/// .mono(true)
/// ```
#[derive(IntoElement)]
pub struct VirtualDataTable {
    id: ElementId,
    columns: Vec<DataColumn>,
    row_count: usize,
    rows: RowsFn,
    mono: bool,
    sort: Option<DataSort>,
    on_sort: Option<SortHandler>,
    row_background: Option<RowBackgroundFn>,
    scroll_handle: Option<UniformListScrollHandle>,
    empty_message: Option<SharedString>,
}

impl VirtualDataTable {
    /// A table of `row_count` rows with no columns yet. `rows` is called with the range of rows
    /// that are on screen (a few dozen, never the whole list) and returns their cells in order,
    /// one `Vec<DataCell>` per row; missing cells are left empty, extra ones ignored. It may be
    /// called again on every frame, so it must be cheap (index into the app's data, no sorting).
    pub fn new(
        id: impl Into<ElementId>,
        row_count: usize,
        rows: impl Fn(Range<usize>, &mut Window, &mut App) -> Vec<Vec<DataCell>> + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            columns: Vec::new(),
            row_count,
            rows: Rc::new(rows),
            mono: false,
            sort: None,
            on_sort: None,
            row_background: None,
            scroll_handle: None,
            empty_message: None,
        }
    }

    /// Adds a column.
    pub fn column(mut self, column: DataColumn) -> Self {
        self.columns.push(column);
        self
    }

    /// Sets the body rows (not the header) in the monospace font, so numbers line up.
    pub fn mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }

    /// Names the sorted column and its direction, see [`DataTable::sort`].
    pub fn sort(mut self, sort: Option<DataSort>) -> Self {
        self.sort = sort;
        self
    }

    /// Called with the column index when a sortable header is clicked, see
    /// [`DataTable::on_sort`].
    pub fn on_sort(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_sort = Some(Rc::new(handler));
        self
    }

    /// A background for some rows, by row index (a tinted error row). `None` leaves a row on the
    /// card. Called once per visible row on every frame. Hovering a row layers the hover fill over
    /// its background.
    pub fn row_background(mut self, background: impl Fn(usize) -> Option<Hsla> + 'static) -> Self {
        self.row_background = Some(Rc::new(background));
        self
    }

    /// Uses the app's scroll handle, so it can move the list (`scroll_to_item(0,
    /// ScrollStrategy::Top)` when the filters change). Without one the table keeps its own.
    pub fn track_scroll(mut self, handle: &UniformListScrollHandle) -> Self {
        self.scroll_handle = Some(handle.clone());
        self
    }

    /// The message centered in the body when `row_count` is 0, in `fg_subtle`. Without one an
    /// empty table shows only its header.
    pub fn empty_message(mut self, message: impl Into<SharedString>) -> Self {
        self.empty_message = Some(message.into());
        self
    }
}

impl RenderOnce for VirtualDataTable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let Self {
            id,
            columns,
            row_count,
            rows,
            mono,
            sort,
            on_sort,
            row_background,
            scroll_handle,
            empty_message,
        } = self;
        let look = VIRTUAL_LOOK;
        let table_min_width = min_width(&columns, look);

        // Keyed by the table's id, like the scroll state of the plain table.
        let handle = scroll_handle.unwrap_or_else(|| {
            window
                .use_keyed_state(id.clone(), cx, |_, _| UniformListScrollHandle::new())
                .read(cx)
                .clone()
        });

        let header_row = line(
            &columns,
            header_cells(&columns, sort, on_sort.as_ref(), &palette),
            look,
            look.header_height,
            HEADER_TEXT_SIZE,
        )
        .id((id.clone(), "header"))
        .flex_none()
        .border_b_1()
        .border_color(palette.border)
        .text_color(palette.fg_subtle)
        .font_weight(FontWeight::MEDIUM);

        let body = if row_count == 0 {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p(px(40.0))
                .text_center()
                .text_color(palette.fg_subtle)
                .children(empty_message)
                .into_any_element()
        } else {
            let rule = palette.border.opacity(ROW_RULE_OPACITY);
            let hover = palette.hover;
            let columns = Rc::new(columns);
            let tabular = FontFeatures(Arc::new(vec![("tnum".to_string(), 1)]));
            let list = uniform_list((id.clone(), "rows"), row_count, move |range, window, cx| {
                let start = range.start;
                rows(range, window, cx)
                    .into_iter()
                    .enumerate()
                    .map(|(offset, cells)| {
                        let index = start + offset;
                        let background = row_background.as_ref().and_then(|f| f(index));
                        let hover = background.map_or(hover, |color| color.blend(hover));
                        line(
                            &columns,
                            body_cells(cells, columns.len()),
                            look,
                            look.row_height,
                            look.body_text,
                        )
                        .id(("row", index))
                        .border_b_1()
                        .border_color(rule)
                        .font_features(tabular.clone())
                        .when(mono, |row| row.font_family(MONO_FAMILY))
                        .when_some(background, |row, color| row.bg(color))
                        .hover(move |style| style.bg(hover))
                    })
                    .collect()
            })
            .track_scroll(&handle)
            .size_full();
            div().flex_1().min_h_0().child(list).into_any_element()
        };
        // A plain gpui scroll area, not gpui-kit's `overflow_x_scrollbar`, which turns the vertical
        // wheel into a sideways scroll and so would scroll both ways at once over the rows.
        let columns_id = ElementId::from((id.clone(), "columns"));
        let columns_handle = window
            .use_keyed_state(columns_id.clone(), cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        let sideways = div().absolute().inset_0().child(
            Scrollbar::new(&columns_handle)
                .axis(ScrollbarAxis::Horizontal)
                .id((id.clone(), "columns-scrollbar")),
        );
        // Over the visible body, not inside the sideways scroll, so it stays in view.
        let scrollbar = (row_count > 0).then(|| {
            overlay(&handle, Some((id.clone(), "scrollbar").into()), &palette)
                .top(px(look.header_height + 1.0))
        });

        v_flex()
            .relative()
            .size_full()
            .min_h_0()
            .border_1()
            .border_color(palette.border)
            .rounded(px(RADIUS_LG))
            .bg(palette.raised)
            .overflow_hidden()
            .child(
                div()
                    .id(columns_id)
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .restrict_scroll_to_axis()
                    .overflow_x_scroll()
                    .track_scroll(&columns_handle)
                    .child(
                        v_flex()
                            .h_full()
                            .w_full()
                            .min_w(px(table_min_width))
                            .child(header_row)
                            .child(body),
                    ),
            )
            .children(scrollbar)
            .child(sideways)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DATA_TABLE_COL_GAP, DATA_TABLE_FLEX_MIN_WIDTH, DATA_TABLE_NUMERIC_COL_WIDTH,
        DATA_TABLE_PADDING_X, DataAlign, DataColumn, TABLE_LOOK, VIRTUAL_LOOK, Width, min_width,
    };

    #[test]
    fn min_width_adds_padding_widths_and_gaps() {
        let columns = [
            DataColumn::new("Name"),
            DataColumn::numeric("Count"),
            DataColumn::new("Other").width(100.0),
        ];
        let expected = 2.0 * DATA_TABLE_PADDING_X
            + DATA_TABLE_FLEX_MIN_WIDTH
            + DATA_TABLE_NUMERIC_COL_WIDTH
            + 100.0
            + 2.0 * DATA_TABLE_COL_GAP;
        assert_eq!(min_width(&columns, TABLE_LOOK), expected);
    }

    #[test]
    fn an_empty_table_is_only_its_padding() {
        assert_eq!(min_width(&[], TABLE_LOOK), 2.0 * DATA_TABLE_PADDING_X);
    }

    #[test]
    fn a_numeric_column_is_right_aligned_and_fixed() {
        let column = DataColumn::numeric("p95");
        assert_eq!(column.align, DataAlign::Right);
        assert_eq!(column.width, Width::Fixed(DATA_TABLE_NUMERIC_COL_WIDTH));
    }

    #[test]
    fn columns_are_not_sortable_until_asked() {
        assert!(!DataColumn::new("Name").sortable);
        assert!(DataColumn::new("Name").sortable().sortable);
    }

    #[test]
    fn the_virtual_look_matches_the_design_column_template() {
        // `84px 92px 56px minmax(0,1fr) 72px 200px 72px`, gap 12, padding 14, except that the
        // flexible column keeps the plain table's least width before the table scrolls sideways.
        let columns = [
            DataColumn::new("#").width(84.0),
            DataColumn::new("t").width(92.0),
            DataColumn::new("VU").width(56.0),
            DataColumn::new("URL"),
            DataColumn::new("status").width(72.0),
            DataColumn::new("latency").width(200.0),
            DataColumn::new("size").width(72.0),
        ];
        assert_eq!(
            min_width(&columns, VIRTUAL_LOOK),
            2.0 * 14.0
                + 84.0
                + 92.0
                + 56.0
                + DATA_TABLE_FLEX_MIN_WIDTH
                + 72.0
                + 200.0
                + 72.0
                + 6.0 * 12.0
        );
    }
}
