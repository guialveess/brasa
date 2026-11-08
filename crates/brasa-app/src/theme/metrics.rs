//! Named layout constants (heights, widths, radii, listed in `docs/design-system.md`). Views
//! read sizes only from here, never as a literal, so every size in the app traces back to one
//! place, the same rule the palette (`theme::palette`) follows for color.
//!
//! Some constants are kept for sizes the design defines even when no view reads them yet, so
//! this file allows dead code for itself.
#![allow(dead_code)]

// The sizes of the standard components and the radius scale, shared with Vano and Nastro.
pub use telaio_ui::metrics::*;

/// Height of the title bar.
pub const TITLE_BAR_HEIGHT: f32 = 46.0;
/// Height of the response viewer's tab bar.
pub const RESPONSE_TAB_BAR_HEIGHT: f32 = 40.0;
/// Width of the name column of the response Headers tab, enough for most standard header names
/// (`access-control-allow-credentials` wraps).
pub const HEADERS_NAME_WIDTH: f32 = 220.0;
/// Height of the response Headers tab's column label row, which also holds its copy button.
pub const HEADERS_HEADER_HEIGHT: f32 = 32.0;
/// Vertical padding of a response header row; its height follows its wrapped value.
pub const HEADERS_ROW_PADDING_Y: f32 = 7.0;
/// Horizontal padding of the response Headers tab rows, the same as telaio's data table.
pub const HEADERS_PADDING_X: f32 = DATA_TABLE_PADDING_X;
/// Gap between the name and value columns of the response Headers tab.
pub const HEADERS_COL_GAP: f32 = 16.0;
/// Line height of a response header's name and value, so a wrapped value reads as one block.
pub const HEADERS_LINE_HEIGHT: f32 = 20.0;
/// Height of the URL bar and the primary Send button.
pub const URL_BAR_HEIGHT: f32 = 38.0;
/// Height of the primary Send button. Equal to [`URL_BAR_HEIGHT`], named separately because the
/// two are independent design decisions that happen to share a value.
pub const SEND_BUTTON_HEIGHT: f32 = 38.0;
/// Minimum width of the primary Send button (label plus key hint), a few px wider than its
/// natural width so it does not look cramped.
pub const SEND_BUTTON_MIN_WIDTH: f32 = 104.0;
/// Height of a sidebar tree row. Shadows telaio's own `TREE_ROW_HEIGHT`, kept in sync with it.
pub const TREE_ROW_HEIGHT: f32 = 31.0;
/// Height of the status bar.
pub const STATUS_BAR_HEIGHT: f32 = 28.0;
/// Side length of the status bar's keyboard shortcuts button, a little under its height.
pub const STATUS_BAR_BUTTON: f32 = 22.0;

/// Default sidebar width.
pub const SIDEBAR_WIDTH: f32 = 260.0;
/// Minimum sidebar width when resizing.
pub const SIDEBAR_WIDTH_MIN: f32 = 180.0;
/// Maximum sidebar width when resizing.
pub const SIDEBAR_WIDTH_MAX: f32 = 480.0;
/// Width of the activity rail left of the sidebar.
pub const RAIL_WIDTH: f32 = 52.0;
/// Side length of an activity rail button.
pub const RAIL_BUTTON_SIZE: f32 = 36.0;
/// Height of a row of the Environments panel.
pub const ENV_ROW_HEIGHT: f32 = 34.0;
/// Width of the command palette's search trigger in the title bar.
pub const SEARCH_TRIGGER_WIDTH: f32 = 360.0;
/// Width (and height) of each window control button (minimize, maximize, close).
pub const WINDOW_CONTROL_WIDTH: f32 = 40.0;

/// Fixed width of a `MethodBadge`'s `label` variant, and of the method label in a sidebar tree
/// row or open tab.
pub const METHOD_LABEL_WIDTH: f32 = 34.0;
/// Height of a `VariableField` (its input or its row of chips).
pub const VARIABLE_FIELD_HEIGHT: f32 = 28.0;
/// Width of the column that centers a colored dot in an `EnvMenu` row.
pub const MENU_DOT_COLUMN_WIDTH: f32 = 13.0;
/// Height of the title bar's `EnvPill` trigger (see `docs/design-system.md`).
pub const ENV_PILL_HEIGHT: f32 = 30.0;
/// Height of the sidebar's filter input (see `docs/design-system.md`).
pub const SIDEBAR_FILTER_HEIGHT: f32 = 32.0;
/// Width of the autocompletion menu of the request editor's fields.
pub const COMPLETION_MENU_WIDTH: f32 = 340.0;
/// Rows the autocompletion menu shows before it scrolls.
pub const COMPLETION_MENU_MAX_ROWS: f32 = 8.0;
/// Maximum width of the Auth tab's fields, label included.
pub const AUTH_FORM_MAX_WIDTH: f32 = 560.0;
/// Width of the labels in front of the Auth tab's fields.
pub const AUTH_LABEL_WIDTH: f32 = 96.0;

/// Maximum width of the welcome tab's content column when it fills the window (no workspace
/// open, so no sidebar).
pub const WELCOME_COLUMN_WIDTH: f32 = 820.0;
/// Maximum width of the welcome tab's content column next to the sidebar.
pub const WELCOME_COLUMN_WIDTH_NARROW: f32 = 760.0;
/// Least width of the welcome tab's content column, padding included. A narrower tab scrolls
/// sideways instead of cutting the files card, the theme cards and the footer.
pub const WELCOME_MIN_WIDTH: f32 = 640.0;
/// Space above the welcome tab's content when it fills the window.
pub const WELCOME_TOP_PADDING: f32 = 72.0;
/// Space above the welcome tab's content next to the sidebar.
pub const WELCOME_TOP_PADDING_NARROW: f32 = 56.0;
/// Side length of the welcome tab's "B" logo square: a quiet inline mark next to the wordmark,
/// not a big stacked hero.
pub const WELCOME_LOGO_SIZE: f32 = 32.0;
/// Corner radius of the welcome tab's logo square.
pub const WELCOME_LOGO_RADIUS: f32 = 10.0;
/// Height of a row of the welcome tab's Start and Recent lists.
pub const WELCOME_ROW_HEIGHT: f32 = 38.0;
/// Width of a recent workspace's name in the welcome tab, before its path.
pub const WELCOME_RECENT_NAME_WIDTH: f32 = 80.0;
