"""Page 100: the index. What is on now and next, and a directory of the other pages.

Layout (row numbers are display rows; ttxd sends the header row itself)::

     1-4  banner on blue: F R I K A N A L E N and TEKST-TV on row 2, tagline on row 4
     6-9  NÅ: start time and title (up to two lines), organisation
          (DIREKTE flashes when live)
    11-14 DERETTER: the same for the next programme
    16    SIDER
    17-22 directory, up to 6 entries: "Sendeplan ........200"
    24    footer band

On a Level 2.5 decoder (see level25.py) the page gets Frikanalen's colours and the
logo, drawn in DRCS pixels, replaces the FRIKANALEN text on rows 1-3. The banner has
no double height: a Level 1 double-height row hides the row below it on every
decoder, and the logo needs rows 1-3 visible.
"""

import functools
import os
from dataclasses import dataclass, field

from . import level25
from .markup import COLS, check, fit, leader, plain, spread, wrap
from .schedule import OSLO, Programme

DIRECTORY_ROWS = 6
LOGO = os.path.join(os.path.dirname(__file__), "assets", "logo.ppm")
LOGO_ROW, LOGO_COL = 1, 2
TITLE = "F R I K A N A L E N"  # the Level 1 stand-in for the logo
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


@dataclass
class Entry:
    page: str
    title: str
    # List the page even when ttxd is not sending it (e.g. subtitles on 888).
    always: bool = False


@dataclass
class IndexConfig:
    page: str = "100"
    tagline: str = "Frivillige organisasjoners TV-kanal"
    footer: str = "frikanalen.tv"
    entries: list[Entry] = field(default_factory=list)
    # Level 2.5 colours and logo. The logo's pixels go on a hidden DRCS page; its
    # number needs tens digit 0 and a hex units digit so nobody can key it in.
    level25: bool = True
    drcs_page: str = "10F"


@functools.cache
def logo() -> level25.Graphic:
    pic = level25.read_ppm(LOGO, level25.BRAND[0])
    return level25.make_graphic(
        pic, LOGO_ROW, LOGO_COL, level25.BRAND + LOGO_COLOURS, level25.BRAND[0]
    )


def enhancements(cfg: IndexConfig) -> dict:
    """Level 2.5 data for page 100: palette, link to the DRCS page, and the logo."""
    # Hide the Level 1 title wherever the logo has no pixels.
    title = range(LOGO_COL, LOGO_COL + 2 + len(TITLE))
    x26 = logo().x26(blank={(r, c) for r in range(LOGO_ROW, LOGO_ROW + 3) for c in title})
    return {
        "x26": x26,
        "packets": [
            {"packet": 27, "designation": 4, "triplets": level25.x27_drcs_link(cfg.drcs_page)},
            {"packet": 28, "designation": 0, "triplets": level25.x28_palette(LOGO_COLOURS)},
        ],
    }


def _band(markup: str = "") -> str:
    return "{blue}{newbg}" + markup


def _programme(label: str, p: Programme | None, flash: bool) -> list[str]:
    live = ""
    if p and p.live:
        live = "{red}{flash}DIREKTE" if flash else "{red}DIREKTE"
    heading = spread("{cyan}" + label, live)
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
    rows = [
        _band(),
        spread(_band("{yellow}" + TITLE), "{white}TEKST-TV "),
        _band(),
        _band("{white}" + fit(plain(cfg.tagline), COLS - 3)),
        "",
        *_programme("NÅ PÅ FRIKANALEN", current, flash=True),
        "",
        *_programme("DERETTER", upcoming, flash=False),
        "",
        "{yellow}SIDER",
    ]
    listed = [leader(e.title, e.page.upper()) for e in entries[:DIRECTORY_ROWS]]
    rows += listed + [""] * (DIRECTORY_ROWS - len(listed))
    rows += ["", _band("{white}" + fit(plain(cfg.footer), COLS - 3))]
    return check(rows)
