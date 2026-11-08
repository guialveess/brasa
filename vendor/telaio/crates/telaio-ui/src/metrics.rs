//! Named layout constants shared by every app: control heights, the radius scale and the sizes
//! of the standard components. Components read sizes only from here, never as a literal, the
//! same rule the palette follows for color. Layout sizes of one app (its sidebar, its title bar)
//! stay in that app.

/// Height of ordinary controls (inputs, buttons, select triggers).
pub const CONTROL_HEIGHT: f32 = 34.0;
/// Height of a menu row.
pub const MENU_ROW_HEIGHT: f32 = 32.0;
/// Height of a segmented control's inner items.
pub const SEGMENTED_CONTROL_INNER_HEIGHT: f32 = 25.0;
/// Height of an `UnderlineTabs` strip.
pub const UNDERLINE_TABS_HEIGHT: f32 = 36.0;
/// Height of a `DocumentTabs` strip.
pub const DOCUMENT_TABS_HEIGHT: f32 = 40.0;
/// Text size of a document tab's detail, after its label.
pub const DOCUMENT_TAB_DETAIL_TEXT_SIZE: f32 = 12.0;
/// Width of the overflow controls of a `DocumentTabs` strip (the two arrows and the list button
/// with their gaps and padding). Fixed, so the strip can tell whether the tabs overflow with and
/// without them.
pub const DOCUMENT_TABS_CONTROLS_WIDTH: f32 = 88.0;
/// Extra space between a document tab's label and its close button, which sits that much closer
/// to the tab's right edge.
pub const DOCUMENT_TAB_CLOSE_SHIFT: f32 = 4.0;
/// Horizontal padding around the document tabs' suffix, so it does not touch the last tab.
pub const DOCUMENT_TABS_SUFFIX_PADDING: f32 = 6.0;

/// Radius for chips and checkboxes, the smallest step of the radius scale.
pub const RADIUS_XS: f32 = 9.0;
/// Radius for segmented control items, tree rows and badges. Close to half the segmented
/// control's inner height, so its items read as pills.
pub const RADIUS_SM: f32 = 12.0;
/// Radius for inputs, buttons and cards in editors. Close to half `CONTROL_HEIGHT`, so ordinary
/// buttons and inputs read as pills, like a capsule search field or a pill call to action.
pub const RADIUS_MD: f32 = 17.0;
/// Radius for menus, popovers and dashboard cards.
pub const RADIUS_LG: f32 = 20.0;
/// Radius for modals.
pub const RADIUS_MODAL: f32 = 24.0;
/// Height of a modal's header strip and of its footer strip (the dialog buttons).
pub const MODAL_HEADER_HEIGHT: f32 = 56.0;
/// Width of a small modal (a confirmation, a name prompt): gpui-component's default dialog width.
pub const MODAL_WIDTH_DEFAULT: f32 = 448.0;

/// Height of a theme card's preview.
pub const THEME_PREVIEW_HEIGHT: f32 = 92.0;
/// Padding between a theme card's frame and the mini window it shows.
pub const THEME_PREVIEW_PADDING: f32 = 8.0;

/// Height of a `StatusBadge`.
pub const STATUS_BADGE_HEIGHT: f32 = 24.0;
/// Side length of an `IconButton`'s small size.
pub const ICON_BUTTON_SM: f32 = 26.0;
/// Side length of an `IconButton`'s large size.
pub const ICON_BUTTON_LG: f32 = 30.0;
/// Side length of the icon buttons in a panel header: the small icon in a slightly larger box,
/// so the hover background has more room.
pub const ICON_BUTTON_HEADER: f32 = 24.0;
/// Horizontal padding of a single-line input's text, 1 px less than gpui-component's default 10.
pub const INPUT_PADDING_X: f32 = 12.0;
/// Padding around a checkbox's 16 px box, the halo it shows on hover (22 px in all).
pub const CHECKBOX_HALO_PADDING: f32 = 3.0;

/// Width of the checkbox and delete columns of a `KeyValueTable`.
pub const KEY_VALUE_SIDE_COL_WIDTH: f32 = 32.0;
/// Height of a `KeyValueTable` row: its fields are 4 px clear of the row lines above and below.
pub const KEY_VALUE_ROW_HEIGHT: f32 = 38.0;
/// Space between the key and the value field of a `KeyValueTable` row, half on each side.
pub const KEY_VALUE_CELL_GAP: f32 = 6.0;

/// Width of the keyboard shortcuts cheat sheet.
pub const SHORTCUTS_WIDTH: f32 = 520.0;
/// Height of the cheat sheet on a tall enough window.
pub const SHORTCUTS_HEIGHT: f32 = 620.0;
/// Height, and least width, of a key chip in the cheat sheet.
pub const SHORTCUTS_KEY_SIZE: f32 = 22.0;

/// Height of a `DataTable` row, header included.
pub const DATA_TABLE_ROW_HEIGHT: f32 = 38.0;
/// Horizontal padding of a `DataTable` row.
pub const DATA_TABLE_PADDING_X: f32 = 18.0;
/// Space between the cells of a `DataTable` row.
pub const DATA_TABLE_COL_GAP: f32 = 12.0;
/// Width of a numeric `DataTable` column, wide enough for "1,234 ms" in the mono font.
pub const DATA_TABLE_NUMERIC_COL_WIDTH: f32 = 64.0;
/// Least width of a flexible `DataTable` column before the table scrolls sideways.
pub const DATA_TABLE_FLEX_MIN_WIDTH: f32 = 60.0;
/// Height of the header row of a `VirtualDataTable`.
pub const VIRTUAL_TABLE_HEADER_HEIGHT: f32 = 32.0;
/// Height of a body row of a `VirtualDataTable`. The list is uniform, so every row has it.
pub const VIRTUAL_TABLE_ROW_HEIGHT: f32 = 30.0;
/// Horizontal padding of a `VirtualDataTable` row.
pub const VIRTUAL_TABLE_PADDING_X: f32 = 14.0;
/// Space between the cells of a `VirtualDataTable` row.
pub const VIRTUAL_TABLE_COL_GAP: f32 = 12.0;
/// Text size of the body rows of a `VirtualDataTable`.
pub const VIRTUAL_TABLE_TEXT_SIZE: f32 = 12.0;
/// Size of the sort arrow or the neutral sort icon in a sortable table header.
pub const TABLE_SORT_ICON_SIZE: f32 = 11.0;

/// Height of a `Chip`.
pub const CHIP_HEIGHT: f32 = 26.0;
/// Left padding of a `Chip`, and its right padding when it has no dismiss button.
pub const CHIP_PADDING_LEFT: f32 = 10.0;
/// Right padding of a `Chip` with a dismiss button.
pub const CHIP_PADDING_RIGHT: f32 = 4.0;
/// Space between the parts of a `Chip`.
pub const CHIP_GAP: f32 = 6.0;
/// Text size of a `Chip`.
pub const CHIP_TEXT_SIZE: f32 = 12.0;
/// Size of the icons of a `Chip`, the leading one and the dismiss cross.
pub const CHIP_ICON_SIZE: f32 = 12.0;
/// Side length of the dismiss button of a `Chip`.
pub const CHIP_DISMISS_SIZE: f32 = 18.0;

/// Height of a single line row of a rich popup menu (choice and shortcut rows).
pub const RICH_MENU_ROW_HEIGHT: f32 = 33.0;
/// Text size of the labels of a rich popup menu.
pub const RICH_MENU_TEXT_SIZE: f32 = 12.5;
/// Text size of the details, descriptions and hints of a rich popup menu.
pub const RICH_MENU_DETAIL_SIZE: f32 = 12.0;
/// Text size of the monospace tags and shortcut hints of a rich popup menu.
pub const RICH_MENU_TAG_SIZE: f32 = 11.0;
/// Size of the check box of a toggle row of a rich popup menu.
pub const RICH_MENU_CHECK_SIZE: f32 = 14.0;

/// Side length of the icon of an `EmptyState`.
pub const EMPTY_STATE_ICON_SIZE: f32 = 28.0;
/// Vertical padding of a compact `EmptyState`, which sits in the flow of a page instead of
/// filling its container.
pub const EMPTY_STATE_COMPACT_PADDING: f32 = 40.0;
/// Widest an `EmptyState` explanation grows before it wraps.
pub const EMPTY_STATE_DESCRIPTION_MAX_WIDTH: f32 = 360.0;

/// Height of a `TreeRow`.
pub const TREE_ROW_HEIGHT: f32 = 31.0;
/// Width of a `TreeRow`'s chevron slot, kept (empty) on a leaf so a leaf lines up with its
/// sibling folders' chevrons.
pub const TREE_ROW_ICON_WIDTH: f32 = 13.0;
/// How far each depth level indents a `TreeRow`.
pub const TREE_ROW_INDENT: f32 = 16.0;
/// Left padding of a `TreeRow` at depth 0.
pub const TREE_ROW_PADDING_LEFT: f32 = 10.0;
/// Extra space after a `TreeRow`'s leading element, on top of the row's 4 px gap.
pub const TREE_ROW_LEADING_GAP: f32 = 2.0;
/// Text size of a `TreeRow`'s detail, after its label.
pub const TREE_ROW_DETAIL_TEXT_SIZE: f32 = 11.5;

/// Height of the header row of an `EditableTable`.
pub const EDITABLE_TABLE_HEADER_HEIGHT: f32 = 34.0;
/// Height of an `EditableTable` row (a row with a note under a cell grows past it).
pub const EDITABLE_TABLE_ROW_HEIGHT: f32 = 38.0;
/// Horizontal padding of an `EditableTable` header label and of its read only and element cells.
pub const EDITABLE_TABLE_PADDING_X: f32 = 12.0;
/// Horizontal padding of an `EditableTable` input cell, 8 px less than the read only cells: the
/// input has padding of its own.
pub const EDITABLE_TABLE_INPUT_PADDING_X: f32 = 4.0;
/// Nominal line height of an `EditableTable` input, which is also the height of its selection
/// highlight. The real one is the whole device pixel size closest to it (see
/// `text_field::input_line_height`).
pub const EDITABLE_TABLE_LINE_HEIGHT: f32 = 22.0;
/// Text size of the body of an `EditableTable`, inputs included.
pub const EDITABLE_TABLE_TEXT_SIZE: f32 = 12.5;
/// Text size of the header labels and of the notes of an `EditableTable`.
pub const EDITABLE_TABLE_SMALL_TEXT_SIZE: f32 = 11.0;

/// Blur radius of the glow under a hovered primary or danger button.
pub const BUTTON_GLOW_BLUR: f32 = 10.0;
/// Vertical offset of that glow, so it reads as light falling a little below the button.
pub const BUTTON_GLOW_OFFSET_Y: f32 = 2.0;
/// Opacity of the button's own color in that glow.
pub const BUTTON_GLOW_OPACITY: f32 = 0.35;

/// Height of a `ProgressBar`'s track.
pub const PROGRESS_BAR_HEIGHT: f32 = 6.0;

/// Space above a `TimeSeriesChart`'s plot, so the top gridline and the line at the maximum fit.
pub const CHART_PAD_TOP: f32 = 8.0;
/// Space above a `TimeSeriesChart`'s plot when an axis has a title, which sits in that space.
pub const CHART_PAD_TOP_TITLED: f32 = 24.0;
/// Space below a `TimeSeriesChart`'s plot, for the x axis labels.
pub const CHART_PAD_BOTTOM: f32 = 24.0;
/// Default width of the strip left of a chart's plot, for the left y axis labels.
pub const CHART_GUTTER_LEFT: f32 = 48.0;
/// Default width of the strip right of a chart's plot, for the right y axis labels.
pub const CHART_GUTTER_RIGHT: f32 = 44.0;
/// Space between a chart's plot and its y axis labels.
pub const CHART_TICK_GAP: f32 = 6.0;
/// Space between the bottom of a chart's plot and its x axis labels.
pub const CHART_X_LABEL_GAP: f32 = 6.0;
/// Width of one x axis label of a chart, which is centered on its tick.
pub const CHART_X_LABEL_WIDTH: f32 = 40.0;
/// Text size of a chart's axis labels and reference line labels, in the monospace family.
pub const CHART_TICK_TEXT_SIZE: f32 = 10.5;
/// Width of a chart's hover tooltip.
pub const CHART_TOOLTIP_WIDTH: f32 = 184.0;
/// Space between the hover crosshair and the tooltip.
pub const CHART_TOOLTIP_OFFSET: f32 = 12.0;
/// Space the tooltip keeps from the right edge of the plot before it flips to the other side of
/// the crosshair.
pub const CHART_TOOLTIP_EDGE: f32 = 14.0;
/// Space between the top of a chart's plot and its tooltip.
pub const CHART_TOOLTIP_TOP: f32 = 2.0;
/// Height of a chart label's box.
pub const CHART_LABEL_HEIGHT: f32 = 18.0;
/// Space between a chart label and the point it labels.
pub const CHART_LABEL_GAP: f32 = 5.0;
/// Diameter of the dots a chart's hover marks on each series, ring included.
pub const CHART_DOT_SIZE: f32 = 9.0;

/// Width of a `NumberStepper`'s readout, wide enough for "12.5 px" in the mono font.
pub const STEPPER_READOUT_WIDTH: f32 = 64.0;
/// Gap between a `NumberStepper`'s buttons and its readout.
pub const STEPPER_GAP: f32 = 8.0;

/// Width of a `SearchableSelect` box.
pub const SELECT_WIDTH: f32 = 220.0;
/// Rows a searchable select always lists, filtered or not, so its list keeps one height.
pub const SELECT_SEARCH_ROWS: usize = 12;
/// Maximum height of a select's list, which scrolls beyond it. Just above [`SELECT_SEARCH_ROWS`]
/// rows (20 px each, plus the list's padding), so a padded list never scrolls.
pub const SELECT_MENU_MAX_HEIGHT: f32 = 252.0;
/// Text size of a select's label when it is set in the monospace family, as in the editors.
pub const SELECT_MONO_TEXT_SIZE: f32 = 12.5;

/// Width of the Settings modal.
pub const SETTINGS_WIDTH: f32 = 800.0;
/// Height of the Settings modal.
pub const SETTINGS_HEIGHT: f32 = 720.0;
/// Width of the Settings modal's category navigation column.
pub const SETTINGS_NAV_WIDTH: f32 = 188.0;

/// Width of a `SearchableMenu` panel.
pub const SEARCHABLE_MENU_WIDTH: f32 = 440.0;
/// Height of the search input strip of a `SearchableMenu`.
pub const SEARCHABLE_MENU_SEARCH_HEIGHT: f32 = 40.0;
/// Height of the buttons of the action row of a `SearchableMenu`.
pub const SEARCHABLE_MENU_ACTION_HEIGHT: f32 = 28.0;
/// Height of a row of a `SearchableMenu`.
pub const SEARCHABLE_MENU_ROW_HEIGHT: f32 = 42.0;
/// Height of a group label of a `SearchableMenu`, without the 4 px above it.
pub const SEARCHABLE_MENU_LABEL_HEIGHT: f32 = 24.0;
/// Space above a group label of a `SearchableMenu`.
pub const SEARCHABLE_MENU_LABEL_GAP: f32 = 4.0;
/// Height of the key hint bar of a `SearchableMenu`.
pub const SEARCHABLE_MENU_FOOTER_HEIGHT: f32 = 32.0;
/// Most the rows of a `SearchableMenu` take in height before they scroll.
pub const SEARCHABLE_MENU_LIST_MAX_HEIGHT: f32 = 380.0;
/// Width of the submenu of a row of a `SearchableMenu`.
pub const SEARCHABLE_MENU_SUBMENU_WIDTH: f32 = 220.0;
/// Height of an item of the submenu of a `SearchableMenu`.
pub const SEARCHABLE_MENU_SUBMENU_ROW_HEIGHT: f32 = 26.0;
/// Text size of the title of a `SearchableMenu` row (monospace).
pub const SEARCHABLE_MENU_TITLE_TEXT_SIZE: f32 = 12.5;
/// Text size of the subtitle of a `SearchableMenu` row and of its key hints.
pub const SEARCHABLE_MENU_SUBTITLE_TEXT_SIZE: f32 = 11.5;
/// Text size of the badge of a `SearchableMenu` row and of its group labels.
pub const SEARCHABLE_MENU_BADGE_TEXT_SIZE: f32 = 11.0;
/// Gap between the trigger of a `SearchableMenu` and its panel.
pub const SEARCHABLE_MENU_GAP: f32 = 6.0;
