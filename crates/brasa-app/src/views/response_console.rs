//! The response's Console tab (#194, design 1a of "Console & Workspace History"): a filter bar
//! (All, Network, Logs, Errors, a text filter and Clear) over the timeline of the last send. The
//! network entry opens into a tree or the raw HTTP text.
//!
//! Everything shown comes from [`ConsoleTimeline`], built once per response on the background
//! thread, and from [`crate::state::console::ConsoleView`], which caches the visible tree rows.
//! Nothing here parses or formats a body.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::input::Copy;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_core::ConsoleLevel;

use crate::state::console::{
    ConsoleEntry, ConsoleFilter, ConsoleTimeline, CopyPart, EntrySource, NetworkDetail, TreeKey,
    TreeRow, ValueTone, copy_text, format_clock, tree_as_json,
};
use crate::state::response_render::{format_duration, format_size};
use crate::theme::metrics::{INPUT_PADDING_X, RADIUS_MD, RADIUS_SM};
use crate::theme::{Palette, PaletteExt, BrasaColors};
use crate::views::components::{GhostButton, SegmentedControl, SegmentedItem, edit_menu};

use super::root::AppView;

/// Height of the filter bar.
const FILTER_BAR_HEIGHT: f32 = 40.0;
/// Height of a chip, the filter field and the buttons of the bars.
const BAR_CONTROL_HEIGHT: f32 = 24.0;
/// Width of the text filter.
const FILTER_FIELD_WIDTH: f32 = 220.0;
/// Height of a timeline row.
const ENTRY_HEIGHT: f32 = 30.0;
/// Horizontal padding of the bars and the rows.
const PADDING_X: f32 = 14.0;
/// Gap between the columns of a row.
const COLUMN_GAP: f32 = 12.0;
/// Width of the chevron or icon column.
const ICON_COLUMN_WIDTH: f32 = 12.0;
/// Width of the time column (`10:42:03.118`).
const TIME_COLUMN_WIDTH: f32 = 88.0;
/// Width of the source column (`post-response`, a method).
const SOURCE_COLUMN_WIDTH: f32 = 96.0;
/// Left padding of the network detail: lines it up with the time column.
const DETAIL_PADDING_LEFT: f32 = PADDING_X + ICON_COLUMN_WIDTH + COLUMN_GAP;
/// Height of a tree row.
const TREE_ROW_HEIGHT: f32 = 22.0;
/// Indentation of one tree level.
const TREE_INDENT: f32 = 16.0;
/// Text size of the timeline and the tree.
const MONO_TEXT_SIZE: f32 = 12.0;

impl AppView {
    /// Renders the Console tab of the response of `tab_id`.
    pub(crate) fn render_console_tab(
        &mut self,
        tab_id: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let generation = self.response_generations.get(tab_id);
        self.console_view.sync(tab_id, generation);
        let Some(timeline) = self.response_consoles.get(tab_id).cloned() else {
            return div().into_any_element();
        };
        let mono_font = cx.theme().mono_font_family.clone();

        let visible = self.console_view.visible(&timeline);
        let list = if timeline.entries.is_empty() || self.console_view.cleared {
            empty_message(t!("response.console.empty"), &palette)
        } else if visible.is_empty() {
            empty_message(t!("response.console.no_match"), &palette)
        } else {
            let mut list = v_flex()
                .font_family(mono_font)
                .text_size(px(MONO_TEXT_SIZE));
            for (index, entry) in visible.into_iter().enumerate() {
                list = list.child(match entry {
                    ConsoleEntry::Network { .. } => self.render_network_entry(entry, &timeline, cx),
                    _ => render_plain_entry(index, entry, &palette),
                });
            }
            list.into_any_element()
        };

        v_flex()
            .size_full()
            .child(self.render_console_filter_bar(&timeline, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("console-list")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.response_scroll)
                            .child(list),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(Scrollbar::new(&self.response_scroll)),
                    ),
            )
            .into_any_element()
    }

    /// The chips, the text filter and Clear.
    fn render_console_filter_bar(
        &self,
        timeline: &ConsoleTimeline,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let palette = cx.palette();
        let weak = cx.weak_entity();
        let current = self.console_view.filter;
        let chips: Vec<_> = ConsoleFilter::ALL
            .into_iter()
            .map(|filter| {
                let weak = weak.clone();
                let selected = filter == current;
                let count = (filter == ConsoleFilter::Errors && timeline.error_count > 0)
                    .then(|| timeline.error_count.to_string());
                h_flex()
                    .id(match filter {
                        ConsoleFilter::All => "console-filter-all",
                        ConsoleFilter::Network => "console-filter-network",
                        ConsoleFilter::Logs => "console-filter-logs",
                        ConsoleFilter::Errors => "console-filter-errors",
                    })
                    .h(px(BAR_CONTROL_HEIGHT))
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .rounded(px(RADIUS_SM))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .map(|chip| {
                        if selected {
                            chip.bg(palette.border).text_color(palette.fg)
                        } else {
                            chip.text_color(palette.fg_muted)
                                .hover(|style| style.bg(palette.hover))
                                .active(|style| style.bg(palette.pressed))
                        }
                    })
                    .child(filter_label(filter))
                    .when_some(count, |chip, count| {
                        chip.child(div().text_color(palette.danger).child(count))
                    })
                    .on_click(move |_, _, cx| {
                        let _ = weak.update(cx, |view, cx| {
                            view.console_view.filter = filter;
                            cx.notify();
                        });
                    })
            })
            .collect();

        let clear_weak = weak.clone();
        h_flex()
            .flex_none()
            .h(px(FILTER_BAR_HEIGHT))
            .items_center()
            .gap(px(6.0))
            .px(px(PADDING_X))
            .border_b_1()
            .border_color(palette.border)
            .children(chips)
            .child(div().flex_1())
            .child(
                h_flex()
                    .id("console-filter-field")
                    .w(px(FILTER_FIELD_WIDTH))
                    .h(px(BAR_CONTROL_HEIGHT + 2.0))
                    .items_center()
                    .gap_2()
                    .px(px(9.0))
                    .rounded(px(RADIUS_MD - 1.0))
                    .bg(palette.raised)
                    .border_1()
                    .border_color(palette.border)
                    .hover({
                        let border_strong = palette.border_strong;
                        move |style| style.border_color(border_strong)
                    })
                    .text_color(palette.fg_subtle)
                    .child(Icon::new(Lucide::Search).small())
                    .child(
                        Input::new(&self.console_filter_input)
                            .context_menu(edit_menu(&self.console_filter_input, cx))
                            .pl_0()
                            .pr(px(INPUT_PADDING_X))
                            .h(px(BAR_CONTROL_HEIGHT))
                            .py_0()
                            .text_size(px(12.0))
                            .bordered(false)
                            .text_color(palette.fg)
                            .bg(palette.bg.opacity(0.0)),
                    ),
            )
            .child(
                GhostButton::new("console-clear", t!("response.console.clear"))
                    .icon(Icon::new(Lucide::Trash))
                    .height(BAR_CONTROL_HEIGHT + 2.0)
                    .on_click(move |_, _, cx| {
                        let _ = clear_weak.update(cx, |view, cx| {
                            view.console_view.cleared = true;
                            cx.notify();
                        });
                    }),
            )
    }

    /// The network entry: a clickable summary row and, when open, its detail.
    fn render_network_entry(
        &mut self,
        entry: &ConsoleEntry,
        timeline: &Arc<ConsoleTimeline>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ConsoleEntry::Network {
            time,
            method,
            url,
            status,
            duration,
            size,
        } = entry
        else {
            return div().into_any_element();
        };
        let palette = cx.palette();
        let weak = cx.weak_entity();
        let open = self.console_view.network_open;
        let status_color = palette.status_colors(*status).0;

        let summary = h_flex()
            .id("console-network")
            .h(px(ENTRY_HEIGHT))
            .items_center()
            .gap(px(COLUMN_GAP))
            .px(px(PADDING_X))
            .cursor_pointer()
            .hover(|style| style.bg(palette.hover))
            .active(|style| style.bg(palette.pressed))
            .child(
                div()
                    .w(px(ICON_COLUMN_WIDTH))
                    .text_color(palette.fg_muted)
                    .child(
                        Icon::new(if open {
                            Lucide::ChevronDown
                        } else {
                            Lucide::ChevronRight
                        })
                        .with_size(px(12.0)),
                    ),
            )
            .child(time_cell(*time, &palette))
            // The method right before the URL, read as one, not in the source column.
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(palette.method_color(method))
                            .child(method.to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(palette.fg)
                            .child(url.clone()),
                    ),
            )
            .when_some(*status, |row, status| {
                row.child(div().text_color(status_color).child(status.to_string()))
            })
            .when_some(*duration, |row, duration| {
                row.child(
                    div()
                        .text_color(palette.fg_muted)
                        .child(format_duration(duration)),
                )
            })
            .when_some(*size, |row, size| {
                row.child(div().text_color(palette.fg_muted).child(format_size(size)))
            })
            .on_click(move |_, _, cx| {
                let _ = weak.update(cx, |view, cx| {
                    view.console_view.network_open = !view.console_view.network_open;
                    cx.notify();
                });
            });

        let detail = match (&timeline.network, open) {
            (Some(detail), true) => Some(self.render_network_detail(detail, timeline, cx)),
            _ => None,
        };
        v_flex()
            .border_b_1()
            .border_color(palette.border.opacity(0.5))
            .when(open, |entry| entry.bg(palette.surface))
            .child(summary)
            .children(detail)
            .into_any_element()
    }

    /// The Tree/Raw switch, Expand all, Collapse and Copy, over the tree or the raw text.
    fn render_network_detail(
        &mut self,
        detail: &NetworkDetail,
        timeline: &Arc<ConsoleTimeline>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = cx.palette();
        let weak = cx.weak_entity();
        let raw = self.console_view.raw;

        let switch = SegmentedControl::new("console-view-switch")
            .item(
                SegmentedItem::new(t!("response.console.tree"))
                    .selected(!raw)
                    .on_click({
                        let weak = weak.clone();
                        move |_, cx| {
                            let _ = weak.update(cx, |view, cx| {
                                view.console_view.raw = false;
                                cx.notify();
                            });
                        }
                    }),
            )
            .item(
                SegmentedItem::new(t!("response.body.raw"))
                    .selected(raw)
                    .on_click({
                        let weak = weak.clone();
                        move |_, cx| {
                            let _ = weak.update(cx, |view, cx| {
                                view.console_view.raw = true;
                                cx.notify();
                            });
                        }
                    }),
            );

        // The buttons keep the shared timeline, not a copy of the detail: cloning the tree on
        // every render would cost in proportion to the response.
        let expand_timeline = timeline.clone();
        let expand_weak = weak.clone();
        let collapse_weak = weak.clone();
        let copy_timeline = timeline.clone();
        let toolbar = h_flex()
            .items_center()
            .gap(px(6.0))
            .font_family(cx.theme().font_family.clone())
            .child(switch)
            .child(div().flex_1())
            .child(
                GhostButton::new("console-expand-all", t!("response.console.expand_all"))
                    .icon(Icon::new(Lucide::ChevronsUpDown))
                    .height(BAR_CONTROL_HEIGHT)
                    .on_click(move |_, _, cx| {
                        let Some(detail) = expand_timeline.network.as_ref() else {
                            return;
                        };
                        let _ = expand_weak.update(cx, |view, cx| {
                            view.console_view.expand_all(detail);
                            cx.notify();
                        });
                    }),
            )
            .child(
                GhostButton::new("console-collapse", t!("response.console.collapse"))
                    .icon(Icon::new(Lucide::ChevronsDownUp))
                    .height(BAR_CONTROL_HEIGHT)
                    .on_click(move |_, _, cx| {
                        let _ = collapse_weak.update(cx, |view, cx| {
                            view.console_view.collapse_all();
                            cx.notify();
                        });
                    }),
            )
            .child(
                GhostButton::new("console-copy", t!("common.copy"))
                    .icon(Icon::new(Lucide::Copy))
                    .height(BAR_CONTROL_HEIGHT)
                    .on_click(move |_, _, cx| {
                        let Some(detail) = copy_timeline.network.as_ref() else {
                            return;
                        };
                        let text = if raw {
                            raw_text(detail)
                        } else {
                            tree_as_json(&detail.tree)
                        };
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }),
            );

        let body = if raw {
            render_raw(detail, &palette, cx)
        } else {
            let rows = self.console_view.rows(detail);
            let selected = self.console_view.selected.clone();
            let mut tree = v_flex();
            for (index, row) in rows.iter().enumerate() {
                let reveal = (selected.as_deref() == Some(row.path.as_str())).then(|| Reveal {
                    scroll: self.response_scroll.clone(),
                    pending: self.console_reveal.clone(),
                });
                tree = tree.child(render_tree_row(
                    index,
                    row,
                    reveal,
                    timeline.clone(),
                    weak.clone(),
                    &palette,
                    cx,
                ));
            }
            // A click on a row focuses the tree (gpui focuses a tracked handle on mouse down),
            // so the arrow keys move the selection and Ctrl+C copies it (#197).
            let key_timeline = timeline.clone();
            let copy_timeline = timeline.clone();
            div()
                .track_focus(&self.console_tree_focus)
                .on_key_down(cx.listener(move |view, event: &KeyDownEvent, _, cx| {
                    view.on_console_tree_key(event, &key_timeline, cx);
                }))
                .on_action(cx.listener(move |view, _: &Copy, _, cx| {
                    match view.console_view.selected.as_deref() {
                        Some(path) => copy_row(&copy_timeline, path, CopyPart::Value, cx),
                        // Nothing selected: the window's text selection, as everywhere else.
                        None => cx.propagate(),
                    }
                }))
                .child(tree)
                .into_any_element()
        };

        v_flex()
            .gap_2()
            .pt(px(4.0))
            .pb(px(PADDING_X))
            .pl(px(DETAIL_PADDING_LEFT))
            .pr(px(PADDING_X))
            .child(toolbar)
            .child(
                div()
                    .py_2()
                    .border_1()
                    .border_color(palette.border)
                    .rounded(px(RADIUS_MD))
                    .bg(palette.bg)
                    .child(body),
            )
            .into_any_element()
    }
}

impl AppView {
    /// The keys of the focused network tree: Up and Down move one row, Home and End to the
    /// first and last, Right opens a branch or goes to its first child, Left closes it or goes
    /// to the parent.
    fn on_console_tree_key(
        &mut self,
        event: &KeyDownEvent,
        timeline: &ConsoleTimeline,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift {
            return;
        }
        let key = match keystroke.key.as_str() {
            "up" => TreeKey::Up,
            "down" => TreeKey::Down,
            "left" => TreeKey::Left,
            "right" => TreeKey::Right,
            "home" => TreeKey::Home,
            "end" => TreeKey::End,
            _ => return,
        };
        let Some(detail) = timeline.network.as_ref() else {
            return;
        };
        cx.stop_propagation();
        if self.console_view.navigate(key, detail) {
            self.console_reveal.set(true);
            cx.notify();
        }
    }
}

/// Writes `part` of the tree row at `path` to the clipboard.
fn copy_row(timeline: &ConsoleTimeline, path: &str, part: CopyPart, cx: &mut App) {
    let text = timeline
        .network
        .as_ref()
        .and_then(|detail| copy_text(&detail.tree, path, part));
    if let Some(text) = text {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

/// A pending request to scroll the console so the selected tree row is fully visible, after an
/// arrow key moved the selection.
struct Reveal {
    scroll: ScrollHandle,
    pending: Rc<Cell<bool>>,
}

/// An empty canvas over the selected row that reads its bounds at prepaint and, when a reveal
/// is pending, adjusts the console's scroll offset once and draws one more frame. The rows are
/// not children of the scrolled element, so `ScrollHandle::scroll_to_item` cannot reach them.
fn reveal_canvas(reveal: Reveal) -> impl IntoElement {
    canvas(
        move |row, window, _cx| {
            if !reveal.pending.replace(false) {
                return;
            }
            let view = reveal.scroll.bounds();
            let mut offset = reveal.scroll.offset();
            if row.top() < view.top() {
                offset.y += view.top() - row.top();
            } else if row.bottom() > view.bottom() {
                offset.y -= row.bottom() - view.bottom();
            } else {
                return;
            }
            reveal.scroll.set_offset(offset);
            window.refresh();
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// The label of a filter chip.
fn filter_label(filter: ConsoleFilter) -> SharedString {
    match filter {
        ConsoleFilter::All => t!("response.console.filter_all"),
        ConsoleFilter::Network => t!("response.console.filter_network"),
        ConsoleFilter::Logs => t!("response.console.filter_logs"),
        ConsoleFilter::Errors => t!("response.console.filter_errors"),
    }
    .into_owned()
    .into()
}

/// The muted message shown instead of the list.
fn empty_message(text: impl Into<SharedString>, palette: &Palette) -> AnyElement {
    div()
        .px(px(PADDING_X))
        .py_2()
        .text_color(palette.fg_muted)
        .child(text.into())
        .into_any_element()
}

/// The time column.
fn time_cell(time: Option<std::time::SystemTime>, palette: &Palette) -> impl IntoElement {
    div()
        .w(px(TIME_COLUMN_WIDTH))
        .flex_none()
        .text_color(palette.fg_subtle)
        .children(time.map(format_clock))
}

/// The name a script goes by in the source and location columns. Kept in English, like the
/// `::: pre` and `::: post` markers they stand for.
fn source_name(source: EntrySource) -> &'static str {
    match source {
        EntrySource::Pre => "pre-request",
        EntrySource::Post => "post-response",
        EntrySource::Network => "network",
    }
}

/// A log line or an error row.
fn render_plain_entry(index: usize, entry: &ConsoleEntry, palette: &Palette) -> AnyElement {
    let (time, source, text, line, color, error) = match entry {
        ConsoleEntry::Log {
            time,
            source,
            level,
            text,
            line,
        } => {
            let color = match level {
                ConsoleLevel::Log | ConsoleLevel::Info => palette.fg,
                ConsoleLevel::Warn => palette.warning,
                ConsoleLevel::Error => palette.danger,
            };
            (
                *time,
                *source,
                text,
                *line,
                color,
                *level == ConsoleLevel::Error,
            )
        }
        ConsoleEntry::Error {
            time,
            source,
            message,
            line,
        } => (*time, *source, message, *line, palette.danger, true),
        ConsoleEntry::Network { .. } => return div().into_any_element(),
    };
    let is_failure = matches!(entry, ConsoleEntry::Error { .. });
    let location = line
        .filter(|_| source != EntrySource::Network)
        .map(|line| format!("{}.js:{line}", source_name(source)));

    h_flex()
        .id(("console-entry", index))
        .min_h(px(ENTRY_HEIGHT))
        .items_center()
        .gap(px(COLUMN_GAP))
        .px(px(PADDING_X))
        .py(px(4.0))
        .border_b_1()
        .border_color(palette.border.opacity(0.5))
        .when(is_failure, |row| row.bg(palette.danger_subtle))
        .child(
            div()
                .w(px(ICON_COLUMN_WIDTH))
                .flex_none()
                .text_color(palette.danger)
                .when(error, |cell| {
                    cell.child(Icon::new(Lucide::CircleX).with_size(px(12.0)))
                }),
        )
        .child(time_cell(time, palette))
        .child(
            div()
                .w(px(SOURCE_COLUMN_WIDTH))
                .flex_none()
                .text_color(palette.fg_muted)
                .child(source_name(source)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(color)
                .child(text.clone()),
        )
        .children(location.map(|location| {
            div()
                .flex_none()
                .text_color(palette.fg_subtle)
                .child(location)
        }))
        .into_any_element()
}

/// One row of the network tree. A click selects it, and opens or closes a branch; a right click
/// selects it and opens the Copy menu. `reveal` is set on the selected row only.
fn render_tree_row(
    index: usize,
    row: &TreeRow,
    reveal: Option<Reveal>,
    timeline: Arc<ConsoleTimeline>,
    weak: WeakEntity<AppView>,
    palette: &Palette,
    cx: &App,
) -> AnyElement {
    let selected = reveal.is_some();
    let mono = cx.theme().mono_font_family.clone();
    let value_color = match row.tone {
        ValueTone::String => palette.syntax_color("string"),
        ValueTone::Number => palette.syntax_color("number"),
        ValueTone::Constant => palette.syntax_color("boolean"),
        ValueTone::Summary => palette.fg_muted,
    };
    let chevron = row.open.map(|open| {
        Icon::new(if open {
            Lucide::ChevronDown
        } else {
            Lucide::ChevronRight
        })
        .with_size(px(12.0))
    });
    let path = row.path.clone();
    let branch = row.open.is_some();
    let right_weak = weak.clone();
    let right_path = path.clone();
    let menu_path = path.clone();
    h_flex()
        .id(("console-tree-row", index))
        .relative()
        .h(px(TREE_ROW_HEIGHT))
        .items_center()
        .gap(px(6.0))
        .pr(px(12.0))
        .pl(px(12.0 + row.depth as f32 * TREE_INDENT))
        .when(selected, |line| line.bg(palette.accent_subtle))
        .when(!selected, |line| {
            line.hover(|style| style.bg(palette.hover))
        })
        // Every row is clickable since #197 (it selects), not only branches.
        .active(|style| style.bg(palette.pressed))
        .cursor_pointer()
        // The UI font for the Copy menu, which inherits it; the key and value are mono.
        .font_family(cx.theme().font_family.clone())
        .child(
            div()
                .w(px(12.0))
                .flex_none()
                .text_color(palette.fg_subtle)
                .children(chevron),
        )
        .child(
            h_flex()
                .flex_none()
                .font_family(mono.clone())
                .child(
                    div()
                        .text_color(palette.syntax_color("property"))
                        .child(row.key.clone()),
                )
                .child(div().text_color(palette.fg_subtle).child(":")),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(mono)
                .text_color(value_color)
                .child(row.text.clone()),
        )
        .children(reveal.map(reveal_canvas))
        .on_click(move |_, _, cx| {
            let _ = weak.update(cx, |view, cx| {
                view.console_view.select(&path);
                if branch {
                    view.console_view.toggle(&path);
                }
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Right, move |_, _, cx| {
            let _ = right_weak.update(cx, |view, cx| {
                view.console_view.select(&right_path);
                cx.notify();
            });
        })
        .context_menu(move |menu, _, _| {
            let item = |label: SharedString, part: CopyPart| {
                let (timeline, path) = (timeline.clone(), menu_path.clone());
                PopupMenuItem::new(label)
                    .on_click(move |_, _, cx| copy_row(&timeline, &path, part, cx))
            };
            menu.item(item(
                t!("response.console.copy_value").into_owned().into(),
                CopyPart::Value,
            ))
            .item(item(
                t!("response.console.copy_key").into_owned().into(),
                CopyPart::Key,
            ))
            .item(item(
                t!("response.console.copy_path").into_owned().into(),
                CopyPart::Path,
            ))
        })
        .into_any_element()
}

/// The raw request and response, each under its label.
fn render_raw(detail: &NetworkDetail, palette: &Palette, cx: &App) -> AnyElement {
    let label = |text: SharedString| {
        div()
            .font_family(cx.theme().font_family.clone())
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(palette.fg_subtle)
            .child(text)
    };
    let block = |text: &Arc<str>| {
        div()
            .text_color(palette.fg)
            .line_height(relative(1.6))
            .child(SharedString::from(text.clone()))
    };
    v_flex()
        .gap_3()
        .px(px(PADDING_X))
        .py_1()
        .child(label(
            t!("response.console.raw_request").into_owned().into(),
        ))
        .child(block(&detail.raw_request))
        .when_some(detail.raw_response.as_ref(), |raw, response| {
            raw.child(label(
                t!("response.console.raw_response").into_owned().into(),
            ))
            .child(block(response))
        })
        .into_any_element()
}

/// The raw text of both sides, for Copy.
fn raw_text(detail: &NetworkDetail) -> String {
    match &detail.raw_response {
        Some(response) => format!("{}\n\n{response}", detail.raw_request),
        None => detail.raw_request.to_string(),
    }
}
