"""Page 100: the index. What is on now and next, and a directory of the other pages.

Layout (row numbers are display rows; ttxd sends the header row itself)::

     1-5  banner on black: the logo mark and FRIKANALEN in mosaic graphics, web address,
          TEKST-TV, tagline
     6    three bars in the logo's colours, centred under FRIKANALEN
     8-11 NÅ PÅ FRIKANALEN tab (DIREKTE flashes when live), start time and title
          (up to two lines), organisation
    13-16 DERETTER: the same for the next programme
    18-22 SIDER tab and a directory of up to 4 pages: "Sendeplan ........200"
    23-24 colour key buttons: red, green, yellow and cyan bars with the first four
          directory pages under them, linked to the remote's keys (fastext)

On a Level 2.5 decoder (see level25.py) the page gets Frikanalen's colours, and the
real logo, drawn in DRCS pixels, replaces the mosaic one on rows 1-5. Nothing on the
page is double height: a Level 1 double-height row hides the row below it on every
decoder.
"""

import functools
import os
from dataclasses import dataclass, field

from . import level25
from .fastext import x27_0
from .markup import COLS, check, fit, leader, plain, spread, wrap
from .mosaic import Canvas, lettering
from .schedule import OSLO, Programme

DIRECTORY_ROWS = 4
ASSETS = os.path.join(os.path.dirname(__file__), "assets")
BANNER_ROWS = 5
BAND = "  "  # the banner canvas starts at column 2, on black
CANVAS_COL = 2
# The logo mark, 12x15 sixels, in three bands of colour like the real one.
MARK = [
    ".........###",
    "......######",
    "..#########.",
    "##########..",
    "....####....",
    "............",
    ".###########",
    "...#######..",
    ".....###....",
    "..#####.....",
    "...####.....",
    "...####.....",
    "....###.....",
    "....##......",
    ".....#......",
]
MARK_BANDS = [(0, 6, "red"), (6, 9, "yellow"), (9, 15, "green")]
MARK_X = 2  # sixels from the canvas edge: canvas columns 1-6
WORD_X, WORD_Y = 16, 3  # FRIKANALEN: canvas columns 8-35, rows 2-4
# Level 2.5: the real logo over the mosaic one. Mark on rows 1-5 from column 3,
# wordmark on rows 2-4 from column 10.
LOGO_ROW, LOGO_COL = 1, 3
WORD_COL, WORD_TOP = 10, 2  # wordmark: column, and pixels below the top of row 2
# Colour table 3: the logo's gradient steps, beyond the brand colours in table 2.
LOGO_COLOURS = [
    level25.hex_rgb(c)
    for c in (
        "#ce4948",
        "#b23f4a",
        "#8e324c",
        "#e78640",
        "#fec40f",
        "#449a4f",
        "#2c7f4d",
        "#0e5f4c",
    )
]
TITLE_WIDTH = COLS - 7  # attribute, HH:MM, attribute
TAGLINE_COL = WORD_X // 2 - 1  # its colour attribute; the text lines up with the lettering
# Slanted bars, 2 sixels tall: three under the lettering, and the colour key buttons.
STRIPE = (["red", "yellow", "green"], 11, 8)  # colours, first column, cells per bar
BUTTONS = (["red", "green", "yellow", "cyan"], 1, 9)
BUTTON_TEXT = {"red": "{red}", "green": "{green}", "yellow": "{yellow}", "cyan": "{cyan}"}


@dataclass
class Entry:
    page: str
    title: str
    # List the page even when ttxd is not sending it (e.g. subtitles on 888).
    always: bool = False


@dataclass
class IndexConfig:
    page: str = "100"
    tagline: str = "Sivilsamfunnets kanal"
    footer: str = "frikanalen.tv"  # web address, top left of the banner
    entries: list[Entry] = field(default_factory=list)
    # Level 2.5 colours and logo. The logo's pixels go on a hidden DRCS page; its
    # number needs tens digit 0 and a hex units digit so nobody can key it in.
    level25: bool = True
    drcs_page: str = "10F"


def banner(tagline: str, web: str) -> Canvas:
    """The banner's mosaic graphics and text, on a canvas from column 2."""
    c = Canvas(COLS - CANVAS_COL, BANNER_ROWS)
    for top, bottom, colour in MARK_BANDS:
        c.draw(MARK_X, top, MARK[top:bottom], colour)
    c.draw(WORD_X, WORD_Y, lettering("FRIKANALEN"), "white")
    c.text(0, TAGLINE_COL, "{white}" + fit(plain(web), c.cols - TAGLINE_COL - 12))
    c.text(0, c.cols - 10, "{cyan}TEKST-TV")
    c.text(BANNER_ROWS - 1, TAGLINE_COL, "{yellow}" + fit(plain(tagline), c.cols - TAGLINE_COL - 1))
    return c


def bars(colours: list[str], first_col: int, width: int) -> str:
    """A row of slanted bars (parallelograms), ``width`` cells each with one blank
    cell between them, the first starting at ``first_col``."""
    c = Canvas(COLS, 1)
    length = width * 2 - 2  # sixels per sixel row; each row is one sixel further left
    for n, colour in enumerate(colours):
        x = (first_col + n * (width + 1)) * 2
        c.draw(x, 0, ["  " + "#" * length, " " + "#" * length, "#" * length], colour)
    return c.markup()[0]


def buttons(entries: list[Entry]) -> list[str]:
    """Rows 23-24: the colour key bars, and under each the page its key goes to."""
    colours, _, width = BUTTONS
    labels = "".join(
        BUTTON_TEXT[colour] + fit(plain(e.title), width).ljust(width)
        for colour, e in zip(colours, entries)
    )
    return [bars(*BUTTONS), labels.rstrip()]


def fastext(cfg: IndexConfig, entries: list[Entry]) -> dict:
    """The X/27/0 packet linking the colour keys to the button pages."""
    pages = [e.page.upper() for e in entries[: len(BUTTONS[0])]]
    return {"packet": 27, "designation": 0, "raw": x27_0(cfg.page, pages, index=cfg.page)}


def _read(name: str) -> level25.Picture:
    return level25.read_ppm(os.path.join(ASSETS, name), level25.BRAND[0])


@functools.cache
def logo() -> level25.Graphic:
    """The real logo for Level 2.5, from rows 1-5, column 3: mark and wordmark."""
    mark, word = _read("mark.ppm"), _read("wordmark.ppm")
    left = (6 * level25.TILE_W - mark.width) // 2
    pic = level25.Picture.blank((WORD_COL - LOGO_COL) * level25.TILE_W + word.width, mark.height)
    pic.paste(mark, left, 0)
    pic.paste(word, (WORD_COL - LOGO_COL) * level25.TILE_W, level25.TILE_H + WORD_TOP)
    return level25.make_graphic(
        pic, LOGO_ROW, LOGO_COL, level25.BRAND + LOGO_COLOURS, level25.BRAND[0]
    )


def enhancements(cfg: IndexConfig) -> dict:
    """Level 2.5 data for page 100: palette, link to the DRCS page, and the logo."""
    # Hide the mosaic logo wherever the real one has no pixels.
    canvas = banner(cfg.tagline, cfg.footer)
    text = set()
    for row, col in canvas.texts:
        text |= {(row, c) for c in range(col, canvas.cols)}
    blank = {(LOGO_ROW + row, CANVAS_COL + col) for row, col in canvas.filled() - text}
    return {
        "x26": logo().x26(blank=blank),
        "packets": [
            {"packet": 27, "designation": 4, "triplets": level25.x27_drcs_link(cfg.drcs_page)},
            {"packet": 28, "designation": 0, "triplets": level25.x28_palette(LOGO_COLOURS)},
        ],
    }


def tab(label: str, right: str = "") -> str:
    """A section heading: a cyan tab with a slanted end on a blue band."""
    left = "{cyan}{newbg}{blue}" + label + " {gblue}{x:7a}{newbg}"
    return spread(left, right)


def _programme(label: str, p: Programme | None, flash: bool) -> list[str]:
    live = ""
    if p and p.live:
        live = "{red}{newbg}{yellow}" + ("{flash}" if flash else "") + "DIREKTE "
    heading = tab(label, live)
    if p is None:
        return [heading, "{white}Ingen program i sendeplanen", "", ""]
    start = p.start.astimezone(OSLO).strftime("%H:%M")
    title = wrap(plain(p.title), TITLE_WIDTH, 2) or [""]
    rows = ["{yellow}" + start + "{white}" + title[0]]
    rows += [" " * 6 + "{white}" + line for line in title[1:]]
    if p.organization:
        rows.append(" " * 6 + "{green}" + fit(plain(p.organization), TITLE_WIDTH))
    return [heading, *rows] + [""] * (3 - len(rows))


def directory(cfg: IndexConfig, live_pages: set[str] | None) -> list[Entry]:
    """Entries to list: those ttxd is sending, plus the ``always`` ones. With no
    page list from ttxd (``None``), every entry is listed."""
    shown = [
        e
        for e in cfg.entries
        if e.page != cfg.page and (live_pages is None or e.always or e.page.upper() in live_pages)
    ]
    return shown[:DIRECTORY_ROWS]


def render(
    cfg: IndexConfig,
    current: Programme | None,
    upcoming: Programme | None,
    entries: list[Entry],
) -> list[str]:
    rows = [BAND + r for r in banner(cfg.tagline, cfg.footer).markup()]
    rows += [
        bars(*STRIPE),
        "",
        *_programme("NÅ PÅ FRIKANALEN", current, flash=True),
        "",
        *_programme("DERETTER", upcoming, flash=False),
        "",
        tab("SIDER"),
    ]
    listed = [leader(e.title, e.page.upper()) for e in entries[:DIRECTORY_ROWS]]
    rows += listed + [""] * (DIRECTORY_ROWS - len(listed))
    rows += buttons(entries)
    return check(rows)
