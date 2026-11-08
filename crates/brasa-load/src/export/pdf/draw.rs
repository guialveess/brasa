//! Drawing operations for the PDF, and replaying them with `krilla`.

use krilla::Document;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, LineCap, LineJoin, Stroke, StrokeDash};
use krilla::surface::Surface;
use krilla::text::TextDirection;

use super::text::{Fonts, Style, clean};
use super::{PAGE_H, PAGE_W, PdfError};
use crate::export::report::Rgb;

/// A stroke: color, opacity, width and an optional dash (on, off).
#[derive(Debug, Clone, Copy)]
pub(super) struct Line {
    pub(super) color: Rgb,
    pub(super) alpha: f32,
    pub(super) width: f32,
    pub(super) dash: Option<(f32, f32)>,
}

impl Line {
    pub(super) fn solid(color: Rgb, width: f32) -> Self {
        Self {
            color,
            alpha: 1.0,
            width,
            dash: None,
        }
    }
}

/// A fill: color and opacity.
pub(super) type Paint = (Rgb, f32);

/// A shape to draw.
#[derive(Debug, Clone)]
pub(super) enum Shape {
    /// A rectangle with rounded corners of radius `r` (`0.0` for square ones).
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
    },
    /// A polyline, or a polygon when `closed`.
    Poly { pts: Vec<(f32, f32)>, closed: bool },
}

/// One drawing operation of a page. Coordinates are in points from the top left corner.
#[derive(Debug, Clone)]
pub(super) enum Op {
    Shape {
        shape: Shape,
        fill: Option<Paint>,
        stroke: Option<Line>,
    },
    Text {
        x: f32,
        y: f32,
        text: String,
        style: Style,
    },
}

/// Replays the pages with `krilla`.
pub(super) fn render(
    pages: &[Vec<Op>],
    fonts: &Fonts<'_>,
    title: &str,
) -> Result<Vec<u8>, PdfError> {
    let mut document = Document::new();
    document.set_metadata(
        Metadata::new()
            .title(clean(title))
            .creator("Brasa".to_string()),
    );
    for ops in pages {
        let settings = PageSettings::from_wh(PAGE_W, PAGE_H)
            .ok_or_else(|| PdfError::Write("invalid page size".into()))?;
        let mut page = document.start_page_with(settings);
        let mut surface = page.surface();
        for op in ops {
            draw(&mut surface, op, fonts);
        }
        surface.finish();
        page.finish();
    }
    document
        .finish()
        .map_err(|e| PdfError::Write(format!("{e:?}")))
}

/// An opacity from `0.0` to `1.0`.
pub(super) fn opacity(alpha: f32) -> NormalizedF32 {
    NormalizedF32::new(alpha.clamp(0.0, 1.0)).unwrap_or(NormalizedF32::ONE)
}

pub(super) fn paint(color: Rgb) -> krilla::paint::Paint {
    rgb::Color::new(color.0, color.1, color.2).into()
}

/// Draws one operation on the page.
pub(super) fn draw(surface: &mut Surface<'_>, op: &Op, fonts: &Fonts<'_>) {
    match op {
        Op::Shape {
            shape,
            fill,
            stroke,
        } => {
            let Some(path) = build_path(shape) else {
                return;
            };
            surface.set_fill(fill.map(|(color, alpha)| Fill {
                paint: paint(color),
                opacity: opacity(alpha),
                ..Fill::default()
            }));
            surface.set_stroke(stroke.map(|line| Stroke {
                paint: paint(line.color),
                width: line.width,
                opacity: opacity(line.alpha),
                line_cap: if line.dash.is_some() {
                    LineCap::Butt
                } else {
                    LineCap::Round
                },
                line_join: LineJoin::Round,
                dash: line.dash.map(|(on, off)| StrokeDash {
                    array: vec![on, off],
                    offset: 0.0,
                }),
                ..Stroke::default()
            }));
            surface.draw_path(&path);
        }
        Op::Text { x, y, text, style } => {
            surface.set_stroke(None);
            surface.set_fill(Some(Fill {
                paint: paint(style.color),
                ..Fill::default()
            }));
            surface.draw_text(
                Point::from_xy(*x, *y),
                fonts.face(style.weight).font.clone(),
                style.size,
                text,
                false,
                TextDirection::Auto,
            );
        }
    }
}

/// The path of a shape, or `None` for an empty one.
pub(super) fn build_path(shape: &Shape) -> Option<krilla::geom::Path> {
    let mut b = PathBuilder::new();
    match shape {
        Shape::Rect { x, y, w, h, r } => {
            let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
            if r < 0.05 {
                b.push_rect(Rect::from_xywh(*x, *y, *w, *h)?);
            } else {
                // A circle quadrant as a cubic curve.
                let k = r * 0.552_285;
                let (x, y, w, h) = (*x, *y, *w, *h);
                b.move_to(x + r, y);
                b.line_to(x + w - r, y);
                b.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
                b.line_to(x + w, y + h - r);
                b.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
                b.line_to(x + r, y + h);
                b.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
                b.line_to(x, y + r);
                b.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
                b.close();
            }
        }
        Shape::Poly { pts, closed } => {
            let (first, rest) = pts.split_first()?;
            b.move_to(first.0, first.1);
            for p in rest {
                b.line_to(p.0, p.1);
            }
            if *closed {
                b.close();
            }
        }
    }
    b.finish()
}
