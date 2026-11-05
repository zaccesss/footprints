#!/usr/bin/env python3
"""Turn a captured terminal transcript into static light and dark SVG images.

    scripts/demo-capture.sh > demo.txt
    scripts/demo-svg.py demo.txt assets/demo/demo

writes assets/demo/demo-light.svg and assets/demo/demo-dark.svg. A line that starts with "$ "
is drawn as a prompt and a command, diff lines starting with + or - get the added and removed
colours and every other line is plain output. Only the standard library is used.
"""

import sys
import textwrap
from xml.sax.saxutils import escape

# a terminal soft-wraps long lines at its width; wrapping here keeps the image narrow enough to
# stay readable once GitHub scales it to the page
COLUMNS = 106
FONT_SIZE = 15
LINE_HEIGHT = 22
# monospace advance is close to 0.6 em in every font of the stack below
CHAR_WIDTH = FONT_SIZE * 0.6
PAD_X = 24
PAD_TOP = 52
PAD_BOTTOM = 24
FONT = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace"

THEMES = {
    "light": {
        "bg": "#ffffff",
        "border": "#57606a",
        "text": "#1f2328",
        "prompt": "#0a3069",
        "title": "#24292f",
        "added": "#0f5323",
        "removed": "#82071e",
    },
    "dark": {
        "bg": "#0d1117",
        "border": "#8b949e",
        "text": "#e6edf3",
        "prompt": "#a5d6ff",
        "title": "#e6edf3",
        "added": "#aff5b4",
        "removed": "#ffc1ba",
    },
}

# every text colour must reach WCAG AAA (7:1) against its background
MIN_CONTRAST = 7.0



def keep_spaces(text):
    # browsers showing an SVG as an image collapse runs of spaces whatever xml:space says, so each
    # space becomes a no-break space, the same width in a monospace font, to keep the columns
    return escape(text).replace(" ", "\u00a0")

def luminance(hex_colour):
    channels = []
    for i in (1, 3, 5):
        c = int(hex_colour[i : i + 2], 16) / 255
        channels.append(c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4)
    r, g, b = channels
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


def check_contrast():
    for name, theme in THEMES.items():
        for role in ("text", "prompt", "title", "added", "removed"):
            ratio = contrast(theme[role], theme["bg"])
            if ratio < MIN_CONTRAST:
                sys.exit(f"{name} {role} is {ratio:.2f}:1, below {MIN_CONTRAST}:1")


def classify(line, in_diff):
    if line.startswith("$ "):
        return "prompt"
    if in_diff and line.startswith("+") and not line.startswith("+++"):
        return "added"
    if in_diff and line.startswith("-") and not line.startswith("---"):
        return "removed"
    return "text"


def rows(transcript):
    """Yield (role, text) pairs, one per drawn row, wrapping long lines."""
    in_diff = False
    for raw in transcript.splitlines():
        line = raw.rstrip()
        if line.startswith("$ "):
            in_diff = line.startswith("$ footprints diff")
        role = classify(line, in_diff)
        pieces = textwrap.wrap(
            line,
            COLUMNS,
            drop_whitespace=False,
            replace_whitespace=False,
            break_on_hyphens=False,
        ) or [""]
        for piece in pieces:
            yield role, piece


def render(transcript, theme, title):
    drawn = list(rows(transcript))
    width = round(PAD_X * 2 + COLUMNS * CHAR_WIDTH)
    height = PAD_TOP + len(drawn) * LINE_HEIGHT + PAD_BOTTOM
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc">',
        f'<title id="title">{escape(title)}</title>',
        '<desc id="desc">' + escape(transcript) + "</desc>",
        f'<rect x="1" y="1" width="{width - 2}" height="{height - 2}" rx="8" '
        f'fill="{theme["bg"]}" stroke="{theme["border"]}" stroke-width="1"/>',
        f'<line x1="1" y1="36" x2="{width - 1}" y2="36" stroke="{theme["border"]}" stroke-width="1"/>',
        f'<text x="{PAD_X}" y="24" font-family="{FONT}" font-size="13" '
        f'fill="{theme["title"]}">~/demo</text>',
        f'<g font-family="{FONT}" font-size="{FONT_SIZE}">',
    ]
    y = PAD_TOP + FONT_SIZE
    for role, text in drawn:
        if role == "prompt":
            out.append(
                f'<text x="{PAD_X}" y="{y}" fill="{theme["prompt"]}" font-weight="bold">'
                f'{keep_spaces(text)}</text>'
            )
        else:
            out.append(f'<text x="{PAD_X}" y="{y}" fill="{theme[role]}">{keep_spaces(text)}</text>')
        y += LINE_HEIGHT
    out += ["</g>", "</svg>", ""]
    return "\n".join(out)


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: demo-svg.py <transcript> <output prefix>")
    check_contrast()
    with open(sys.argv[1], encoding="utf-8") as handle:
        transcript = handle.read()
    title = "A Footprints run: an agent script edits config.toml and creates NOTES.md, then log, diff and undo"
    for name, theme in THEMES.items():
        path = f"{sys.argv[2]}-{name}.svg"
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(render(transcript, theme, title))
        print(f"wrote {path}")


if __name__ == "__main__":
    main()
