//! [`TimeSeriesChart`]: a chart of several series over one shared x axis, as in the "performance
//! over time" card of Postino's load test results.
//!
//! The x axis has `N` evenly spaced points (one per second, say). Each series sits on the left or
//! the right y axis, and each axis has its own maximum (the bottom is always 0) and its own tick
//! labels, which the app passes as strings. Series are lines, dashed lines or filled areas, drawn
//! in the order they were added, and `f64::NAN` values are gaps. Reference lines mark a goal,
//! an axis can have a title above its labels (its unit, say), and [`ChartLabel`]s write a short
//! text next to a point (the peak of a series that has no axis of its own).
//!
//! Hovering shows a crosshair at the nearest point, a dot per series and a tooltip card. Pressing
//! and dragging over the plot selects a range of points (a brush), which the app keeps and passes
//! back with [`TimeSeriesChart::selection`]. The hover and the drag in progress are state of the
//! component ([`Window::use_keyed_state`]), and it re-renders only when the nearest point
//! changes, so an idle window does no work. There are no timers and no animation.
//!
//! Colors are never chosen here: series, axis labels and reference lines take the colors the app
//! passes, and the rest comes from [`crate::Palette`]. A series the app wants to hide (a legend
//! toggle) is simply not added.
//!
//! The math (value to y, index to x, x to nearest index, selection, tooltip placement) is in
//! plain functions at the top of the file, unit tested without a window.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::*;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::PaletteExt;
use crate::fonts::MONO_FAMILY;
use crate::metrics::{
    CHART_DOT_SIZE, CHART_GUTTER_LEFT, CHART_GUTTER_RIGHT, CHART_LABEL_GAP, CHART_LABEL_HEIGHT,
    CHART_PAD_BOTTOM, CHART_PAD_TOP, CHART_PAD_TOP_TITLED, CHART_TICK_GAP, CHART_TICK_TEXT_SIZE,
    CHART_TOOLTIP_EDGE, CHART_TOOLTIP_OFFSET, CHART_TOOLTIP_TOP, CHART_TOOLTIP_WIDTH,
    CHART_X_LABEL_GAP, CHART_X_LABEL_WIDTH, RADIUS_MD,
};

/// Height of one axis label's box, which is vertically centered on its tick.
const TICK_LABEL_HEIGHT: f32 = 14.0;
/// Opacity of the hover crosshair, drawn in the text color.
const CROSSHAIR_OPACITY: f32 = 0.35;
/// Opacity of the selection band's fill, drawn in the accent color.
const BAND_OPACITY: f32 = 0.14;
/// Opacity of the selection band's two edges, drawn in the accent text color.
const BAND_EDGE_OPACITY: f32 = 0.6;
/// Opacity of a reference line, against its label's full color.
const REFERENCE_LINE_OPACITY: f32 = 0.55;
/// The dash pattern of a reference line: dash, then gap, in pixels.
const REFERENCE_DASH: [f32; 2] = [4.0, 3.0];

/// A selected range of points: first and last index, both included, `first < last`.
pub type ChartSelection = Option<(usize, usize)>;

/// A tooltip value formatter, from the plotted value.
type ValueFormat = Rc<dyn Fn(f64) -> SharedString>;
/// The time line formatter of the tooltip, from the index of the point.
type TimeFormat = Rc<dyn Fn(usize) -> SharedString>;
/// The selection handler: `Some((first, last))` for a new range, `None` to clear it.
type SelectHandler = Rc<dyn Fn(ChartSelection, &mut Window, &mut App)>;

// Math: plain functions, unit tested.

/// The fraction (0 through 1) of the plot's width where point `index` of `count` sits. A chart
/// with a single point puts it in the middle.
fn x_fraction(index: usize, count: usize) -> f32 {
    if count <= 1 {
        0.5
    } else {
        index.min(count - 1) as f32 / (count - 1) as f32
    }
}

/// The x offset, in pixels from the left of the plot, of point `index` of `count`.
fn index_x(index: usize, count: usize, plot_width: f32) -> f32 {
    x_fraction(index, count) * plot_width
}

/// The index of the point nearest to `x`, an offset in pixels from the left of the plot.
/// Clamped to the first and last point.
fn nearest_index(x: f32, plot_width: f32, count: usize) -> usize {
    if count <= 1 || plot_width <= 0.0 || x.is_nan() {
        return 0;
    }
    let fraction = (x / plot_width).clamp(0.0, 1.0);
    (fraction * (count - 1) as f32).round() as usize
}

/// The y offset, in pixels from the top of the plot, of `value` on a scale from 0 (the bottom of
/// the plot) to `max` (the top). Values outside the scale are clamped to the plot. A NaN value
/// gives NaN.
fn value_y(value: f64, max: f64, plot_height: f32) -> f32 {
    if value.is_nan() {
        return f32::NAN;
    }
    let fraction = if max > 0.0 {
        (value / max).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    plot_height - fraction * plot_height
}

/// The fraction (0 through 1) of an axis where tick `index` of `count` sits, from the bottom.
fn tick_fraction(index: usize, count: usize) -> f32 {
    if count <= 1 {
        0.0
    } else {
        index.min(count - 1) as f32 / (count - 1) as f32
    }
}

/// `values[index]`, or NaN (a gap) past the end of the data.
fn value_at(values: &[f64], index: usize) -> f64 {
    values.get(index).copied().unwrap_or(f64::NAN)
}

/// The runs of consecutive points of the first `count` that have a value, as index ranges. NaN
/// and infinite values end a run, so they show as gaps.
fn finite_runs(values: &[f64], count: usize) -> Vec<Range<usize>> {
    let mut runs = Vec::new();
    let mut start: Option<usize> = None;
    for index in 0..count {
        match (value_at(values, index).is_finite(), start) {
            (true, None) => start = Some(index),
            (false, Some(first)) => {
                runs.push(first..index);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(first) = start {
        runs.push(first..count);
    }
    runs
}

/// The two ends of a drag in order, `(first, last)`.
fn normalize_selection(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// What releasing a drag from point `a` to point `b` selects: the range when there is at least
/// one point between the ends, `None` (clear the selection) for a click.
fn drag_selection(a: usize, b: usize) -> ChartSelection {
    let (first, last) = normalize_selection(a, b);
    (last > first).then_some((first, last))
}

/// The fractions of the plot's width where the band of the range `first..=last` starts and ends.
/// The ends are put in order and kept inside the `count` points.
fn band_fractions(first: usize, last: usize, count: usize) -> (f32, f32) {
    let (first, last) = normalize_selection(first, last);
    (x_fraction(first, count), x_fraction(last, count))
}

/// The x offset, in pixels from the left of the plot, of the tooltip's left edge when the
/// crosshair is at `x`: to the right of the crosshair, or to its left when the tooltip would come
/// within [`CHART_TOOLTIP_EDGE`] of the plot's right edge. Never negative.
fn tooltip_left(x: f32, plot_width: f32, tooltip_width: f32) -> f32 {
    let left = if x > plot_width - (tooltip_width + CHART_TOOLTIP_OFFSET + CHART_TOOLTIP_EDGE) {
        x - CHART_TOOLTIP_OFFSET - tooltip_width
    } else {
        x + CHART_TOOLTIP_OFFSET
    };
    left.max(0.0)
}

/// A value written as an integer when it is one, with two decimals otherwise. The formatter of a
/// series that has none of its own.
fn default_format(value: f64) -> SharedString {
    if value.fract() == 0.0 {
        format!("{value:.0}").into()
    } else {
        format!("{value:.2}").into()
    }
}

// State.

/// What the component remembers between renders.
#[derive(Default)]
struct ChartState {
    /// The plot's bounds in the window, written when the chart paints.
    plot: Bounds<Pixels>,
    /// The point under the pointer.
    hover: Option<usize>,
    /// The drag in progress: the point where it started and the point it is at.
    drag: Option<(usize, usize)>,
}

impl ChartState {
    /// The pointer moved over point `index`. Returns whether anything shown changed.
    fn pointer_moved(&mut self, index: usize) -> bool {
        let mut changed = self.hover != Some(index);
        self.hover = Some(index);
        if let Some((_, current)) = self.drag.as_mut()
            && *current != index
        {
            *current = index;
            changed = true;
        }
        changed
    }

    /// The button went down over point `index`: a drag starts there.
    fn pressed(&mut self, index: usize) {
        self.hover = Some(index);
        self.drag = Some((index, index));
    }

    /// The button went up or the pointer left the plot: ends the drag. The outer `None` means no
    /// drag was in progress, the inner value is what to report, see [`drag_selection`].
    fn finish_drag(&mut self) -> Option<ChartSelection> {
        self.drag
            .take()
            .map(|(start, end)| drag_selection(start, end))
    }

    /// The pointer left the plot. Returns whether anything shown changed, and the selection to
    /// report if a drag ended.
    fn left(&mut self) -> (bool, Option<ChartSelection>) {
        let changed = self.hover.take().is_some();
        let finished = self.finish_drag();
        (changed || finished.is_some(), finished)
    }

    /// The point index under `x`, a window x coordinate, or `None` before the first paint.
    fn index_at(&self, x: Pixels, count: usize) -> Option<usize> {
        let width = f32::from(self.plot.size.width);
        (width > 0.0).then(|| nearest_index(f32::from(x - self.plot.origin.x), width, count))
    }
}

// Public data types.

/// The y axis a series or reference line is scaled to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AxisSide {
    /// The axis on the left of the plot.
    #[default]
    Left,
    /// The axis on the right of the plot.
    Right,
}

/// A y axis: its maximum, its tick labels, their color and an optional title. The bottom of the
/// plot is always 0.
#[derive(Clone)]
pub struct ChartAxis {
    max: f64,
    labels: Vec<SharedString>,
    label_color: Option<Hsla>,
    title: Option<SharedString>,
}

impl ChartAxis {
    /// An axis from 0 to `max`, with one tick per label, evenly spaced from the bottom (the first
    /// label) to the top (the last). The labels are the strings the app wants to show, for
    /// example `["0", "600", "1,200", "1,800", "2,400/s"]`.
    pub fn new<L: Into<SharedString>>(max: f64, labels: impl IntoIterator<Item = L>) -> Self {
        Self {
            max,
            labels: labels.into_iter().map(Into::into).collect(),
            label_color: None,
            title: None,
        }
    }

    /// A title above the axis' labels, in their color, for example its unit (`req/s`). A chart
    /// with a titled axis leaves more room above the plot for it.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// The color of the tick labels, `fg_muted` by default. An app can use a series' color for
    /// its axis.
    pub fn label_color(mut self, color: Hsla) -> Self {
        self.label_color = Some(color);
        self
    }
}

/// How a series is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeriesStyle {
    /// A solid line of this width, in pixels.
    Line {
        /// Stroke width.
        width: f32,
    },
    /// A dashed line.
    Dashed {
        /// Stroke width.
        width: f32,
        /// Length of a dash, in pixels.
        dash: f32,
        /// Length of the gap between dashes, in pixels.
        gap: f32,
    },
    /// An area down to 0, filled, with a thin outline along its top.
    Area {
        /// The fill, usually the series color at a low alpha.
        fill: Hsla,
        /// The outline color.
        outline: Hsla,
        /// Outline width.
        width: f32,
    },
}

impl SeriesStyle {
    /// A solid line `width` pixels wide.
    pub fn line(width: f32) -> Self {
        Self::Line { width }
    }

    /// A dashed line `width` pixels wide, with `dash` pixel dashes and `gap` pixel gaps.
    pub fn dashed(width: f32, dash: f32, gap: f32) -> Self {
        Self::Dashed { width, dash, gap }
    }

    /// A filled area with a 1 px outline.
    pub fn area(fill: Hsla, outline: Hsla) -> Self {
        Self::Area {
            fill,
            outline,
            width: 1.0,
        }
    }
}

/// One series of a [`TimeSeriesChart`]: a label, a color, the values and how to draw them.
#[derive(Clone)]
pub struct ChartSeries {
    label: SharedString,
    color: Hsla,
    values: Arc<[f64]>,
    side: AxisSide,
    style: SeriesStyle,
    scale_max: Option<f64>,
    format: ValueFormat,
    dot: bool,
    tooltip_order: i32,
}

impl ChartSeries {
    /// A series called `label`, with its `values` (one per point; `f64::NAN` is a gap, and points
    /// past the end of the values are gaps too). `color` is the one of the line, the tooltip's
    /// swatch and the hover dot. It is a solid line on the left axis, 1.5 px wide, with a hover
    /// dot, until the builder methods say otherwise. Pass an `Arc<[f64]>` to share the data with
    /// the app instead of copying it on every render.
    pub fn new(label: impl Into<SharedString>, color: Hsla, values: impl Into<Arc<[f64]>>) -> Self {
        Self {
            label: label.into(),
            color,
            values: values.into(),
            side: AxisSide::Left,
            style: SeriesStyle::line(1.5),
            scale_max: None,
            format: Rc::new(default_format),
            dot: true,
            tooltip_order: 0,
        }
    }

    /// The y axis the series is scaled to.
    pub fn side(mut self, side: AxisSide) -> Self {
        self.side = side;
        self
    }

    /// How the series is drawn.
    pub fn style(mut self, style: SeriesStyle) -> Self {
        self.style = style;
        self
    }

    /// Scales this series to its own maximum instead of its axis' one, for a series that has no
    /// tick labels (the virtual users area under the other series, say).
    pub fn scale_max(mut self, max: f64) -> Self {
        self.scale_max = Some(max);
        self
    }

    /// Writes the value shown in the tooltip, from the series' value at the hovered point.
    pub fn format(mut self, format: impl Fn(f64) -> SharedString + 'static) -> Self {
        self.format = Rc::new(format);
        self
    }

    /// Whether hovering marks the series with a dot on its line. On by default; an area under the
    /// other series usually has none.
    pub fn dot(mut self, dot: bool) -> Self {
        self.dot = dot;
        self
    }

    /// Where the series' row goes in the tooltip: rows are sorted by this key, lowest first, and
    /// series with the same key keep the order they were added in. All `0` by default, so the
    /// tooltip follows the drawing order unless the app lists the series like its legend.
    pub fn tooltip_order(mut self, order: i32) -> Self {
        self.tooltip_order = order;
        self
    }
}

/// A horizontal dashed line at a value of an axis, with a small label above its left end.
#[derive(Clone)]
pub struct ReferenceLine {
    side: AxisSide,
    value: f64,
    label: SharedString,
    color: Hsla,
}

impl ReferenceLine {
    /// A line at `value` on the `side` axis, labeled `label`. The label has the full `color`, the
    /// line is the same color at a lower opacity.
    pub fn new(side: AxisSide, value: f64, label: impl Into<SharedString>, color: Hsla) -> Self {
        Self {
            side,
            value,
            label: label.into(),
            color,
        }
    }
}

/// Which side of its point a [`ChartLabel`] sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelPlacement {
    /// Ending just left of the point and just below it: the end of a plateau, inside its area.
    #[default]
    BeforeBelow,
    /// Ending just left of the point and just above it: the end of a plateau too thin to hold
    /// the label under it.
    BeforeAbove,
    /// Starting just right of the point and just above it: a live edge.
    AfterAbove,
    /// Starting just right of the point, its bottom on the bottom of the plot: a caption at the
    /// foot of a slope, whatever the point's value.
    AfterOnBase,
}

/// A short text next to one point of the chart, such as the peak of a series without an axis of
/// its own (`50 VUs`). The point is a value on an axis, or on a scale of its own like a series'
/// [`ChartSeries::scale_max`]. Boxed labels sit on a small card in the surface color, so they stay
/// readable over the series.
#[derive(Clone)]
pub struct ChartLabel {
    index: usize,
    value: f64,
    text: SharedString,
    side: AxisSide,
    scale_max: Option<f64>,
    placement: LabelPlacement,
    boxed: bool,
}

impl ChartLabel {
    /// A label for point `index` at `value` on the left axis, before and below it, boxed.
    pub fn new(index: usize, value: f64, text: impl Into<SharedString>) -> Self {
        Self {
            index,
            value,
            text: text.into(),
            side: AxisSide::Left,
            scale_max: None,
            placement: LabelPlacement::default(),
            boxed: true,
        }
    }

    /// The axis the value is on, the left one by default.
    pub fn side(mut self, side: AxisSide) -> Self {
        self.side = side;
        self
    }

    /// A scale of its own from 0 to `max`, like the series it labels.
    pub fn scale_max(mut self, max: f64) -> Self {
        self.scale_max = Some(max);
        self
    }

    /// Where the label sits around its point.
    pub fn placement(mut self, placement: LabelPlacement) -> Self {
        self.placement = placement;
        self
    }

    /// Whether the label sits on a small card (the default) or is bare text.
    pub fn boxed(mut self, boxed: bool) -> Self {
        self.boxed = boxed;
        self
    }
}

/// The top of a label's box, in pixels from the top of the plot, for a point at `y` of a plot
/// `plot_height` tall.
fn label_top(placement: LabelPlacement, y: f32, plot_height: f32) -> f32 {
    match placement {
        LabelPlacement::BeforeBelow => y + CHART_LABEL_GAP,
        LabelPlacement::BeforeAbove | LabelPlacement::AfterAbove => {
            y - CHART_LABEL_GAP - CHART_LABEL_HEIGHT
        }
        LabelPlacement::AfterOnBase => plot_height - CHART_LABEL_GAP - CHART_LABEL_HEIGHT,
    }
}

// The element.

/// A chart of several series over one shared x axis, with hover and range selection. See the
/// [module docs](self).
///
/// The width fills the parent and the height is set by the app ([`Self::height`]). A strip on
/// each side of the plot holds the y axis labels, and the x labels go below.
#[derive(IntoElement)]
pub struct TimeSeriesChart {
    id: ElementId,
    count: usize,
    height: f32,
    gutters: (f32, f32),
    left: Option<ChartAxis>,
    right: Option<ChartAxis>,
    x_labels: Vec<SharedString>,
    series: Vec<ChartSeries>,
    references: Vec<ReferenceLine>,
    labels: Vec<ChartLabel>,
    time_label: Option<TimeFormat>,
    selection: ChartSelection,
    surface: Option<Hsla>,
    on_select: Option<SelectHandler>,
}

impl TimeSeriesChart {
    /// A chart of `count` points on the x axis, with the 220 px height of Postino's load test.
    /// `id` keeps the hover and the drag between renders, so it must be unique in the window.
    pub fn new(id: impl Into<ElementId>, count: usize) -> Self {
        Self {
            id: id.into(),
            count,
            height: CHART_PAD_TOP + 188.0 + CHART_PAD_BOTTOM,
            gutters: (CHART_GUTTER_LEFT, CHART_GUTTER_RIGHT),
            left: None,
            right: None,
            x_labels: Vec::new(),
            series: Vec::new(),
            references: Vec::new(),
            labels: Vec::new(),
            time_label: None,
            selection: None,
            surface: None,
            on_select: None,
        }
    }

    /// The total height in pixels, x labels included. The plot is that minus 32 px, or minus
    /// 48 px when an axis has a title.
    pub fn height(mut self, height: f32) -> Self {
        self.height = height;
        self
    }

    /// The width of the strips left and right of the plot, for the y axis labels.
    pub fn gutters(mut self, left: f32, right: f32) -> Self {
        self.gutters = (left, right);
        self
    }

    /// The left y axis. Its ticks also set the horizontal gridlines.
    pub fn left_axis(mut self, axis: ChartAxis) -> Self {
        self.left = Some(axis);
        self
    }

    /// The right y axis. The gridlines follow it only when there is no left axis.
    pub fn right_axis(mut self, axis: ChartAxis) -> Self {
        self.right = Some(axis);
        self
    }

    /// The x axis labels, evenly spaced from the first point to the last, for example
    /// `["0s", "10s", "20s"]`.
    pub fn x_labels<L: Into<SharedString>>(mut self, labels: impl IntoIterator<Item = L>) -> Self {
        self.x_labels = labels.into_iter().map(Into::into).collect();
        self
    }

    /// Adds a series. They are drawn in the order added, so the last one is on top.
    pub fn series(mut self, series: ChartSeries) -> Self {
        self.series.push(series);
        self
    }

    /// Adds a reference line.
    pub fn reference_line(mut self, line: ReferenceLine) -> Self {
        self.references.push(line);
        self
    }

    /// Adds a label next to a point.
    pub fn label(mut self, label: ChartLabel) -> Self {
        self.labels.push(label);
        self
    }

    /// Writes the first line of the tooltip from the index of the hovered point, for example
    /// `00:36 - 00:37`. Without it the tooltip has no time line.
    pub fn time_label(mut self, label: impl Fn(usize) -> SharedString + 'static) -> Self {
        self.time_label = Some(Rc::new(label));
        self
    }

    /// The committed selection, first and last point both included, drawn as a band. The app
    /// keeps it: set it from [`Self::on_select`].
    pub fn selection(mut self, selection: ChartSelection) -> Self {
        self.selection = selection;
        self
    }

    /// The color of the surface the chart sits on, which the hover dots use for their ring.
    /// `raised` by default, the color of a [`super::Card`].
    pub fn surface(mut self, color: Hsla) -> Self {
        self.surface = Some(color);
        self
    }

    /// Called when a drag over the plot ends (the button went up or the pointer left the plot)
    /// with `Some((first, last))`, both included, when the drag crossed at least one point. A
    /// click without a drag calls it with `None`: the app clears its selection.
    pub fn on_select(
        mut self,
        handler: impl Fn(ChartSelection, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }

    /// The maximum of the axis on `side`; the other axis when there is none on that side, and 1
    /// when the chart has no axis at all.
    fn axis_max(&self, side: AxisSide) -> f64 {
        let (own, other) = match side {
            AxisSide::Left => (&self.left, &self.right),
            AxisSide::Right => (&self.right, &self.left),
        };
        own.as_ref().or(other.as_ref()).map_or(1.0, |axis| axis.max)
    }
}

/// What the canvas needs to draw a series.
struct SeriesPaint {
    values: Arc<[f64]>,
    color: Hsla,
    style: SeriesStyle,
    max: f64,
}

/// What the canvas needs to draw a reference line.
struct ReferencePaint {
    y: f32,
    color: Hsla,
}

/// A stroked path through `points`, dashed when `dash` is `(dash, gap)`. `None` for fewer than
/// two points, or when the path cannot be built.
fn stroke_path(
    points: &[Point<Pixels>],
    width: f32,
    dash: Option<(f32, f32)>,
) -> Option<Path<Pixels>> {
    let (first, rest) = points.split_first()?;
    if rest.is_empty() {
        return None;
    }
    let mut builder = PathBuilder::stroke(px(width));
    if let Some((dash, gap)) = dash {
        builder = builder.dash_array(&[px(dash), px(gap)]);
    }
    builder.move_to(*first);
    for point in rest {
        builder.line_to(*point);
    }
    builder.build().ok()
}

/// A filled path through `points` and down to the `baseline` y. `None` for fewer than two
/// points, or when the path cannot be built.
fn area_path(points: &[Point<Pixels>], baseline: Pixels) -> Option<Path<Pixels>> {
    let (first, rest) = points.split_first()?;
    let last = points.last()?;
    if rest.is_empty() {
        return None;
    }
    let mut builder = PathBuilder::fill();
    builder.move_to(point(first.x, baseline));
    builder.line_to(*first);
    for point in rest {
        builder.line_to(*point);
    }
    builder.line_to(gpui_kit::point(last.x, baseline));
    builder.close();
    builder.build().ok()
}

/// Paints the series and the reference lines into the plot at `bounds`.
fn paint_plot(
    bounds: Bounds<Pixels>,
    count: usize,
    series: &[SeriesPaint],
    references: &[ReferencePaint],
    window: &mut Window,
) {
    let width = f32::from(bounds.size.width);
    let height = f32::from(bounds.size.height);
    for series in series {
        for run in finite_runs(&series.values, count) {
            let points: Vec<Point<Pixels>> = run
                .map(|index| {
                    point(
                        bounds.origin.x + px(index_x(index, count, width)),
                        bounds.origin.y + px(value_y(series.values[index], series.max, height)),
                    )
                })
                .collect();
            match series.style {
                SeriesStyle::Line { width } => {
                    if let Some(path) = stroke_path(&points, width, None) {
                        window.paint_path(path, series.color);
                    }
                }
                SeriesStyle::Dashed { width, dash, gap } => {
                    if let Some(path) = stroke_path(&points, width, Some((dash, gap))) {
                        window.paint_path(path, series.color);
                    }
                }
                SeriesStyle::Area {
                    fill,
                    outline,
                    width,
                } => {
                    if let Some(path) = area_path(&points, bounds.origin.y + bounds.size.height) {
                        window.paint_path(path, fill);
                    }
                    if let Some(path) = stroke_path(&points, width, None) {
                        window.paint_path(path, outline);
                    }
                }
            }
        }
    }
    for reference in references {
        // Half a pixel off, so the 1 px line covers whole pixels instead of blurring over two.
        let y = bounds.origin.y + px(reference.y.floor() + 0.5);
        let ends = [
            point(bounds.origin.x, y),
            point(bounds.origin.x + bounds.size.width, y),
        ];
        let dash = Some((REFERENCE_DASH[0], REFERENCE_DASH[1]));
        if let Some(path) = stroke_path(&ends, 1.0, dash) {
            window.paint_path(path, reference.color);
        }
    }
}

impl RenderOnce for TimeSeriesChart {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = cx.palette();
        let state =
            window.use_keyed_state(ElementId::from((self.id.clone(), "state")), cx, |_, _| {
                ChartState::default()
            });
        let (hover, drag, plot_width) = {
            let state = state.read(cx);
            (state.hover, state.drag, f32::from(state.plot.size.width))
        };

        let count = self.count;
        let titled = [&self.left, &self.right]
            .into_iter()
            .any(|axis| axis.as_ref().is_some_and(|axis| axis.title.is_some()));
        let pad_top = if titled {
            CHART_PAD_TOP_TITLED
        } else {
            CHART_PAD_TOP
        };
        let plot_height = (self.height - pad_top - CHART_PAD_BOTTOM).max(0.0);
        let (gutter_left, gutter_right) = self.gutters;
        let ring = self.surface.unwrap_or(palette.raised);
        let max_of = |side, series: &ChartSeries| series.scale_max.unwrap_or(self.axis_max(side));

        // Gridlines follow the left axis' ticks, or the right one's.
        let grid_ticks = self
            .left
            .as_ref()
            .or(self.right.as_ref())
            .map_or(0, |axis| axis.labels.len());
        let grid = (0..grid_ticks).map(|tick| {
            let y = plot_height - tick_fraction(tick, grid_ticks) * plot_height;
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(y))
                .h(px(1.0))
                .bg(if tick == 0 {
                    palette.border_strong
                } else {
                    palette.border
                })
        });

        // The y axis labels sit in the gutters, centered on their tick.
        // The title, if any, sits at the very top of the gutter, above the highest label.
        let y_labels = |axis: &Option<ChartAxis>, side: AxisSide| -> Vec<Div> {
            let Some(axis) = axis else {
                return Vec::new();
            };
            let color = axis.label_color.unwrap_or(palette.fg_muted);
            let ticks = axis.labels.len();
            let title = axis.title.as_ref().map(|title| (0.0, title, true));
            let tick_labels = axis.labels.iter().enumerate().map(|(tick, label)| {
                let y = pad_top + plot_height - tick_fraction(tick, ticks) * plot_height;
                (y - TICK_LABEL_HEIGHT / 2.0, label, false)
            });
            title
                .into_iter()
                .chain(tick_labels)
                .map(|(top, label, is_title)| {
                    let base = div()
                        .absolute()
                        .top(px(top))
                        .when(is_title, |title| title.font_weight(FontWeight::SEMIBOLD))
                        .h(px(TICK_LABEL_HEIGHT))
                        .line_height(px(TICK_LABEL_HEIGHT))
                        .text_size(px(CHART_TICK_TEXT_SIZE))
                        .font_family(MONO_FAMILY)
                        .text_color(color)
                        .whitespace_nowrap();
                    match side {
                        AxisSide::Left => base
                            .left_0()
                            .w(px((gutter_left - CHART_TICK_GAP).max(0.0)))
                            .text_right(),
                        AxisSide::Right => base
                            .right_0()
                            .w(px((gutter_right - CHART_TICK_GAP).max(0.0)))
                            .text_left(),
                    }
                    .child(label.clone())
                })
                .collect()
        };
        let left_labels = y_labels(&self.left, AxisSide::Left);
        let right_labels = y_labels(&self.right, AxisSide::Right);

        let x_label_count = self.x_labels.len();
        let x_labels = self.x_labels.iter().enumerate().map(|(tick, label)| {
            div()
                .absolute()
                .left(relative(tick_fraction(tick, x_label_count)))
                .ml(px(-CHART_X_LABEL_WIDTH / 2.0))
                .w(px(CHART_X_LABEL_WIDTH))
                .text_center()
                .child(label.clone())
        });

        // Reference lines: the line itself is painted with the series, the label is text.
        let reference_labels = self.references.iter().map(|line| {
            let y = value_y(line.value, self.axis_max(line.side), plot_height);
            div()
                .absolute()
                .left(px(4.0))
                .top(px(y - 15.0))
                .text_size(px(CHART_TICK_TEXT_SIZE))
                .font_family(MONO_FAMILY)
                .text_color(line.color)
                .whitespace_nowrap()
                .child(line.label.clone())
        });
        let reference_paints: Vec<ReferencePaint> = self
            .references
            .iter()
            .map(|line| ReferencePaint {
                y: value_y(line.value, self.axis_max(line.side), plot_height),
                color: line.color.opacity(REFERENCE_LINE_OPACITY),
            })
            .collect();

        // Labels next to points, drawn over the series and under the hover.
        let point_labels = self.labels.iter().filter(|_| count > 0).map(|label| {
            let max = label.scale_max.unwrap_or(self.axis_max(label.side));
            let y = value_y(label.value, max, plot_height);
            let fraction = x_fraction(label.index, count);
            let base = h_flex()
                .absolute()
                .top(px(label_top(label.placement, y, plot_height)))
                .h(px(CHART_LABEL_HEIGHT))
                .px(px(6.0))
                .items_center()
                .rounded(px(4.0))
                .text_size(px(CHART_TICK_TEXT_SIZE))
                .font_family(MONO_FAMILY)
                .text_color(palette.fg_muted)
                .whitespace_nowrap()
                .when(label.boxed, |boxed| {
                    boxed.bg(ring).border_1().border_color(palette.border)
                });
            match label.placement {
                LabelPlacement::BeforeBelow | LabelPlacement::BeforeAbove => {
                    base.right(relative(1.0 - fraction)).mr(px(CHART_LABEL_GAP))
                }
                LabelPlacement::AfterAbove | LabelPlacement::AfterOnBase => {
                    base.left(relative(fraction)).ml(px(CHART_LABEL_GAP))
                }
            }
            .child(label.text.clone())
        });

        // The band: the drag in progress, or else the committed selection.
        let band = drag.or(self.selection).filter(|_| count > 0);
        let band = band.map(|(first, last)| {
            let (start, end) = band_fractions(first, last, count);
            div()
                .absolute()
                .top_0()
                .h_full()
                .left(relative(start))
                .w(relative(end - start))
                .min_w(px(1.0))
                .bg(palette.accent.opacity(BAND_OPACITY))
                .border_l_1()
                .border_r_1()
                .border_color(palette.accent_text.opacity(BAND_EDGE_OPACITY))
        });

        // The hover: crosshair, a dot per series and the tooltip. None while dragging.
        let hovered = hover.filter(|index| drag.is_none() && *index < count && plot_width > 0.0);
        let hover_layer = hovered.map(|index| {
            let fraction = x_fraction(index, count);
            let x = fraction * plot_width;
            let dots = self
                .series
                .iter()
                .filter(|series| series.dot && value_at(&series.values, index).is_finite())
                .map(|series| {
                    let max = max_of(series.side, series);
                    let y = value_y(value_at(&series.values, index), max, plot_height);
                    div()
                        .absolute()
                        .left(relative(fraction))
                        .ml(px(-CHART_DOT_SIZE / 2.0))
                        .top(px(y - CHART_DOT_SIZE / 2.0))
                        .size(px(CHART_DOT_SIZE))
                        .rounded_full()
                        .bg(series.color)
                        .border_2()
                        .border_color(ring)
                });
            let mut listed: Vec<&ChartSeries> = self
                .series
                .iter()
                .filter(|series| value_at(&series.values, index).is_finite())
                .collect();
            listed.sort_by_key(|series| series.tooltip_order);
            let rows = listed.into_iter().map(|series| {
                h_flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_none()
                            .size(px(8.0))
                            .rounded(px(2.0))
                            .bg(series.color),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_color(palette.fg_muted)
                            .child(series.label.clone()),
                    )
                    .child(
                        div()
                            .font_family(MONO_FAMILY)
                            .text_color(palette.fg)
                            .child((series.format)(value_at(&series.values, index))),
                    )
            });
            let header = self.time_label.as_ref().map(|label| {
                div()
                    .font_family(MONO_FAMILY)
                    .text_size(px(11.0))
                    .text_color(palette.fg_muted)
                    .pb(px(2.0))
                    .child(label(index))
            });
            let tooltip = v_flex()
                .absolute()
                .top(px(CHART_TOOLTIP_TOP))
                .left(px(tooltip_left(x, plot_width, CHART_TOOLTIP_WIDTH)))
                .w(px(CHART_TOOLTIP_WIDTH))
                .gap(px(4.0))
                .py(px(8.0))
                .px(px(10.0))
                .rounded(px(RADIUS_MD))
                .border_1()
                .border_color(palette.border_strong)
                .bg(palette.overlay)
                .shadow(palette.shadow.clone())
                .text_size(px(12.0))
                .text_color(palette.fg)
                .children(header)
                .children(rows);
            div()
                .absolute()
                .size_full()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .h_full()
                        .left(relative(fraction))
                        .w(px(1.0))
                        .bg(palette.fg.opacity(CROSSHAIR_OPACITY)),
                )
                .children(dots)
                .child(tooltip)
        });

        // The canvas paints the series and remembers where the plot is, for the pointer.
        let series_paints: Vec<SeriesPaint> = self
            .series
            .iter()
            .map(|series| SeriesPaint {
                values: series.values.clone(),
                color: series.color,
                style: series.style,
                max: max_of(series.side, series),
            })
            .collect();
        let paint_state = state.clone();
        let canvas = canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                paint_state.update(cx, |state, _| state.plot = bounds);
                paint_plot(bounds, count, &series_paints, &reference_paints, window);
            },
        )
        .absolute()
        .size_full();

        // The pointer layer on top of everything. Only a change of the nearest point, the start
        // of a drag or the end of one notifies, so moving within a point costs nothing.
        let on_select = self.on_select.clone();
        let pointer = div()
            .id(ElementId::from((self.id.clone(), "plot")))
            .absolute()
            .size_full()
            .cursor_crosshair()
            .on_mouse_move({
                let state = state.clone();
                move |event, _, cx| {
                    state.update(cx, |state, cx| {
                        if let Some(index) = state.index_at(event.position.x, count)
                            && state.pointer_moved(index)
                        {
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let state = state.clone();
                move |event, _, cx| {
                    state.update(cx, |state, cx| {
                        if let Some(index) = state.index_at(event.position.x, count) {
                            state.pressed(index);
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let state = state.clone();
                let on_select = on_select.clone();
                move |_, window, cx| {
                    let finished = state.update(cx, |state, cx| {
                        let finished = state.finish_drag();
                        if finished.is_some() {
                            cx.notify();
                        }
                        finished
                    });
                    if let (Some(selection), Some(handler)) = (finished, on_select.as_ref()) {
                        handler(selection, window, cx);
                    }
                }
            })
            .on_hover({
                let state = state.clone();
                move |hovered, window, cx| {
                    if *hovered {
                        return;
                    }
                    let finished = state.update(cx, |state, cx| {
                        let (changed, finished) = state.left();
                        if changed {
                            cx.notify();
                        }
                        finished
                    });
                    if let (Some(selection), Some(handler)) = (finished, on_select.as_ref()) {
                        handler(selection, window, cx);
                    }
                }
            });

        div()
            .id(self.id.clone())
            .relative()
            .w_full()
            .h(px(self.height))
            .children(left_labels)
            .children(right_labels)
            .child(
                div()
                    .absolute()
                    .left(px(gutter_left))
                    .right(px(gutter_right))
                    .top(px(pad_top + plot_height + CHART_X_LABEL_GAP))
                    .h(px(TICK_LABEL_HEIGHT))
                    .line_height(px(TICK_LABEL_HEIGHT))
                    .text_size(px(CHART_TICK_TEXT_SIZE))
                    .font_family(MONO_FAMILY)
                    .text_color(palette.fg_subtle)
                    .whitespace_nowrap()
                    .children(x_labels),
            )
            .child(
                div()
                    .absolute()
                    .left(px(gutter_left))
                    .right(px(gutter_right))
                    .top(px(pad_top))
                    .h(px(plot_height))
                    .children(grid)
                    .children(reference_labels)
                    .children(band)
                    .child(canvas)
                    .children(point_labels)
                    .children(hover_layer)
                    .child(pointer),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use gpui_kit::{Bounds, Hsla, point, px, size};
    use pretty_assertions::assert_eq;

    use super::{
        AxisSide, ChartAxis, ChartSeries, ChartState, LabelPlacement, SeriesStyle, TimeSeriesChart,
        band_fractions, default_format, drag_selection, finite_runs, index_x, label_top,
        nearest_index, normalize_selection, tick_fraction, tooltip_left, value_at, value_y,
        x_fraction,
    };

    const NAN: f64 = f64::NAN;

    #[test]
    fn points_are_spread_evenly_over_the_plot() {
        assert_eq!(index_x(0, 60, 944.0), 0.0);
        assert_eq!(index_x(59, 60, 944.0), 944.0);
        assert_eq!(index_x(30, 61, 600.0), 300.0);
        // A single point goes in the middle, and an index past the end sticks to the last point.
        assert_eq!(index_x(0, 1, 100.0), 50.0);
        assert_eq!(x_fraction(70, 60), 1.0);
    }

    #[test]
    fn values_map_to_y_from_the_bottom_and_are_clamped() {
        assert_eq!(value_y(0.0, 100.0, 188.0), 188.0);
        assert_eq!(value_y(100.0, 100.0, 188.0), 0.0);
        assert_eq!(value_y(50.0, 100.0, 188.0), 94.0);
        // Above the maximum the line stays on the top edge, below zero on the bottom edge.
        assert_eq!(value_y(250.0, 100.0, 188.0), 0.0);
        assert_eq!(value_y(-5.0, 100.0, 188.0), 188.0);
        // A scale without a maximum puts everything on the bottom.
        assert_eq!(value_y(5.0, 0.0, 188.0), 188.0);
        assert!(value_y(NAN, 100.0, 188.0).is_nan());
    }

    #[test]
    fn labels_sit_beside_their_point_or_on_the_base() {
        // An 18 px box 5 px from a point at y = 100 of a 200 px plot.
        assert_eq!(label_top(LabelPlacement::BeforeBelow, 100.0, 200.0), 105.0);
        assert_eq!(label_top(LabelPlacement::BeforeAbove, 100.0, 200.0), 77.0);
        assert_eq!(label_top(LabelPlacement::AfterAbove, 100.0, 200.0), 77.0);
        assert_eq!(label_top(LabelPlacement::AfterOnBase, 100.0, 200.0), 177.0);
    }

    #[test]
    fn x_snaps_to_the_nearest_point() {
        assert_eq!(nearest_index(0.0, 590.0, 60), 0);
        assert_eq!(nearest_index(589.9, 590.0, 60), 59);
        // Points are 10 px apart: 14 is closer to the second one, 16 to the third.
        assert_eq!(nearest_index(14.0, 590.0, 60), 1);
        assert_eq!(nearest_index(16.0, 590.0, 60), 2);
        // Outside the plot it sticks to the ends.
        assert_eq!(nearest_index(-30.0, 590.0, 60), 0);
        assert_eq!(nearest_index(900.0, 590.0, 60), 59);
        // Degenerate input never panics.
        assert_eq!(nearest_index(10.0, 0.0, 60), 0);
        assert_eq!(nearest_index(10.0, 100.0, 1), 0);
        assert_eq!(nearest_index(10.0, 100.0, 0), 0);
        assert_eq!(nearest_index(f32::NAN, 100.0, 60), 0);
    }

    #[test]
    fn index_to_x_and_back_round_trips() {
        for index in 0..60 {
            let x = index_x(index, 60, 944.0);
            assert_eq!(nearest_index(x, 944.0, 60), index);
        }
    }

    #[test]
    fn ticks_are_spread_from_the_bottom_to_the_top() {
        assert_eq!(tick_fraction(0, 5), 0.0);
        assert_eq!(tick_fraction(2, 5), 0.5);
        assert_eq!(tick_fraction(4, 5), 1.0);
        assert_eq!(tick_fraction(0, 1), 0.0);
        assert_eq!(tick_fraction(0, 0), 0.0);
    }

    #[test]
    fn nan_values_split_a_series_into_runs() {
        assert_eq!(finite_runs(&[1.0, 2.0, 3.0], 3), vec![0..3]);
        assert_eq!(finite_runs(&[1.0, NAN, 3.0, 4.0, NAN], 5), vec![0..1, 2..4]);
        assert_eq!(finite_runs(&[NAN, NAN], 2), Vec::<Range<usize>>::new());
        assert_eq!(finite_runs(&[1.0, f64::INFINITY, 2.0], 3), vec![0..1, 2..3]);
        // Points past the end of the data are gaps, extra values are ignored.
        assert_eq!(finite_runs(&[1.0, 2.0], 4), vec![0..2]);
        assert_eq!(finite_runs(&[1.0, 2.0, 3.0], 2), vec![0..2]);
        assert_eq!(finite_runs(&[], 0), Vec::<Range<usize>>::new());
    }

    #[test]
    fn a_drag_selects_its_ends_in_order() {
        assert_eq!(normalize_selection(4, 9), (4, 9));
        assert_eq!(normalize_selection(9, 4), (4, 9));
        assert_eq!(drag_selection(4, 9), Some((4, 9)));
        assert_eq!(drag_selection(9, 4), Some((4, 9)));
        // One point apart is enough, no movement is a click that clears.
        assert_eq!(drag_selection(4, 5), Some((4, 5)));
        assert_eq!(drag_selection(4, 4), None);
    }

    #[test]
    fn the_band_covers_the_selected_points() {
        assert_eq!(band_fractions(0, 59, 60), (0.0, 1.0));
        assert_eq!(band_fractions(30, 0, 61), (0.0, 0.5));
        assert_eq!(band_fractions(10, 10, 11), (1.0, 1.0));
    }

    #[test]
    fn the_tooltip_sits_right_of_the_crosshair_and_flips_near_the_edge() {
        // 944 wide plot: the tooltip flips once the crosshair is beyond 944 - 210 = 734.
        assert_eq!(tooltip_left(100.0, 944.0, 184.0), 112.0);
        assert_eq!(tooltip_left(734.0, 944.0, 184.0), 746.0);
        assert_eq!(tooltip_left(735.0, 944.0, 184.0), 539.0);
        assert_eq!(tooltip_left(944.0, 944.0, 184.0), 748.0);
        // In a plot too narrow for it, it stays inside on the left.
        assert_eq!(tooltip_left(50.0, 200.0, 184.0), 0.0);
    }

    #[test]
    fn values_default_to_integers_or_two_decimals() {
        assert_eq!(default_format(2100.0).as_ref(), "2100");
        assert_eq!(default_format(12.345).as_ref(), "12.35");
        assert_eq!(default_format(0.5).as_ref(), "0.50");
    }

    #[test]
    fn missing_values_are_gaps() {
        assert_eq!(value_at(&[1.0, 2.0], 1), 2.0);
        assert!(value_at(&[1.0, 2.0], 2).is_nan());
    }

    #[test]
    fn moving_within_a_point_changes_nothing() {
        let mut state = ChartState::default();
        assert!(state.pointer_moved(3));
        assert!(!state.pointer_moved(3));
        assert!(!state.pointer_moved(3));
        assert!(state.pointer_moved(4));
        assert_eq!(state.hover, Some(4));
    }

    #[test]
    fn dragging_follows_the_pointer_and_releasing_commits() {
        let mut state = ChartState::default();
        state.pointer_moved(10);
        state.pressed(10);
        assert_eq!(state.drag, Some((10, 10)));
        assert!(state.pointer_moved(14));
        assert!(!state.pointer_moved(14));
        assert_eq!(state.drag, Some((10, 14)));
        assert_eq!(state.finish_drag(), Some(Some((10, 14))));
        assert_eq!(state.drag, None);
        // A second release has nothing to commit.
        assert_eq!(state.finish_drag(), None);
    }

    #[test]
    fn dragging_backwards_selects_in_order() {
        let mut state = ChartState::default();
        state.pressed(20);
        state.pointer_moved(12);
        assert_eq!(state.finish_drag(), Some(Some((12, 20))));
    }

    #[test]
    fn a_click_without_a_drag_clears_the_selection() {
        let mut state = ChartState::default();
        state.pressed(7);
        assert_eq!(state.finish_drag(), Some(None));
    }

    #[test]
    fn leaving_the_plot_commits_a_drag_and_clears_the_hover() {
        let mut state = ChartState::default();
        state.pressed(5);
        state.pointer_moved(9);
        assert_eq!(state.left(), (true, Some(Some((5, 9)))));
        assert_eq!(state.hover, None);
        assert_eq!(state.drag, None);
        // Leaving again, with nothing hovered, changes nothing.
        assert_eq!(state.left(), (false, None));
    }

    #[test]
    fn leaving_without_a_drag_only_hides_the_hover() {
        let mut state = ChartState::default();
        state.pointer_moved(5);
        assert_eq!(state.left(), (true, None));
    }

    #[test]
    fn the_pointer_needs_a_painted_plot() {
        let mut state = ChartState::default();
        assert_eq!(state.index_at(px(10.0), 60), None);
        state.plot = Bounds {
            origin: point(px(100.0), px(20.0)),
            size: size(px(590.0), px(188.0)),
        };
        assert_eq!(state.index_at(px(100.0), 60), Some(0));
        assert_eq!(state.index_at(px(114.0), 60), Some(1));
        assert_eq!(state.index_at(px(700.0), 60), Some(59));
    }

    #[test]
    fn a_series_is_a_solid_line_on_the_left_axis_by_default() {
        let series = ChartSeries::new("Req/s", Hsla::default(), vec![1.0, 2.0]);
        assert_eq!(series.side, AxisSide::Left);
        assert_eq!(series.style, SeriesStyle::line(1.5));
        assert!(series.dot);
        assert_eq!((series.format)(2.0).as_ref(), "2");
    }

    #[test]
    fn an_axis_falls_back_to_the_other_one_and_then_to_one() {
        let chart = TimeSeriesChart::new("chart", 10);
        assert_eq!(chart.axis_max(AxisSide::Left), 1.0);
        let chart = chart.right_axis(ChartAxis::new(100.0, ["0", "100"]));
        assert_eq!(chart.axis_max(AxisSide::Left), 100.0);
        let chart = chart.left_axis(ChartAxis::new(2400.0, ["0", "2,400"]));
        assert_eq!(chart.axis_max(AxisSide::Left), 2400.0);
        assert_eq!(chart.axis_max(AxisSide::Right), 100.0);
    }
}
