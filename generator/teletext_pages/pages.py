"""The pages behind the index: the programme schedule and the information page.

Both share a frame with page 100: the web address and TEKST-TV on the top row, the
page's name in block letters over three bars in the logo's colours, section headings
as slanted tabs, and the colour-key chips on the last two rows. On a Level 2.5 decoder
they get page 100's palette, the standard key colours on the chips, and straight
slants from the shared glyphs.

These are the two pages a DVB teletext descriptor can point a TV at besides the
initial page: "programme schedule page" and "additional information page".
"""

from dataclasses import dataclass, field
from datetime import datetime, timedelta

from . import index, level25
from .art import CYAN, GREEN, RED, WHITE, YELLOW, Picture
from .index import BUTTON_ROW, Entry, IndexConfig
from .markup import COLS, ROWS, check, fit, plain, wrap
from .mosaic import lettering
from .schedule import OSLO, Programme

TITLE_Y = 3  # sixels: the block letters stand on rows 2-4
BARS_ROW = 4  # 0-based: the three bars are the top third of this row
BODY_TOP, BODY_END = 5, BUTTON_ROW - 1  # 0-based rows for the page's own content
TEXT_COL, TEXT_WIDTH = 1, COLS - 2
TIME_COL, TITLE_COL = 1, 8
DAYS = ["mandag", "tirsdag", "onsdag", "torsdag", "fredag", "lørdag", "søndag"]
MONTHS = [
    "januar",
    "februar",
    "mars",
    "april",
    "mai",
    "juni",
    "juli",
    "august",
    "september",
    "oktober",
    "november",
    "desember",
]


@dataclass
class ScheduleConfig:
    enabled: bool = True
    page: str = "200"
    title: str = "SENDEPLAN"
    # Programmes on a subpage (two rows each), and how many subpages at most.
    per_page: int = 8
    max_subpages: int = 6
    cycle_secs: int = 12


@dataclass
class Section:
    heading: str
    text: str


def _default_sections() -> list[Section]:
    return [
        Section(
            "OM FRIKANALEN",
            "Frikanalen er sivilsamfunnets TV-kanal. Her sender organisasjoner og "
            "foreninger sine egne programmer.",
        ),
        Section(
            "SE OSS",
            "Direkte og i arkivet på frikanalen.tv. Sendeplanen står på side 200.",
        ),
        Section(
            "BLI MED",
            "Organisasjoner kan bli medlem og sende egne programmer. Les mer på frikanalen.tv.",
        ),
    ]


@dataclass
class InfoConfig:
    enabled: bool = True
    page: str = "300"
    title: str = "INFORMASJON"
    sections: list[Section] = field(default_factory=_default_sections)
    cycle_secs: int = 15


def chips(page: str, home: IndexConfig, entries: list[Entry]) -> list[Entry]:
    """The colour keys of a page behind the index: the index first, then the other
    pages on the index's own keys."""
    others = [e for e in entries if e.page.upper() not in (page.upper(), home.page.upper())]
    return [Entry(home.page, "Forside"), *others][: index.BUTTONS]


def _frame(home: IndexConfig, title: str, keys: list[Entry], part: tuple[int, int]) -> Picture:
    pic = Picture(ROWS)
    pic.text(0, 1, fit(plain(home.footer), 16), WHITE)
    pic.text(0, COLS - 9, "TEKST-TV", CYAN)
    index._stamp(pic, 2, TITLE_Y, lettering(title), YELLOW)
    # Three bars in the logo's colours, as long as the letters above them.
    width = len(lettering(title)[0])
    third = width // 3
    for n, colour in enumerate((RED, YELLOW, GREEN)):
        for x in range(2 + n * third, 2 + (n + 1) * third if n < 2 else 2 + width):
            pic.sixels[BARS_ROW * 3][x] = colour
    if part[1] > 1:
        pic.text(BARS_ROW, COLS - 6, f"{part[0]}/{part[1]}".rjust(5), CYAN)
    index._buttons(pic, keys)
    return pic


def _subpage(home: IndexConfig, pic: Picture, keys: list[Entry], tabs: list[int], shared) -> dict:
    """A picture as a ttxd subpage: rows, fastext links, and the Level 2.5 data."""
    rows = check(pic.markup())
    out = {"rows": rows, "packets": [index.fastext(home, keys)]}
    if home.level25:
        colours = index._key_colours(rows)
        glyphs = (shared or {}).get("glyphs", {})
        slants, _ = index._finer(rows, {}, glyphs, tuple(tabs))
        out["x26"] = level25.page_x26(index._merge(colours, slants))
        out["packets"].append(
            {
                "packet": 28,
                "designation": 0,
                "triplets": level25.x28_palette([], clut2=index.PALETTE),
            }
        )
    return out


def day_label(day, today) -> str:
    name = f"{DAYS[day.weekday()]} {day.day}. {MONTHS[day.month - 1]}"
    if day == today:
        return f"I DAG {name}"
    if day == today + timedelta(days=1):
        return f"I MORGEN {name}"
    return name.upper()


def schedule_pages(
    cfg: ScheduleConfig,
    home: IndexConfig,
    items: list[Programme],
    at: datetime,
    entries: list[Entry],
    shared: dict | None = None,
) -> tuple[dict, ...]:
    """The schedule from the programme on air onwards: a subpage per ``per_page``
    programmes, and a new subpage where a new day starts."""
    today = at.astimezone(OSLO).date()
    coming = [p for p in items if p.end > at]
    groups: list[tuple[object, list[Programme]]] = []
    for p in coming:
        day = max(p.start.astimezone(OSLO).date(), today)
        if groups and groups[-1][0] == day and len(groups[-1][1]) < cfg.per_page:
            groups[-1][1].append(p)
        else:
            groups.append((day, [p]))
    groups = groups[: cfg.max_subpages] or [(today, [])]
    keys = chips(cfg.page, home, entries)
    out = []
    for n, (day, programmes) in enumerate(groups, 1):
        pic = _frame(home, cfg.title, keys, (n, len(groups)))
        index._heading(pic, BODY_TOP, day_label(day, today))
        if not programmes:
            pic.text(BODY_TOP + 2, TEXT_COL, "Ingen program i sendeplanen", WHITE)
        row = BODY_TOP + 1
        for p in programmes:
            on_air = p.start <= at < p.end
            clock = p.start.astimezone(OSLO).strftime("%H:%M")
            # The programme on air stands out in cyan.
            ink = CYAN if on_air else WHITE
            pic.text(row, TIME_COL, clock, CYAN if on_air else YELLOW)
            width = COLS - TITLE_COL
            if p.organization or p.live:
                pic.text(row, TITLE_COL, fit(plain(p.title), width), ink)
                col = TITLE_COL
                if p.live:
                    pic.text(row + 1, col, "DIREKTE", RED, flash=on_air)
                    col += 9  # room to end the flash and change colour
                if p.organization:
                    pic.text(row + 1, col, fit(plain(p.organization), COLS - col), GREEN)
            else:
                for i, line in enumerate(wrap(plain(p.title), width, 2)):
                    pic.text(row + i, TITLE_COL, line, ink)
            row += 2
        out.append(_subpage(home, pic, keys, [BODY_TOP], shared))
    return tuple(out)


def _blocks(cfg: InfoConfig) -> list[tuple[str, list[str]]]:
    return [
        (fit(plain(s.heading).upper(), COLS - 8), wrap(plain(s.text), TEXT_WIDTH, 40))
        for s in cfg.sections
    ]


def info_pages(
    cfg: InfoConfig, home: IndexConfig, entries: list[Entry], shared: dict | None = None
) -> tuple[dict, ...]:
    """The information page: each section a heading tab and its text. Sections that
    do not fit together go on further subpages; one too long for a page is cut."""
    room = BODY_END - BODY_TOP + 1
    sheets: list[list[tuple[str, list[str]]]] = [[]]
    used = 0
    for heading, lines in _blocks(cfg):
        lines = lines[: room - 1]
        need = 1 + len(lines) + (1 if sheets[-1] else 0)
        if sheets[-1] and used + need > room:
            sheets.append([])
            used, need = 0, need - 1
        sheets[-1].append((heading, lines))
        used += need
    keys = chips(cfg.page, home, entries)
    out = []
    for n, sheet in enumerate(sheets, 1):
        pic = _frame(home, cfg.title, keys, (n, len(sheets)))
        row = BODY_TOP
        tabs = []
        for heading, lines in sheet:
            index._heading(pic, row, heading)
            tabs.append(row)
            for i, line in enumerate(lines, 1):
                pic.text(row + i, TEXT_COL, line, WHITE)
            row += len(lines) + 2
        out.append(_subpage(home, pic, keys, tabs, shared))
    return tuple(out)
