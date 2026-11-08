//! The What's new tab (#189): a header with the version and a link to its GitHub release, the
//! release notes rendered as Markdown, and the "Show after an update" checkbox.
//!
//! The notes are embedded in the binary (`state::whats_new::NOTES`). They are parsed once, the
//! first time the tab opens, into a [`TextViewState`] kept for the session, so render does no
//! Markdown work.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;
use telaio_ui::components::labelled_checkbox;

use crate::state::whats_new::{self, NOTES, VERSION};
use crate::theme::PaletteExt;
use crate::theme::metrics::{WELCOME_COLUMN_WIDTH_NARROW, WELCOME_TOP_PADDING_NARROW};

use super::components::SecondaryButton;
use super::root::AppView;

impl AppView {
    /// Opens the What's new tab, or brings it to the front. Also the command palette's "Show
    /// what's new" action and the About pane's button.
    pub(crate) fn open_whats_new_tab(&mut self, cx: &mut Context<Self>) {
        if self.whats_new_text.is_none() {
            let text = match NOTES {
                Some(notes) => whats_new::link_issues(notes, brasa_cli::IDENTITY.repository),
                None => t!("whats_new.no_notes").into_owned(),
            };
            self.whats_new_text = Some(cx.new(|cx| TextViewState::markdown(&text, cx)));
        }
        self.state.tabs.open_whats_new();
        log::debug!("opened the what's new tab");
        cx.notify();
    }

    /// Renders the active What's new tab.
    pub(crate) fn render_whats_new(&self, cx: &Context<Self>) -> AnyElement {
        let palette = cx.palette();
        let weak = cx.weak_entity();
        let release = whats_new::release_url(brasa_cli::IDENTITY.repository, VERSION);
        let header = h_flex()
            .items_start()
            .justify_between()
            .gap(px(16.0))
            .pb(px(20.0))
            .border_b_1()
            .border_color(palette.border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(24.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette.fg)
                            .child(t!("whats_new.title", version = VERSION)),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(palette.fg_muted)
                            .child(t!("whats_new.subtitle")),
                    ),
            )
            .child(
                SecondaryButton::new("whats-new-release", t!("whats_new.release_page"))
                    .icon(Icon::new(Lucide::ExternalLink))
                    .on_click(move |_, _, cx| cx.open_url(&release)),
            );
        let notes = self.whats_new_text.as_ref().map(|state| {
            div()
                .pt(px(20.0))
                .text_size(px(14.0))
                .text_color(palette.fg)
                .child(TextView::new(state))
        });
        let footer = h_flex()
            .pt(px(16.0))
            .border_t_1()
            .border_color(palette.border)
            .text_size(px(12.5))
            .child(div().flex_1())
            .child(
                labelled_checkbox(
                    "whats-new-show-after-update",
                    self.state.settings.show_whats_new,
                    t!("whats_new.show_after_update"),
                    &palette,
                )
                .on_change(move |checked, window, cx| {
                    let checked = *checked;
                    let _ =
                        weak.update(cx, |view, cx| view.set_show_whats_new(checked, window, cx));
                }),
            );
        let column = v_flex()
            .w_full()
            .max_w(px(WELCOME_COLUMN_WIDTH_NARROW))
            .pt(px(WELCOME_TOP_PADDING_NARROW))
            .px(px(40.0))
            .pb(px(32.0))
            .child(header)
            .children(notes)
            .child(div().flex_1().min_h(px(24.0)))
            .child(footer);
        let area = div()
            .id("whats-new-tab")
            .size_full()
            .bg(palette.bg)
            .overflow_y_scroll()
            .track_scroll(&self.whats_new_scroll)
            .child(
                h_flex()
                    .w_full()
                    .min_h_full()
                    .items_stretch()
                    .justify_center()
                    .child(column),
            );
        // Not `overflow_scrollbar`, for the same reason as the welcome tab: its wrapper
        // stretches the content to the area's height, so the tab would no longer scroll.
        div()
            .relative()
            .size_full()
            .child(area)
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(Scrollbar::vertical(&self.whats_new_scroll)),
            )
            .into_any_element()
    }
}
