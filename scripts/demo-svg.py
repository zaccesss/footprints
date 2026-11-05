#!/usr/bin/env python3
"""Turn a captured terminal transcript into static light and dark terminal window SVGs.

    scripts/demo-capture.sh > /tmp/demo.ansi
    scripts/demo-svg.py /tmp/demo.ansi assets/demo/demo

writes assets/demo/demo-light.svg, assets/demo/demo-dark.svg and assets/demo/demo.txt, the same
transcript as plain text. A line that starts with "$ " is drawn as a shell prompt and a command.
Every other line is output, drawn in the colours its ANSI escape codes ask for. Only the
standard library is used.
"""

import re
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
TITLE_BAR = 40
PAD_TOP = 18
PAD_BOTTOM = 22
RADIUS = 10
FONT = "ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, 'Liberation Mono', monospace"
PROMPT_DIR = "~/demo"
PROMPT_MARK = "❯"
TITLE = "me@demo: ~/demo"

# colours from the Zacess themes palette 0.12.0: Zacess Deep Contrast Dark and Zacess Light.
# The ANSI roles use the theme's terminal colours where they reach 7:1 on its background and
# the theme's own semantic colour for the same meaning where they do not (Zacess Light's
# terminal green, red, yellow and cyan are tuned for 4.5:1)
THEMES = {
    "dark": {
        "bg": "#000000",
        "fg": "#ffffff",
        "muted": "#b3b3b3",
        "accent": "#00ffff",
        "frame": "#00ffff",
        "bar": "#0a0a0a",
        "bar_rule": "#262626",
        "title": "#b3b3b3",
        "red": "#ff5f5f",
        "green": "#00cd00",
        "yellow": "#cdcd00",
        "cyan": "#00cdcd",
        "buttons": ("#ff5f5f", "#ffd370", "#00cd00"),
    },
    "light": {
        "bg": "#ffffff",
        "fg": "#111111",
        "muted": "#424242",
        "accent": "#0049a3",
        "frame": "#707070",
        "bar": "#f4f4f4",
        "bar_rule": "#d6dde8",
        "title": "#424242",
        # error, syntax.added, warning and info from the Zacess Light palette
        "red": "#810005",
        "green": "#005b14",
        "yellow": "#843800",
        "cyan": "#003a84",
        "buttons": ("#ca010e", "#9d6902", "#01751c"),
    },
}

# every text colour must reach WCAG AAA (7:1) against the terminal background. The title must
# reach it against the title bar
MIN_CONTRAST = 7.0
TEXT_ROLES = ("fg", "muted", "accent", "red", "green", "yellow", "cyan")

# the SGR foreground codes footprints uses, by role
SGR_COLOURS = {31: "red", 32: "green", 33: "yellow", 36: "cyan"}
ANSI = re.compile(r"\x1b\[([0-9;]*)m")


def keep_spaces(text):
    # browsers showing an SVG as an image collapse runs of spaces whatever xml:space says, so each
    # space becomes a no-break space, the same width in a monospace font, to keep the columns
    return escape(text).replace(" ", " ")


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
        pairs = [(role, theme[role], theme["bg"]) for role in TEXT_ROLES]
        pairs.append(("title", theme["title"], theme["bar"]))
        for role, colour, background in pairs:
            ratio = contrast(colour, background)
            if ratio < MIN_CONTRAST:
                sys.exit(f"{name} {role} is {ratio:.2f}:1, below {MIN_CONTRAST}:1")


def styled(line):
    """Split one line with ANSI codes into (character, role, bold) triples."""
    out = []
    role, bold = "fg", False
    pos = 0
    for m in ANSI.finditer(line):
        out += [(c, role, bold) for c in line[pos : m.start()]]
        pos = m.end()
        for code in [int(x) for x in m.group(1).split(";") if x] or [0]:
            if code == 0:
                role, bold = "fg", False
            elif code == 1:
                bold = True
            elif code == 22:
                bold = False
            elif code == 39:
                role = "fg"
            elif code in SGR_COLOURS:
                role = SGR_COLOURS[code]
    out += [(c, role, bold) for c in line[pos:]]
    return out


def plain(text):
    return ANSI.sub("", text)


def rows(transcript):
    """Yield one list of (text, role, bold) runs per drawn row, wrapping long lines."""
    for raw in transcript.splitlines():
        line = raw.rstrip()
        if line.startswith("$ "):
            cells = [(c, "accent", True) for c in f"{PROMPT_DIR} {PROMPT_MARK} "]
            cells += [(c, "fg", True) for c in line[2:]]
        else:
            cells = styled(line)
            # footprints' own notes on standard error are secondary to the command output
            if plain(line).startswith("footprints: "):
                cells = [(c, "muted", b) for c, _, b in cells]
        text = "".join(c for c, _, _ in cells)
        pieces = textwrap.wrap(
            text,
            COLUMNS,
            drop_whitespace=False,
            replace_whitespace=False,
            break_on_hyphens=False,
            expand_tabs=False,
        ) or [""]
        assert "".join(pieces) == text, "wrapping must keep every character"
        start = 0
        for piece in pieces:
            runs = []
            for c, role, bold in cells[start : start + len(piece)]:
                if runs and runs[-1][1] == role and runs[-1][2] == bold:
                    runs[-1][0] += c
                else:
                    runs.append([c, role, bold])
            start += len(piece)
            yield [tuple(r) for r in runs]


def render(transcript, theme, title):
    drawn = list(rows(transcript))
    width = round(PAD_X * 2 + COLUMNS * CHAR_WIDTH)
    height = TITLE_BAR + PAD_TOP + len(drawn) * LINE_HEIGHT + PAD_BOTTOM
    out = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc">',
        f'<title id="title">{escape(title)}</title>',
        '<desc id="desc">' + escape(plain(transcript)) + "</desc>",
        # the window: a rounded frame, then a title bar clipped to the frame's top corners
        f'<clipPath id="window"><rect x="1" y="1" width="{width - 2}" height="{height - 2}" '
        f'rx="{RADIUS}"/></clipPath>',
        f'<rect x="1" y="1" width="{width - 2}" height="{height - 2}" rx="{RADIUS}" '
        f'fill="{theme["bg"]}"/>',
        f'<rect x="0" y="0" width="{width}" height="{TITLE_BAR}" fill="{theme["bar"]}" '
        'clip-path="url(#window)"/>',
        f'<line x1="1" y1="{TITLE_BAR}" x2="{width - 1}" y2="{TITLE_BAR}" '
        f'stroke="{theme["bar_rule"]}" stroke-width="1"/>',
        f'<rect x="1" y="1" width="{width - 2}" height="{height - 2}" rx="{RADIUS}" '
        f'fill="none" stroke="{theme["frame"]}" stroke-width="1.5"/>',
    ]
    for i, colour in enumerate(theme["buttons"]):
        out.append(
            f'<circle cx="{PAD_X + 6 + i * 20}" cy="{TITLE_BAR / 2}" r="6" fill="{colour}"/>'
        )
    out += [
        f'<text x="{width / 2}" y="{TITLE_BAR / 2 + 5}" text-anchor="middle" '
        f'font-family="{FONT}" font-size="13" fill="{theme["title"]}">{escape(TITLE)}</text>',
        f'<g font-family="{FONT}" font-size="{FONT_SIZE}" fill="{theme["fg"]}">',
    ]
    y = TITLE_BAR + PAD_TOP + FONT_SIZE
    for runs in drawn:
        spans = []
        for text, role, bold in runs:
            attrs = "" if role == "fg" else f' fill="{theme[role]}"'
            if bold:
                attrs += ' font-weight="bold"'
            spans.append(f"<tspan{attrs}>{keep_spaces(text)}</tspan>" if attrs else keep_spaces(text))
        out.append(f'<text x="{PAD_X}" y="{y}">{"".join(spans)}</text>')
        y += LINE_HEIGHT
    out += ["</g>", "</svg>", ""]
    return "\n".join(out)


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: demo-svg.py <transcript> <output prefix>")
    check_contrast()
    with open(sys.argv[1], encoding="utf-8") as handle:
        transcript = handle.read()
    title = (
        "A Footprints run in a terminal: an agent script edits config.toml and creates NOTES.md, "
        "then log, diff and undo"
    )
    for name, theme in THEMES.items():
        path = f"{sys.argv[2]}-{name}.svg"
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(render(transcript, theme, title))
        print(f"wrote {path}")
    path = f"{sys.argv[2]}.txt"
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(plain(transcript))
    print(f"wrote {path}")


if __name__ == "__main__":
    main()
