#!/usr/bin/env python3
"""Draws the z-overlay icons.

The app icon is a rounded tile with two stacked cards inside corner brackets; the tray
icons are separate pixel-aligned glyphs on a transparent background, one for a
light taskbar and one for a dark one.

Usage: python3 tools/make-icons.py [output-dir]
"""

import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter

SS = 8  # supersampling factor for the drawn (non pixel-aligned) sizes

TILE_TOP = (74, 132, 240)
TILE_BOTTOM = (39, 31, 99)
TILE_BLOOM = (126, 96, 235)
TILE_FLAT = (58, 92, 208)
BRACKET = (125, 214, 255)
CARD = (255, 255, 255)
SHADOW = (8, 12, 40)

ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]

# Two stacked cards in a unit box: a translucent one behind, a solid one in
# front carrying two short text lines. (left, top, right, bottom)
BACK_CARD = (0.00, 0.00, 0.62, 0.50)
FRONT_CARD = (0.30, 0.34, 1.00, 0.86)
TEXT_LINES = ((0.40, 0.49, 0.86, 0.565), (0.40, 0.64, 0.72, 0.715))

# 16x16 glyphs, drawn pixel by pixel. "#" is the front card, "+" the back
# card's outline and "-" a text line cut out of the front card.
SMALL_CARDS = [
    "+++++.....",
    "+...+.....",
    "+..#######",
    "+..#-----#",
    "+++#######",
    "...#---###",
    "...#######",
]
SMALL_CARDS = [row.ljust(10, ".") for row in SMALL_CARDS]


def cards_origin(size):
    rows, columns = len(SMALL_CARDS), len(SMALL_CARDS[0])
    return (size - columns) // 2, (size - rows) // 2


def gradient(size):
    """Diagonal blue ramp lifted by a violet bloom in the lower left."""
    image = Image.new("RGB", (size, size))
    pixels = image.load()
    span = 2 * (size - 1)
    reach = size * 0.72
    for y in range(size):
        for x in range(size):
            t = (x + y) / span
            base = [
                top + (bottom - top) * t
                for top, bottom in zip(TILE_TOP, TILE_BOTTOM)
            ]
            dx, dy = x - size * 0.10, y - size * 0.98
            distance = (dx * dx + dy * dy) ** 0.5
            bloom = max(0.0, 1 - distance / reach) ** 2 * 0.40
            pixels[x, y] = tuple(
                round(value + (glow - value) * bloom)
                for value, glow in zip(base, TILE_BLOOM)
            )
    return image


def tile_mask(size, radius):
    mask = Image.new("L", (size * SS, size * SS), 0)
    draw = ImageDraw.Draw(mask)
    inset = size * SS * 0.03
    draw.rounded_rectangle(
        (inset, inset, size * SS - inset - 1, size * SS - inset - 1),
        radius=radius * SS,
        fill=255,
    )
    return mask.resize((size, size), Image.LANCZOS)


def corner_brackets(canvas):
    """Four L shapes cut from a rounded frame, so their corners echo the tile."""
    inset = canvas * 0.155
    radius = canvas * 0.11
    thickness = round(canvas * 0.05)
    reach = radius + canvas * 0.115
    near, far = inset, canvas - inset

    frame = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    ImageDraw.Draw(frame).rounded_rectangle(
        (near, near, far, far), radius=radius, outline=BRACKET, width=thickness
    )

    corners = Image.new("L", (canvas, canvas), 0)
    draw = ImageDraw.Draw(corners)
    for left, top in (
        (near, near),
        (far - reach, near),
        (near, far - reach),
        (far - reach, far - reach),
    ):
        draw.rectangle(
            (
                left - thickness,
                top - thickness,
                left + reach + thickness,
                top + reach + thickness,
            ),
            fill=255,
        )
    # Trim the strokes back to the corners; the straight runs stop square.
    draw.rectangle((near + reach, 0, far - reach, canvas), fill=0)
    draw.rectangle((0, near + reach, canvas, far - reach), fill=0)

    frame.putalpha(ImageChops.multiply(frame.getchannel("A"), corners))
    return frame


def cards_layer(canvas):
    """Back card at 50% white, front card solid white with text lines cut out."""
    span = canvas * 0.50
    left, top = canvas * 0.25, canvas * 0.27
    radius = span * 0.09

    def box(unit):
        x0, y0, x1, y1 = unit
        return (left + x0 * span, top + y0 * span, left + x1 * span, top + y1 * span)

    back = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    ImageDraw.Draw(back).rounded_rectangle(box(BACK_CARD), radius=radius, fill=(*CARD, 125))

    front = Image.new("L", (canvas, canvas), 0)
    draw = ImageDraw.Draw(front)
    draw.rounded_rectangle(box(FRONT_CARD), radius=radius, fill=255)
    for line in TEXT_LINES:
        draw.rounded_rectangle(box(line), radius=span * 0.035, fill=0)
    layer = Image.new("RGBA", (canvas, canvas), (*CARD, 0))
    layer.putalpha(front)
    return back, layer


def draw_app_icon(size):
    """Rounded tile, corner brackets, and two cards floating above them."""
    mask = tile_mask(size, size * 0.22)
    tile = gradient(size)
    tile.putalpha(mask)

    # A soft highlight across the upper half gives the tile some depth.
    highlight = Image.new("L", (size, size), 0)
    pixels = highlight.load()
    for y in range(size):
        value = max(0, 1 - y / (size * 0.55))
        for x in range(size):
            pixels[x, y] = round(38 * value)
    tile.alpha_composite(
        Image.merge("RGBA", (*[Image.new("L", (size, size), 255)] * 3, highlight))
        .convert("RGBA"),
    )
    tile.putalpha(mask)

    canvas = size * SS
    if size >= 32:
        tile.alpha_composite(corner_brackets(canvas).resize((size, size), Image.LANCZOS))

    back, front = (layer.resize((size, size), Image.LANCZOS) for layer in cards_layer(canvas))
    tile.alpha_composite(back)

    # Two shadows under the front card alone: a tight one to seat it, a wide
    # one to lift it off the card behind.
    if size >= 40:
        silhouette = front.getchannel("A")
        for blur, drop, opacity in (
            (size * 0.016, size * 0.012, 150),
            (size * 0.055, size * 0.040, 95),
        ):
            shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
            shadow.paste((*SHADOW, opacity), (0, 0), silhouette)
            tile.alpha_composite(
                shadow.filter(ImageFilter.GaussianBlur(blur)), (0, round(drop))
            )

    tile.alpha_composite(front)
    return tile


def plot_cards(plot, size):
    left, top = cards_origin(size)
    for y, row in enumerate(SMALL_CARDS):
        for x, cell in enumerate(row):
            if cell in "#+":
                plot(left + x, top + y)


def draw_small_app_icon(size):
    """16 and 20 pixel icons: tile plus cards, no brackets, no gradient."""
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    radius = 3 if size >= 20 else 2
    draw.rounded_rectangle((0, 0, size - 1, size - 1), radius=radius, fill=TILE_FLAT)
    plot_cards(lambda x, y: draw.point((x, y), fill=CARD), size)
    return image


def draw_tray_icon(size, ink):
    """Pixel-aligned tray glyph: corner ticks plus cards, transparent behind."""
    scale = size // 16
    image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)

    def plot(x, y, width=1, height=1):
        draw.rectangle(
            (
                x * scale,
                y * scale,
                (x + width) * scale - 1,
                (y + height) * scale - 1,
            ),
            fill=ink,
        )

    for x, y, dx, dy in ((1, 1, 1, 1), (14, 1, -1, 1), (1, 14, 1, -1), (14, 14, -1, -1)):
        plot(min(x, x + dx * 2), y, 3, 1)
        plot(x, min(y, y + dy * 2), 1, 3)

    plot_cards(plot, 16)
    return image


def app_icon(size):
    return draw_small_app_icon(size) if size < 24 else draw_app_icon(size)


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "assets")
    out.mkdir(parents=True, exist_ok=True)

    icons = {size: app_icon(size) for size in ICO_SIZES}
    icons[256].save(out / "z-overlay.ico", sizes=[(s, s) for s in ICO_SIZES],
                    append_images=[icons[s] for s in ICO_SIZES if s != 256])
    draw_app_icon(512).save(out / "z-overlay.png")

    for name, ink in (("light", (26, 26, 26, 255)), ("dark", (255, 255, 255, 255))):
        for size in (16, 32):
            draw_tray_icon(size, ink).save(out / f"tray-{name}-{size}.png")

    print(f"wrote icons to {out}")


if __name__ == "__main__":
    main()
