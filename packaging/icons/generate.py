#!/usr/bin/env python3
"""Generates Brasa's app icon: nested rounded squares in the app's own `ember_rings` gradient
(crates/brasa-app/src/theme/palette.rs, dark mode stops), drifting toward the upper right as they
nest inward so the stack reads as one organic blob instead of a flat concentric target.

Writes `brasa-icon.svg` (the source of truth, edit the constants below and re-run to tweak it)
and every PNG size `packaging/icons/png/` and the macOS/Windows packaging scripts expect.

Requires `rsvg-convert` (`brew install librsvg`) on PATH.

Usage: python3 packaging/icons/generate.py
"""

import pathlib
import subprocess
import sys

# ember_rings' dark-mode stops, outer (darkest) to inner (palest) — already shaped like a ring
# gradient, so reusing them keeps the icon and the in-app brand mark (views/welcome.rs's
# `render_ember_mark`) visually the same family without hand-picking new colors.
STOPS = ["#2a140c", "#5c2210", "#a83a14", "#fd572a", "#ffd9ad"]

RING_COUNT = 9
CANVAS = 1024
BACKGROUND = "#140a07"  # near-black, warm-tinted, not flat black
OUTER_MARGIN = 120
RING_STEP = 50
OUTER_RADIUS = 190
# More inset from the bottom-left than the top-right each ring, so the stack drifts toward one
# corner instead of staying perfectly concentric.
INSET_LEFT, INSET_RIGHT, INSET_TOP, INSET_BOTTOM = 0.62, 0.38, 0.40, 0.60
# How quickly the color ramp approaches its palest stop: >1 keeps most rings darker/mid-toned and
# only lightens sharply at the innermost one or two, instead of washing out half the icon pale.
COLOR_RAMP_POWER = 1.6

SIZES = (16, 24, 32, 48, 64, 128, 256, 512, 1024)


def _hex_to_rgb(value: str) -> tuple[int, int, int]:
    value = value.lstrip("#")
    return tuple(int(value[i : i + 2], 16) for i in (0, 2, 4))


def _rgb_to_hex(rgb: tuple[float, float, float]) -> str:
    return "#%02x%02x%02x" % tuple(max(0, min(255, round(c))) for c in rgb)


def _mix(c1: str, c2: str, t: float) -> str:
    r1, g1, b1 = _hex_to_rgb(c1)
    r2, g2, b2 = _hex_to_rgb(c2)
    return _rgb_to_hex((r1 + (r2 - r1) * t, g1 + (g2 - g1) * t, b1 + (b2 - b1) * t))


def _ring_color(index: int, count: int) -> str:
    t = (index / (count - 1)) ** COLOR_RAMP_POWER
    segments = len(STOPS) - 1
    position = t * segments
    segment = min(int(position), segments - 1)
    return _mix(STOPS[segment], STOPS[segment + 1], position - segment)


def _rings():
    x0 = y0 = OUTER_MARGIN
    x1 = y1 = CANVAS - OUTER_MARGIN
    for i in range(RING_COUNT):
        x0i = x0 + i * RING_STEP * INSET_LEFT
        x1i = x1 - i * RING_STEP * INSET_RIGHT
        y0i = y0 + i * RING_STEP * INSET_TOP
        y1i = y1 - i * RING_STEP * INSET_BOTTOM
        radius = max(OUTER_RADIUS - i * RING_STEP * 0.5, 30)
        yield (x0i, y0i, x1i - x0i, y1i - y0i, radius, _ring_color(i, RING_COUNT))


def build_svg() -> str:
    body = "".join(
        f'<rect x="{x:.1f}" y="{y:.1f}" width="{w:.1f}" height="{h:.1f}" '
        f'rx="{rx:.1f}" ry="{rx:.1f}" fill="{color}"/>'
        for (x, y, w, h, rx, color) in _rings()
    )
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{CANVAS}" height="{CANVAS}" '
        f'viewBox="0 0 {CANVAS} {CANVAS}">'
        f'<rect width="{CANVAS}" height="{CANVAS}" fill="{BACKGROUND}"/>'
        f"{body}</svg>"
    )


def main() -> None:
    icons_dir = pathlib.Path(__file__).parent
    svg_path = icons_dir / "brasa-icon.svg"
    svg_path.write_text(build_svg())

    png_dir = icons_dir / "png"
    png_dir.mkdir(exist_ok=True)
    for size in SIZES:
        out = png_dir / f"brasa-{size}.png"
        subprocess.run(
            ["rsvg-convert", "-w", str(size), "-h", str(size), str(svg_path), "-o", str(out)],
            check=True,
        )
        print(f"wrote {out}")


if __name__ == "__main__":
    sys.exit(main())
