//! The load test tab's left config panel: the target segmented control and picker, the numeric
//! fields, the thresholds, the "Stop on errors" and "Reuse connections" switches, and the
//! Start/Stop run button.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;
use rust_i18n::t;

use brasa_load::ThresholdMetric;

use crate::state::load_test::{
    LoadTestConfigInputs, LoadTestTab, LoadTestTarget, TargetKind, ThresholdInput,
    threshold_metric_label, threshold_unit,
};
use crate::state::locale;
use crate::theme::metrics::{CONTROL_HEIGHT, INPUT_PADDING_X, RADIUS_MD};
use crate::theme::{Palette, PaletteExt};
use crate::views::components::{
    DangerButton, IconButton, NeutralButtonExt, PrimaryButton, SegmentedControl, SegmentedItem,
    edit_menu, select_trigger, switch_halo,
};

use crate::views::root::AppView;

/// Width of the config panel (280 px).
const PANEL_WIDTH: f32 = 280.0;

/// The `gpui` entities behind a load test tab's editable numeric fields, rebuilt only when the
/// active tab id changes, the same reasoning as `views/request_editor.rs`'s
/// `RequestEditorEntities`: typing into one of these must never lose focus on an unrelated
/// re-render. The target picker needs no entity of its own (a plain dropdown menu on
/// `components::select_trigger`).
#[derive(Default)]
pub(crate) struct LoadTestEntities {
    built_for: Option<String>,
    vus: Option<Entity<InputState>>,
    duration_secs: Option<Entity<InputState>>,
    ramp_up_secs: Option<Entity<InputState>>,
    think_time_ms: Option<Entity<InputState>>,
    /// One input per "Thresholds" row, kept in step with the rows by [`Self::sync`].
    thresholds: Vec<(ThresholdMetric, Entity<InputState>)>,
    /// The results' Requests tab search box.
    pub(crate) request_search: Option<Entity<InputState>>,
    /// The scroll position of the results' Requests table, reset to the top when its filters or
    /// sort change.
    pub(crate) requests_scroll: UniformListScrollHandle,
    /// The height the Summary's chart had room for in the last frame, measured by the chart
    /// itself (see `summary.rs`'s `render_chart`). Zero until first measured.
    pub(crate) chart_room: Rc<Cell<f32>>,
}

impl LoadTestEntities {
    /// Rebuilds every field entity when `tab_id` differs from what they were last built for, and
    /// adds or drops threshold inputs as rows are added or removed.
    fn sync(
        &mut self,
        tab_id: &str,
        load_test: &LoadTestTab,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        let config = &load_test.config;
        if self.built_for.as_deref() != Some(tab_id) {
            self.build(tab_id, config, &load_test.request_filter.query, window, cx);
        }
        self.sync_thresholds(tab_id, &config.thresholds, window, cx);
    }

    /// Keeps one input per threshold row: drops the inputs of removed rows and builds one for
    /// each new row, leaving the others (and their focus) alone.
    fn sync_thresholds(
        &mut self,
        tab_id: &str,
        rows: &[ThresholdInput],
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        self.thresholds
            .retain(|(metric, _)| rows.iter().any(|row| row.metric == *metric));
        for row in rows {
            if self
                .thresholds
                .iter()
                .any(|(metric, _)| *metric == row.metric)
            {
                continue;
            }
            let metric = row.metric;
            let entity = build_field(
                tab_id,
                &row.limit,
                window,
                cx,
                move |view, tab_id, value, cx| {
                    view.set_load_test_threshold_limit(tab_id, metric, value, cx);
                },
            );
            self.thresholds.push((metric, entity));
        }
    }

    /// Builds every numeric field entity for `tab_id`.
    fn build(
        &mut self,
        tab_id: &str,
        config: &LoadTestConfigInputs,
        query: &str,
        window: &mut Window,
        cx: &mut Context<AppView>,
    ) {
        self.thresholds.clear();
        self.requests_scroll = UniformListScrollHandle::new();
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("load_test.requests.search").into_owned())
        });
        search.update(cx, |state, cx| {
            state.set_value(query.to_string(), window, cx);
        });
        let search_tab_id = tab_id.to_string();
        cx.subscribe(&search, move |view, entity, event: &InputEvent, cx| {
            if !matches!(event, InputEvent::Change) {
                return;
            }
            let query = entity.read(cx).value().to_string();
            view.edit_load_test_request_filter(&search_tab_id, cx, |filter| filter.query = query);
        })
        .detach();
        self.request_search = Some(search);
        self.vus = Some(build_field(
            tab_id,
            &config.vus,
            window,
            cx,
            |view, tab_id, value, cx| view.set_load_test_vus(tab_id, value, cx),
        ));
        self.duration_secs = Some(build_field(
            tab_id,
            &config.duration_secs,
            window,
            cx,
            |view, tab_id, value, cx| view.set_load_test_duration_secs(tab_id, value, cx),
        ));
        self.ramp_up_secs = Some(build_field(
            tab_id,
            &config.ramp_up_secs,
            window,
            cx,
            |view, tab_id, value, cx| view.set_load_test_ramp_up_secs(tab_id, value, cx),
        ));
        self.think_time_ms = Some(build_field(
            tab_id,
            &config.think_time_ms,
            window,
            cx,
            |view, tab_id, value, cx| view.set_load_test_think_time_ms(tab_id, value, cx),
        ));
        self.built_for = Some(tab_id.to_string());
    }
}

/// Builds one numeric field's `InputState`, seeded with `initial`, calling `on_change` with its
/// new text on every edit.
fn build_field(
    tab_id: &str,
    initial: &str,
    window: &mut Window,
    cx: &mut Context<AppView>,
    on_change: impl Fn(&mut AppView, &str, String, &mut Context<AppView>) + 'static,
) -> Entity<InputState> {
    let input = cx.new(|cx| InputState::new(window, cx));
    input.update(cx, |state, cx| {
        state.set_value(initial.to_string(), window, cx);
    });
    let tab_id = tab_id.to_string();
    cx.subscribe(&input, move |view, entity, event: &InputEvent, cx| {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let value = entity.read(cx).value().to_string();
        on_change(view, &tab_id, value, cx);
    })
    .detach();
    input
}

impl AppView {
    /// Renders the config panel for the load test tab `tab_id`.
    pub(crate) fn render_load_test_config_panel(
        &mut self,
        tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(load_test) = self
            .state
            .tabs
            .active()
            .and_then(|tab| tab.load_test())
            .cloned()
        else {
            return div().into_any_element();
        };
        self.load_test_entities.sync(tab_id, &load_test, window, cx);

        let palette = cx.palette();
        let mono_font = cx.theme().mono_font_family.clone();
        let ui_font = cx.theme().font_family.clone();
        let is_running = load_test.is_running();
        let weak = cx.weak_entity();
        let tree = self
            .state
            .workspace
            .as_ref()
            .map(|workspace| workspace.tree().to_vec());

        // Split in two: a scrollable fields area (`flex_1().min_h_0().overflow_y_scroll()`) and a
        // `flex_none` footer holding the Start/Stop button and the note below it. Previously
        // everything (including the button) was one `v_flex` with a `div().flex_1()` spacer pushing
        // the button down; with no scroll anywhere, a short window simply clipped the button and
        // note below the visible area instead of keeping them reachable.
        v_flex()
            .flex_none()
            .w(px(PANEL_WIDTH))
            .h_full()
            .min_h_0()
            .border_r_1()
            .border_color(palette.border)
            .child(
                v_flex()
                    .id("load-test-config-fields")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p(px(16.0))
                    .gap(px(16.0))
                    .child(render_target_section(
                        weak.clone(),
                        tab_id,
                        &palette,
                        &load_test,
                        tree.as_deref(),
                        is_running,
                        cx,
                    ))
                    .child(
                        v_flex()
                            .gap(px(10.0))
                            .child(
                                h_flex()
                                    .gap(px(10.0))
                                    .child(render_field(
                                        &palette,
                                        &mono_font,
                                        &ui_font,
                                        t!("load_test.config.vus").into_owned(),
                                        self.load_test_entities.vus.as_ref(),
                                        "VUs",
                                        is_running,
                                        cx,
                                    ))
                                    .child(render_field(
                                        &palette,
                                        &mono_font,
                                        &ui_font,
                                        t!("load_test.config.duration").into_owned(),
                                        self.load_test_entities.duration_secs.as_ref(),
                                        "s",
                                        is_running,
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap(px(10.0))
                                    .child(render_field(
                                        &palette,
                                        &mono_font,
                                        &ui_font,
                                        t!("load_test.config.ramp_up").into_owned(),
                                        self.load_test_entities.ramp_up_secs.as_ref(),
                                        "s",
                                        is_running,
                                        cx,
                                    ))
                                    .child(render_field(
                                        &palette,
                                        &mono_font,
                                        &ui_font,
                                        t!("load_test.config.think_time").into_owned(),
                                        self.load_test_entities.think_time_ms.as_ref(),
                                        "ms",
                                        is_running,
                                        cx,
                                    )),
                            ),
                    )
                    .child(render_thresholds(
                        weak.clone(),
                        tab_id,
                        &palette,
                        &mono_font,
                        &ui_font,
                        &load_test,
                        &self.load_test_entities.thresholds,
                        is_running,
                        cx,
                    ))
                    .child({
                        let weak = weak.clone();
                        let tab_id = tab_id.to_string();
                        render_switch_row(
                            "load-test-stop-on-error",
                            t!("load_test.config.stop_on_errors").into_owned(),
                            t!("load_test.config.stop_on_errors_hint").into_owned(),
                            load_test.config.stop_on_error,
                            &palette,
                            is_running,
                            move |checked, cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    view.set_load_test_stop_on_error(&tab_id, checked, cx);
                                });
                            },
                        )
                    })
                    .child({
                        let weak = weak.clone();
                        let tab_id = tab_id.to_string();
                        render_switch_row(
                            "load-test-reuse-connections",
                            t!("load_test.config.reuse_connections").into_owned(),
                            t!("load_test.config.reuse_connections_hint").into_owned(),
                            load_test.config.reuse_connections,
                            &palette,
                            is_running,
                            move |checked, cx| {
                                let _ = weak.update(cx, |view, cx| {
                                    view.set_load_test_reuse_connections(&tab_id, checked, cx);
                                });
                            },
                        )
                    }),
            )
            .child(
                v_flex()
                    .flex_none()
                    .px(px(16.0))
                    .pb(px(16.0))
                    .gap(px(16.0))
                    .child(render_start_stop_button(weak.clone(), tab_id, is_running))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(palette.fg_subtle)
                            .text_center()
                            .w_full()
                            .child(div().w_full().child(t!("load_test.config.footer")))
                            .child(div().font_family(mono_font).child(".brasa/runs/")),
                    ),
            )
            .into_any_element()
    }
}

/// The "Target" label, segmented control (Request/Collection) and the target picker dropdown.
fn render_target_section(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    tree: Option<&[brasa_workspace::Node]>,
    is_running: bool,
    cx: &App,
) -> AnyElement {
    let request_weak = weak.clone();
    let request_tab_id = tab_id.to_string();
    let collection_weak = weak.clone();
    let collection_tab_id = tab_id.to_string();

    v_flex()
        .gap(px(6.0))
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(palette.fg_muted)
                .child(t!("load_test.config.target")),
        )
        .child(
            SegmentedControl::new("load-test-target-kind")
                .item(
                    SegmentedItem::new(t!("common.request"))
                        .selected(load_test.target_kind == TargetKind::Request)
                        .on_click(move |_, cx| {
                            let _ = request_weak.update(cx, |view, cx| {
                                view.set_load_test_target_kind(
                                    &request_tab_id,
                                    TargetKind::Request,
                                    cx,
                                );
                            });
                        }),
                )
                .item(
                    SegmentedItem::new(t!("load_test.config.kind_collection"))
                        .selected(load_test.target_kind == TargetKind::Collection)
                        .on_click(move |_, cx| {
                            let _ = collection_weak.update(cx, |view, cx| {
                                view.set_load_test_target_kind(
                                    &collection_tab_id,
                                    TargetKind::Collection,
                                    cx,
                                );
                            });
                        }),
                ),
        )
        .child(render_target_picker(
            weak, tab_id, palette, load_test, tree, is_running, cx,
        ))
        .into_any_element()
}

/// The bordered row showing the currently picked target (or a placeholder), opening a dropdown of
/// every request or folder (depending on [`LoadTestTab::target_kind`]) on click.
fn render_target_picker(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    load_test: &LoadTestTab,
    tree: Option<&[brasa_workspace::Node]>,
    is_running: bool,
    cx: &App,
) -> AnyElement {
    use crate::state::load_test::{folders_flat, requests_flat};

    let is_collection = load_test.target_kind == TargetKind::Collection;
    let icon = if is_collection {
        Some(IconName::Folder)
    } else {
        None
    };
    let label = if load_test.target_label.is_empty() {
        if is_collection {
            t!("load_test.config.choose_folder").into_owned()
        } else {
            t!("load_test.config.choose_request").into_owned()
        }
    } else {
        load_test.target_label.clone()
    };
    let hint = if load_test.target.is_some() && is_collection {
        Some(request_count_label(load_test.target_request_count))
    } else {
        None
    };

    let mut content = h_flex()
        .w_full()
        .items_center()
        .gap(px(8.0))
        .when_some(icon, |row, icon| {
            row.child(Icon::new(icon).small().text_color(palette.fg_subtle))
        })
        .child(div().flex_1().min_w_0().truncate().child(label));
    if let Some(hint) = hint {
        content = content.child(
            div()
                .text_size(px(12.0))
                .text_color(palette.fg_subtle)
                .child(hint),
        );
    }

    let trigger = select_trigger("load-test-target-picker", palette, cx)
        .w_full()
        .disabled(is_running)
        .when(is_running, |button| button.opacity(0.5))
        .child(content);
    if is_running {
        return trigger.into_any_element();
    }

    let Some(tree) = tree else {
        return trigger.into_any_element();
    };

    if is_collection {
        let folders = folders_flat(tree);
        let tab_id = tab_id.to_string();
        trigger
            .dropdown_menu(move |mut menu, _, _| {
                for (id, name, count) in &folders {
                    let target_weak = weak.clone();
                    let tab_id = tab_id.clone();
                    let id = id.clone();
                    let name = name.clone();
                    let count = *count;
                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(move |_, _, cx| {
                        let id = id.clone();
                        let name = name.clone();
                        let _ = target_weak.update(cx, |view, cx| {
                            view.pick_load_test_target(
                                &tab_id,
                                LoadTestTarget::Collection(id),
                                name,
                                count,
                                cx,
                            );
                        });
                    }));
                }
                menu
            })
            .into_any_element()
    } else {
        let requests = requests_flat(tree);
        let tab_id = tab_id.to_string();
        trigger
            .dropdown_menu(move |mut menu, _, _| {
                for (id, label) in &requests {
                    let target_weak = weak.clone();
                    let tab_id = tab_id.clone();
                    let id = id.clone();
                    let label = label.clone();
                    menu =
                        menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, _, cx| {
                            let id = id.clone();
                            let label = label.clone();
                            let _ = target_weak.update(cx, |view, cx| {
                                view.pick_load_test_target(
                                    &tab_id,
                                    LoadTestTarget::Request(id),
                                    label,
                                    1,
                                    cx,
                                );
                            });
                        }));
                }
                menu
            })
            .into_any_element()
    }
}

/// `"1 request"` or `"N requests"`, in the current language.
fn request_count_label(count: usize) -> String {
    locale::plural("load_test.config.requests", count)
}

/// One numeric field, half the panel wide: a label above a bordered pill with the input and its
/// unit suffix.
#[allow(clippy::too_many_arguments)]
fn render_field(
    palette: &Palette,
    mono_font: &SharedString,
    ui_font: &SharedString,
    label: String,
    entity: Option<&Entity<InputState>>,
    unit: &'static str,
    disabled: bool,
    cx: &App,
) -> AnyElement {
    // An id keeps the pill's hover state, so entering and leaving renders again.
    let pill_id = SharedString::from(format!("load-test-field-{label}"));
    let row = div()
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(palette.fg_muted)
        .truncate()
        .child(label);

    let mut pill = h_flex()
        .id(pill_id)
        .h(px(CONTROL_HEIGHT))
        .items_center()
        .px(px(10.0))
        .rounded(px(RADIUS_MD - 1.0))
        .border_1()
        .border_color(palette.border)
        // The hover of every text input (telaio's `HoverInput`), not while a run disables it.
        .when(!disabled, |pill| {
            let border_strong = palette.border_strong;
            pill.hover(move |style| style.border_color(border_strong))
        })
        .bg(palette.raised)
        .font_family(mono_font.clone())
        .text_size(px(12.5));
    if let Some(entity) = entity {
        pill = pill.child(
            Input::new(entity)
                .context_menu(edit_menu(entity, cx))
                .px(px(INPUT_PADDING_X))
                .flex_1()
                .h(px(CONTROL_HEIGHT))
                .py_0()
                .bordered(false)
                .disabled(disabled)
                .bg(palette.bg.opacity(0.0)),
        );
    }
    // The interface font, not `mono_font` (the unit suffix is drawn in the UI
    // font while the value itself is mono, and `pill`'s own `font_family` above would otherwise
    // cascade to this child too).
    pill = pill.child(
        div()
            .font_family(ui_font.clone())
            .text_size(px(12.0))
            .text_color(palette.fg_subtle)
            .child(unit),
    );

    v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(6.0))
        .child(row)
        .child(pill)
        .into_any_element()
}

/// The "Thresholds" section: a label with its hint, one row per threshold, and the "Add
/// threshold" menu of the metrics that have none yet.
#[allow(clippy::too_many_arguments)]
fn render_thresholds(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    palette: &Palette,
    mono_font: &SharedString,
    ui_font: &SharedString,
    load_test: &LoadTestTab,
    entities: &[(ThresholdMetric, Entity<InputState>)],
    disabled: bool,
    cx: &App,
) -> AnyElement {
    let mut section = v_flex().gap(px(6.0)).child(
        h_flex()
            .gap(px(8.0))
            .justify_between()
            .text_size(px(12.0))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(palette.fg_muted)
                    .child(t!("load_test.thresholds.title")),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(palette.fg_subtle)
                    .child(t!("load_test.thresholds.hint")),
            ),
    );

    for row in &load_test.config.thresholds {
        let entity = entities
            .iter()
            .find(|(metric, _)| *metric == row.metric)
            .map(|(_, entity)| entity);
        let remove_weak = weak.clone();
        let remove_tab_id = tab_id.to_string();
        let metric = row.metric;
        section = section.child(render_threshold_row(
            palette,
            mono_font,
            ui_font,
            row,
            entity,
            disabled,
            move |cx| {
                let _ = remove_weak.update(cx, |view, cx| {
                    view.remove_load_test_threshold(&remove_tab_id, metric, cx);
                });
            },
            cx,
        ));
    }

    let missing = load_test.missing_threshold_metrics();
    if !missing.is_empty() {
        let tab_id = tab_id.to_string();
        let button = Button::new("load-test-add-threshold")
            .neutral_ghost(cx, palette.accent_text)
            .h(px(28.0))
            .px(px(4.0))
            .text_size(px(12.5))
            .disabled(disabled)
            .when(disabled, |button| button.opacity(0.5))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(Icon::new(IconName::Plus).small())
                    .child(t!("load_test.thresholds.add")),
            );
        let button = if disabled {
            button.into_any_element()
        } else {
            button
                .dropdown_menu(move |mut menu, _, _| {
                    for metric in &missing {
                        let metric = *metric;
                        let item_weak = weak.clone();
                        let tab_id = tab_id.clone();
                        menu =
                            menu.item(PopupMenuItem::new(threshold_metric_label(metric)).on_click(
                                move |_, _, cx| {
                                    let _ = item_weak.update(cx, |view, cx| {
                                        view.add_load_test_threshold(&tab_id, metric, cx);
                                    });
                                },
                            ));
                    }
                    menu
                })
                .into_any_element()
        };
        section = section.child(h_flex().child(button));
    }

    section.into_any_element()
}

/// One threshold row: the metric, its operator, the limit input with its unit, and a remove
/// button. The border turns `danger` while the limit does not parse.
#[allow(clippy::too_many_arguments)]
fn render_threshold_row(
    palette: &Palette,
    mono_font: &SharedString,
    ui_font: &SharedString,
    row: &ThresholdInput,
    entity: Option<&Entity<InputState>>,
    disabled: bool,
    on_remove: impl Fn(&mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let metric = row.metric;
    let invalid = row.resolved().is_none();
    let border = if invalid {
        palette.danger
    } else {
        palette.border
    };
    let mut pill = h_flex()
        .id(SharedString::from(format!(
            "load-test-threshold-{metric:?}"
        )))
        .h(px(CONTROL_HEIGHT))
        .items_center()
        .gap(px(8.0))
        .pl(px(10.0))
        .pr(px(4.0))
        .rounded(px(RADIUS_MD - 1.0))
        .border_1()
        .border_color(border)
        .when(!disabled && !invalid, |pill| {
            let border_strong = palette.border_strong;
            pill.hover(move |style| style.border_color(border_strong))
        })
        .bg(palette.raised)
        .text_size(px(12.5))
        .child(
            div()
                .min_w(px(52.0))
                .flex_none()
                .text_color(palette.fg_muted)
                .child(threshold_metric_label(metric)),
        )
        .child(
            div()
                .flex_none()
                .font_family(mono_font.clone())
                .text_color(palette.fg_subtle)
                .child(metric.operator()),
        );
    if let Some(entity) = entity {
        pill = pill.child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(mono_font.clone())
                .child(
                    Input::new(entity)
                        .context_menu(edit_menu(entity, cx))
                        .px_0()
                        .h(px(CONTROL_HEIGHT))
                        .py_0()
                        .bordered(false)
                        .disabled(disabled)
                        .bg(palette.bg.opacity(0.0)),
                ),
        );
    }
    let unit = threshold_unit(metric);
    if !unit.is_empty() {
        pill = pill.child(
            div()
                .flex_none()
                .font_family(ui_font.clone())
                .text_size(px(12.0))
                .text_color(palette.fg_subtle)
                .child(unit),
        );
    }
    pill.child(
        IconButton::new(
            SharedString::from(format!("load-test-threshold-remove-{metric:?}")),
            IconName::Close,
        )
        .disabled(disabled)
        .on_click(move |_, _, cx| on_remove(cx)),
    )
    .into_any_element()
}

/// A switch row ("Stop on errors", "Reuse connections"): a label and a hint on the left, the
/// switch on the right, dimmed while a run is in progress.
fn render_switch_row(
    id: &'static str,
    label: String,
    hint: String,
    checked: bool,
    palette: &Palette,
    is_running: bool,
    on_change: impl Fn(bool, &mut App) + 'static,
) -> AnyElement {
    h_flex()
        .items_center()
        .justify_between()
        .gap(px(12.0))
        .when(is_running, |row| row.opacity(0.5))
        .child(
            v_flex()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(palette.fg_subtle)
                        .child(hint),
                ),
        )
        .child(switch_halo(
            SharedString::from(format!("{id}-halo")),
            Switch::new(id)
                .checked(checked)
                .on_click(move |checked, _, cx| on_change(*checked, cx)),
            palette,
        ))
        .into_any_element()
}

/// The primary "Start run" button, replaced by a danger-text "Stop run" secondary button while
/// running.
fn render_start_stop_button(
    weak: WeakEntity<AppView>,
    tab_id: &str,
    is_running: bool,
) -> AnyElement {
    let tab_id = tab_id.to_string();
    if is_running {
        let stop_id = tab_id.clone();
        return DangerButton::new("load-test-stop", t!("load_test.config.stop_run"))
            .on_click(move |_, _, cx| {
                let stop_id = stop_id.clone();
                let _ = weak.update(cx, |view, cx| {
                    view.stop_load_test(&stop_id);
                    cx.notify();
                });
            })
            .into_any_element();
    }
    PrimaryButton::new("load-test-start", t!("load_test.config.start_run"))
        .on_click(move |_, _, cx| {
            let tab_id = tab_id.clone();
            let _ = weak.update(cx, |view, cx| view.start_load_test(tab_id, cx));
        })
        .into_any_element()
}
