//! `gpui` views: the window's visual tree.
//!
//! Application state lives in [`crate::state`], kept free of `gpui` types. These modules render
//! that state and turn user input (clicks, key bindings) into calls back into
//! [`root::AppView`], the single top-level view that owns the app state.

mod collection_export;
pub mod command_palette;
pub mod completion;
pub mod components;
pub mod define_variable;
pub mod env_color_picker;
pub mod env_editor;
pub mod env_panel;
pub mod env_picker;
pub mod git;
mod import_destination;
mod import_menu;
pub mod load_panel;
pub mod load_test;
pub mod navigation;
pub mod new_workspace;
pub mod openapi_import;
pub mod request_editor;
pub mod response_console;
pub mod response_view;
pub mod root;
pub mod script_completion;
pub mod send;
pub mod settings;
pub mod sidebar;
pub mod snippet_dialog;
pub mod status_bar;
pub mod title_bar;
pub mod update;
pub mod variable_hover;
pub mod welcome;
pub mod whats_new;

pub use root::AppView;
