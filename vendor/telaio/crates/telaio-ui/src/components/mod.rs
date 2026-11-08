//! The standard components: one file per component, each a `RenderOnce` struct with builder
//! methods (or a function for the smallest ones), reading colors only from [`crate::Palette`]
//! and sizes only from [`crate::metrics`].
//!
//! The `telaio-gallery` binary renders every one of them in every state.

pub mod buttons;
pub mod card;
pub mod chip;
pub mod code_editor;
pub mod control_halo;
pub mod data_table;
pub mod document_tabs;
pub mod edit_menu;
pub mod editable_table;
pub mod empty_state;
pub mod hover_input;
pub mod icon_button;
pub mod inline_message;
pub mod key_value_table;
pub mod menu_rows;
pub mod modal;
pub mod number_stepper;
pub mod progress_bar;
pub mod scrollbar;
pub mod searchable_menu;
pub mod searchable_select;
pub mod section_label;
pub mod segmented_control;
pub mod select_trigger;
pub mod settings;
pub mod shortcuts;
pub mod status_badge;
pub mod switch_halo;
pub mod text_area;
pub mod text_field;
pub mod theme_card;
pub mod time_series_chart;
pub mod tree_row;
pub mod tri_checkbox;
pub mod underline_tabs;

pub use buttons::{DangerButton, GhostButton, NeutralButtonExt, PrimaryButton, SecondaryButton};
pub use card::Card;
pub use chip::Chip;
pub use code_editor::CodeEditor;
pub use control_halo::{
    LabelledControl, checkbox_halo, labelled_checkbox, labelled_radio, radio_halo,
};
pub use data_table::{DataAlign, DataCell, DataColumn, DataSort, DataTable, VirtualDataTable};
pub use document_tabs::{DocumentTab, DocumentTabs};
pub use edit_menu::edit_menu;
pub use editable_table::{EditableCell, EditableColumn, EditableRow, EditableTable};
pub use empty_state::EmptyState;
pub use hover_input::HoverInput;
pub use icon_button::IconButton;
pub use inline_message::{InlineMessage, InlineMessageKind};
pub use key_value_table::{KeyValueRow, KeyValueTable, SortColumn, SortOrder};
pub use menu_rows::{
    menu_choice_item, menu_described_item, menu_shortcut_item, menu_toggle_item, rich_menu_width,
};
pub use modal::{
    ConfirmDialog, MODAL_INNER_RADIUS, ModalPlacement, framed_dialog, modal_body, modal_field,
    modal_footer, modal_header, open_confirm_dialog,
};
pub use number_stepper::NumberStepper;
pub use progress_bar::ProgressBar;
pub use scrollbar::ScrollbarOverlay;
pub use searchable_menu::{
    MenuAction, MenuGroup, MenuRow, SearchableMenu, SearchableMenuEvent, SearchableMenuState,
    SubmenuEntry, SubmenuItem,
};
pub use searchable_select::{
    PaddedOptions, SearchableSelect, SelectHandle, SelectOption, on_pick, pick, select_state,
    set_options,
};
pub use section_label::SectionLabel;
pub use segmented_control::{SegmentedControl, SegmentedItem};
pub use select_trigger::{select_label_row, select_trigger};
pub use settings::{LabeledRow, SectionHeading, SettingsCategory, SettingsLayout, settings_dialog};
pub use shortcuts::{
    MODIFIER_KEY, OPEN_SHORTCUTS_KEYSTROKE, Shortcut, ShortcutGroup, open_hint, open_keys,
    open_shortcuts_dialog, open_tooltip,
};
pub use status_badge::StatusBadge;
pub use switch_halo::switch_halo;
pub use text_area::TextArea;
pub use text_field::{input_line_height, text_field};
pub use theme_card::ThemeCard;
pub use time_series_chart::{
    AxisSide, ChartAxis, ChartLabel, ChartSelection, ChartSeries, LabelPlacement, ReferenceLine,
    SeriesStyle, TimeSeriesChart,
};
pub use tree_row::TreeRow;
pub use tri_checkbox::{CheckState, TriCheckbox};
pub use underline_tabs::{UnderlineTabItem, UnderlineTabs};
