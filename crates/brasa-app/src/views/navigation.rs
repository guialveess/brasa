//! The activity rail (GitHub #64): a 48 px column left of the sidebar with one button per
//! section (Collections, Source Control, Environments, Load tests), and the dispatcher that renders the active
//! section's panel. The state (which section, collapsed or not) is `state::navigation`; the
//! panels are `views/sidebar.rs`, `views/env_panel.rs` and `views/load_panel.rs`.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::actions::{ToggleCollections, ToggleEnvironments, ToggleLoadTests, ToggleSourceControl};
use crate::state::navigation::NavSection;
use crate::theme::PaletteExt;
use crate::theme::metrics::{RAIL_BUTTON_RADIUS, RAIL_BUTTON_SIZE, RAIL_WIDTH};

use super::root::AppView;

/// The Lucide icon of a rail button.
fn section_icon(section: NavSection) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName;
    match section {
        NavSection::Collections => IconName::FolderTree,
        NavSection::SourceControl => IconName::GitBranch,
        NavSection::Environments => IconName::Variable,
        NavSection::LoadTests => IconName::Gauge,
    }
}

/// The action of a rail button's shortcut, whose keys its tooltip shows.
fn section_action(section: NavSection) -> Box<dyn Action> {
    match section {
        NavSection::Collections => Box::new(ToggleCollections),
        NavSection::SourceControl => Box::new(ToggleSourceControl),
        NavSection::Environments => Box::new(ToggleEnvironments),
        NavSection::LoadTests => Box::new(ToggleLoadTests),
    }
}

/// The tooltip (and accessible name) of a rail button.
fn section_label(section: NavSection) -> String {
    match section {
        NavSection::Collections => t!("navigation.rail.collections").into_owned(),
        NavSection::SourceControl => t!("navigation.rail.source_control").into_owned(),
        NavSection::Environments => t!("navigation.rail.environments").into_owned(),
        NavSection::LoadTests => t!("navigation.rail.load_tests").into_owned(),
    }
}

/// The pending changes counter on the Source Control button, capped at 99+.
fn change_badge(count: usize, palette: &crate::theme::Palette) -> impl IntoElement {
    let text = if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    };
    div()
        .absolute()
        .top(px(1.0))
        .right(px(0.0))
        .min_w(px(15.0))
        .h(px(15.0))
        .px(px(3.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(palette.accent)
        .text_color(palette.accent_fg)
        .text_size(px(9.5))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

impl AppView {
    /// Renders the activity rail: a button per [`NavSection`], the active one tinted. Clicking
    /// goes through [`AppView::click_rail`].
    pub(crate) fn render_rail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let palette = cx.palette();
        let weak = cx.weak_entity();
        let changes = self.git.change_count();

        v_flex()
            .flex_none()
            .w(px(RAIL_WIDTH))
            .h_full()
            .items_center()
            .gap(px(4.0))
            .pt(px(12.0))
            .bg(palette.bg)
            .border_r_1()
            .border_color(palette.border)
            .children(
                NavSection::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(index, section)| {
                        let active = self.nav.is_active(section);
                        let tooltip = SharedString::from(section_label(section));
                        let weak = weak.clone();
                        div()
                            .id(("rail-button", index))
                            .flex_none()
                            .size(px(RAIL_BUTTON_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            // A quiet tile, not a glowing filled circle: no shadow, a soft
                            // corner radius rather than one that reads as a near-full circle at
                            // this size, the active pick carried by a faint accent tint and the
                            // icon's own color instead of a loud solid fill.
                            .rounded(px(RAIL_BUTTON_RADIUS))
                            .cursor_pointer()
                            .when(active, |button| {
                                button
                                    .bg(palette.accent_subtle)
                                    .text_color(palette.accent_text)
                            })
                            .when(!active, |button| {
                                button
                                    .text_color(palette.fg_muted)
                                    .hover(|style| style.bg(palette.hover).text_color(palette.fg))
                                    .active(|style| style.bg(palette.pressed))
                            })
                            .relative()
                            .child(Icon::new(section_icon(section)).with_size(px(16.0)))
                            .when(
                                section == NavSection::SourceControl && changes > 0,
                                |button| button.child(change_badge(changes, &palette)),
                            )
                            .tooltip(move |window, cx| {
                                Tooltip::new(tooltip.clone())
                                    .action(section_action(section).as_ref(), None)
                                    .build(window, cx)
                            })
                            .on_click(move |_, _, cx| {
                                let _ = weak.update(cx, |view, cx| view.click_rail(section, cx));
                            })
                    }),
            )
            .into_any_element()
    }

    /// Handles a click on a rail button (`NavState::click`): switches section, collapses the
    /// sidebar on the active one, or expands it when collapsed. Opening the Environments panel
    /// reloads its rows, since the files may have changed since they were last read.
    pub(crate) fn click_rail(&mut self, section: NavSection, cx: &mut Context<Self>) {
        self.nav.click(section);
        log::debug!(
            "rail: {:?}, sidebar {}",
            self.nav.section(),
            if self.nav.sidebar_visible() {
                "visible"
            } else {
                "collapsed"
            }
        );
        if self.nav.sidebar_visible() && self.nav.section() == NavSection::Environments {
            self.refresh_environments();
        }
        if self.nav.section() != NavSection::Environments {
            self.new_env_input = None;
        }
        cx.notify();
    }

    /// Collapses the sidebar from its own "collapse" button, regardless of which section is
    /// active (`NavState::collapse`): a dedicated, more discoverable counterpart to re-clicking
    /// the active rail button.
    pub(crate) fn collapse_sidebar(&mut self, cx: &mut Context<Self>) {
        self.nav.collapse();
        cx.notify();
    }

    /// Renders the sidebar's content for the active section.
    pub(crate) fn render_side_panel(
        &mut self,
        weak: WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.nav.section() {
            NavSection::Collections => self.render_sidebar(weak, cx),
            NavSection::SourceControl => self.render_git_panel(cx),
            NavSection::Environments => self.render_env_panel(weak, cx),
            NavSection::LoadTests => self.render_load_panel(weak, cx),
        }
    }
}
