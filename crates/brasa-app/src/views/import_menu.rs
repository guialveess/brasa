//! The "Import" dropdown in the sidebar's header. Every entry opens the import wizard, which works
//! with or without a workspace (`views/openapi_import.rs`).

use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::theme::PaletteExt;
use crate::theme::metrics::ICON_BUTTON_HEADER;
use crate::views::components::NeutralButtonExt;

use super::root::AppView;

impl AppView {
    /// Renders the "Import" dropdown button, an icon button styled like the sidebar header's
    /// `IconButton`s next to it.
    pub(crate) fn render_import_menu(&self, cx: &Context<Self>) -> AnyElement {
        let weak = cx.weak_entity();
        Button::new("import-menu")
            .neutral_ghost(cx, cx.palette().fg_muted)
            .small()
            .size(px(ICON_BUTTON_HEADER))
            .icon(Icon::new(gpui_kit::assets::IconName::Download))
            .text_color(cx.palette().fg_muted)
            .tooltip(t!("shell.import.button"))
            .dropdown_menu(move |menu, _, _| {
                let openapi_weak = weak.clone();
                let postman_weak = weak.clone();
                let bruno_weak = weak.clone();
                menu.item(
                    PopupMenuItem::new(t!("openapi.menu"))
                        .icon(Icon::new(gpui_kit::assets::IconName::FileCode))
                        .on_click(move |_, window, cx| {
                            super::openapi_import::open_openapi_wizard(
                                openapi_weak.clone(),
                                crate::state::openapi_import::SourceKind::File,
                                window,
                                cx,
                            );
                        }),
                )
                .item(
                    PopupMenuItem::new(t!("import.menu_postman"))
                        .icon(Icon::new(gpui_kit::assets::IconName::Package))
                        .on_click(move |_, window, cx| {
                            super::openapi_import::open_postman_wizard(
                                postman_weak.clone(),
                                window,
                                cx,
                            );
                        }),
                )
                .item(
                    PopupMenuItem::new(t!("import.menu_bruno"))
                        .icon(Icon::new(gpui_kit::assets::IconName::FolderInput))
                        .on_click(move |_, window, cx| {
                            super::openapi_import::open_bruno_wizard(
                                bruno_weak.clone(),
                                window,
                                cx,
                            );
                        }),
                )
            })
            .into_any_element()
    }
}
