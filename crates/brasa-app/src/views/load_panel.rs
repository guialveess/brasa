//! The Load tests panel of the sidebar (GitHub #64): a "New load test" button, the target tree
//! (folders collapse, a play icon shows on hover, clicking a request or a folder's play opens a
//! load test tab for it) and "Runs this session", one entry per open load test tab. The data
//! comes from `state::load_panel`: the tree is rebuilt in [`AppView::refresh_tree`], the run
//! entries are derived from the open tabs, which are few.

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use crate::state::load_panel::{
    RunDot, RunEntry, TargetRow, TargetRowKind, meta_text, run_entries,
};
use crate::theme::metrics::{ICON_BUTTON_HEADER, RADIUS_SM};
use crate::theme::{Palette, PaletteExt};
use crate::views::components::{IconButton, MethodBadge, PrimaryButton, SectionLabel, TreeRow};

use super::root::AppView;

impl AppView {
    /// Renders the Load tests panel.
    pub(crate) fn render_load_panel(
        &mut self,
        weak: WeakEntity<Self>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let has_workspace = self.state.workspace.is_some();
        let runs = run_entries(
            self.state.tabs.open_tabs(),
            self.state.tabs.active_index(),
            crate::views::load_test::tab_label,
        );

        let header = h_flex()
            .justify_between()
            .items_center()
            .pt(px(10.0))
            .pr(px(10.0))
            .pb(px(6.0))
            .pl(px(14.0))
            .child(SectionLabel::new(t!("navigation.load.title")))
            .child(
                IconButton::new("new-load-test-header", IconName::Plus)
                    .box_size(ICON_BUTTON_HEADER)
                    .tooltip(t!("navigation.load.new"))
                    .on_click({
                        let weak = weak.clone();
                        move |_, _, cx| {
                            let _ = weak.update(cx, |view, cx| view.open_new_load_test_tab(cx));
                        }
                    }),
            );

        let new_button = div().px(px(10.0)).pt(px(2.0)).pb(px(10.0)).child(
            PrimaryButton::new("new-load-test", t!("navigation.load.new"))
                .icon(gpui_kit::assets::IconName::Gauge)
                .full_width()
                .on_click({
                    let weak = weak.clone();
                    move |_, _, cx| {
                        let _ = weak.update(cx, |view, cx| view.open_new_load_test_tab(cx));
                    }
                }),
        );

        let mut targets = v_flex().px(px(6.0));
        if has_workspace {
            for (index, row) in self.load_targets.visible().iter().enumerate() {
                targets = targets.child(target_row(index, row, weak.clone(), &palette));
            }
        }

        let mut run_list = v_flex().px(px(6.0)).gap(px(2.0));
        if runs.is_empty() {
            run_list = run_list.child(
                div()
                    .px(px(8.0))
                    .py(px(4.0))
                    .text_size(px(12.0))
                    .text_color(palette.fg_subtle)
                    .child(t!("navigation.load.no_runs")),
            );
        }
        for (index, run) in runs.iter().enumerate() {
            run_list = run_list.child(run_entry(index, run, weak.clone(), &palette, &mono_font));
        }

        v_flex()
            .size_full()
            .bg(palette.surface)
            .border_r_1()
            .border_color(palette.border)
            .child(header)
            .child(new_button)
            .child(
                v_flex()
                    .id("load-panel-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(muted_label(t!("navigation.load.targets"), 6.0, &palette))
                    .child(targets)
                    .child(muted_label(t!("navigation.load.runs"), 14.0, &palette))
                    .child(run_list)
                    .pb(px(8.0)),
            )
            .into_any_element()
    }
}

/// A small muted heading ("Targets", "Runs this session").
fn muted_label(text: impl Into<SharedString>, top: f32, palette: &Palette) -> AnyElement {
    div()
        .px(px(14.0))
        .pt(px(top))
        .pb(px(6.0))
        .text_size(px(11.0))
        .text_color(palette.fg_subtle)
        .child(text.into())
        .into_any_element()
}

/// One row of the target tree. A folder row toggles its children, a request row opens a load
/// test for it; the play icon (visible on hover) opens one for the request or the whole folder.
fn target_row(
    index: usize,
    row: &TargetRow,
    weak: WeakEntity<AppView>,
    palette: &Palette,
) -> AnyElement {
    let id = row.id.clone();
    let is_folder = matches!(row.kind, TargetRowKind::Folder { .. });

    let open = {
        let weak = weak.clone();
        let id = id.clone();
        move |cx: &mut App| {
            let id = id.clone();
            let _ = weak.update(cx, |view, cx| {
                if is_folder {
                    view.open_load_test_for_collection(id, cx);
                } else {
                    view.open_load_test_for_request(id, cx);
                }
            });
        }
    };
    let play = div()
        .id(("load-target-play", index))
        .size(px(20.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(RADIUS_SM))
        .cursor_pointer()
        .text_color(palette.fg_muted)
        .hover(|style| style.bg(palette.hover))
        .active(|style| style.bg(palette.pressed))
        .child(Icon::new(IconName::Play).small())
        .on_click({
            let open = open.clone();
            move |_, _, cx| {
                cx.stop_propagation();
                open(cx);
            }
        });

    let mut tree_row = TreeRow::new(("load-target", index), row.label.clone())
        .depth(row.depth)
        .trailing(play);
    match &row.kind {
        TargetRowKind::Folder { expanded } => tree_row = tree_row.folder(*expanded),
        TargetRowKind::Request {
            method: Some(method),
        } => tree_row = tree_row.leading(MethodBadge::inline(method.clone())),
        TargetRowKind::Request { method: None } => {}
    }
    let toggle_id = id.clone();
    tree_row
        .on_click(move |_, _, cx| {
            if is_folder {
                let id = toggle_id.clone();
                let _ = weak.update(cx, |view, cx| {
                    view.load_targets.toggle(&id);
                    cx.notify();
                });
            } else {
                open(cx);
            }
        })
        .into_any_element()
}

/// One "Runs this session" entry: a status dot, the tab label and a mono meta line. Clicking
/// activates the tab.
fn run_entry(
    index: usize,
    run: &RunEntry,
    weak: WeakEntity<AppView>,
    palette: &Palette,
    mono_font: &SharedString,
) -> AnyElement {
    let dot_color = match run.dot {
        RunDot::Running => palette.accent_text,
        RunDot::Done => palette.success,
        RunDot::Failed => palette.danger,
        RunDot::Idle => palette.fg_subtle,
    };
    let tab_id = run.tab_id.clone();

    v_flex()
        .id(("load-run", index))
        .gap(px(2.0))
        .px(px(8.0))
        .py(px(6.0))
        .rounded(px(RADIUS_SM))
        .cursor_pointer()
        .text_size(px(13.0))
        .when(run.active, |this| {
            this.bg(palette.accent_subtle)
                .text_color(palette.accent_text)
        })
        .when(!run.active, |this| {
            this.text_color(palette.fg)
                .hover(|style| style.bg(palette.hover))
                .active(|style| style.bg(palette.pressed))
        })
        .child(
            h_flex()
                .items_center()
                .gap(px(6.0))
                .child(div().flex_none().size(px(6.0)).rounded_full().bg(dot_color))
                .child(div().flex_1().min_w_0().truncate().child(run.label.clone())),
        )
        .child(
            div()
                .pl(px(12.0))
                .font_family(mono_font.clone())
                .text_size(px(11.5))
                .text_color(palette.fg_subtle)
                .child(meta_text(&run.meta)),
        )
        .on_click(move |_, _, cx| {
            let tab_id = tab_id.clone();
            let _ = weak.update(cx, |view, cx| {
                if let Some(index) = view.state.tabs.index_of(&tab_id) {
                    view.state.tabs.set_active(index);
                    cx.notify();
                }
            });
        })
        .into_any_element()
}
