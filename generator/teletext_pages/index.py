"""Page 100: the index. What is on now and next, and a directory of the other pages.

Layout (row numbers are display rows; ttxd sends the header row itself)::

     1-2  FRIKANALEN banner, double height, on blue
     3    tagline, on blue
     5-7  NÅ: start time, title, organisation (DIREKTE flashes when live)
     9-11 DERETTER: the same for the next programme
    13    SIDER
    14-22 directory, up to 9 entries: "Sendeplan ........200"
    24    footer band
"""

from dataclasses import dataclass, field

from .markup import COLS, check, fit, leader, plain, spread
from .schedule import OSLO, Programme

DIRECTORY_ROWS = 9
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


def _band(markup: str = "") -> str:
    return "{blue}{newbg}" + markup


def _programme(label: str, p: Programme | None, flash: bool) -> list[str]:
    live = ""
    if p and p.live:
        live = "{red}{flash}DIREKTE" if flash else "{red}DIREKTE"
    heading = spread("{cyan}" + label, live)
    if p is None:
        return [heading, "{white}Ingen program i sendeplanen", ""]
    start = p.start.astimezone(OSLO).strftime("%H:%M")
    title = "{yellow}" + start + "{white}" + fit(plain(p.title), TITLE_WIDTH)
    org = ""
    if p.organization:
        org = " " * 6 + "{green}" + fit(plain(p.organization), TITLE_WIDTH)
    return [heading, title, org]


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
        spread(_band("{double}{yellow}FRIKANALEN"), "{white}TEKST-TV "),
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
