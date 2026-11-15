"""Page 100: the index. A sky, the logo, what is on now and next, and the way on.

Layout (display rows; ttxd sends the header row itself)::

     1-8  the sky over Oslo as it is now (see sky.py): a sliced sun on the horizon,
          or a moon and stars, with the weather drawn over it; the web address and
          TEKST-TV in the top corners
     8-12 the logo: the mark, FRIKANALEN in block letters, the tagline under it
    13-16 NÅ PÅ FRIKANALEN tab (DIREKTE flashes when live), start time and title
          (up to two lines), organisation
    18-21 DERETTER: the same for the next programme
    23-24 colour keys: red, green, yellow and cyan chips with the page numbers of up
          to four other pages, their names under them, linked to the remote's keys
          (fastext)

The whole page is one picture (see art.py), so the search there decides where the
colour attributes go.

On a Level 2.5 decoder (see level25.py) the sky gets a gradient, one colour per row,
that follows the sun from night through dawn and day to dusk; the page gets its own
palette; and the real logo, in DRCS pixels, replaces the block letters. Nothing on the
page is double height: a Level 1 double-height row hides the row below it on every
decoder.
"""

import functools
import os
import re
from dataclasses import dataclass, field

from . import art, level25, sky
from .art import BLACK, BLUE, CYAN, GREEN, MAGENTA, RED, WHITE, YELLOW, Picture
from .fastext import x27_0
from .markup import COLS, ROWS, check, fit, plain, wrap
from .mosaic import lettering
from .schedule import OSLO, Programme

BUTTONS = 4
ASSETS = os.path.join(os.path.dirname(__file__), "assets")
# The logo mark, 12x15 sixels, in three bands of colour like the real one.
MARK = [
    "..........##",
    ".......####.",
    "....######..",
    ".#########..",
    "...######...",
    ".....###....",
    ".#########..",
    "...#########",
    "......###...",
    "..####......",
    "...####.....",
    "...####.....",
    "....###.....",
    ".....##.....",
    "......#.....",
]
MARK_BANDS = [(0, 6, RED), (6, 9, YELLOW), (9, 15, GREEN)]
MARK_X, MARK_Y = 4, 21  # sixels: columns 2-7, rows 8-12
WORD_X, WORD_Y = 18, 24  # FRIKANALEN: columns 9-36, rows 9-11
TAGLINE_ROW, TAGLINE_COL = 11, 9  # 0-based row; the text lines up with the lettering
NOW_ROW, NEXT_ROW, BUTTON_ROW = 12, 17, 22
# The row between the programmes and the colour keys (as ttxd counts rows, from 1),
# where what comes later scrolls by.
TICKER_ROW = 22
# The label ends by setting the colour of whatever text has scrolled up to it.
TICKER_LABEL = "{cyan}SENERE{white}"
TICKER_SPEED = 4
TITLE_COL = 8
TITLE_WIDTH = COLS - TITLE_COL
# Level 2.5: the real logo over the block one. Mark on rows 8-12 from column 2,
# wordmark on rows 9-11 from column 9, 3 pixels below the top of its first row.
LOGO_ROW, LOGO_COL, WORD_COL, WORD_TOP = 8, 2, 9, 3
# Where the mark goes in that, in pixels. A cell has one colour, and from here the
# blades fall in cells of their own but for a few pixels.
MARK_AT = (3, 5)
# The logo as a shared object in ttxd (PUT /assets), for any page to place.
LOGO_OBJECT = "logo"
LOGO_ROWS = range(7, 12)  # 0-based rows the logo touches
# Colour table 2 on a Level 2.5 decoder, in the order of the Level 1 colours it
# stands in for: black, red, green, yellow, blue, magenta, cyan, white.
PALETTE = [
    level25.hex_rgb(c)
    for c in (
        "#0f0a26",  # ink: the page
        "#f15645",
        "#64bc50",
        "#ffc709",
        "#2c1a5c",  # the bands behind the headings
        "#e83e8c",  # the heading tabs and the foot of the sun
        "#4fd8e8",
        "#f5efe4",  # paper: text
    )
]
SKY_COLOUR = 24  # palette index of the first colour of table 3, the sky's gradient
# The most X/26 triplets a page takes is 16 packets of 13, less one to end them; ttxd
# adds a few of its own for Æ and Ø.
MAX_TRIPLETS = 190
CHIPS = [(RED, WHITE), (GREEN, BLUE), (YELLOW, BLUE), (CYAN, BLUE)]  # chip, its number
# The chips keep the colours of the remote's keys on a Level 2.5 decoder too: these
# Level 1 colours are taken from colour table 0, which the palette leaves alone.
KEY_COLOURS = {"red": 1, "green": 2, "yellow": 3, "cyan": 6}
CHIP_WIDTH = 10


@dataclass
class Entry:
    page: str
    title: str
    # Offer the page even when ttxd is not sending it (e.g. subtitles on 888).
    always: bool = False


@dataclass
class IndexConfig:
    page: str = "100"
    tagline: str = "Sivilsamfunnets kanal"
    footer: str = "frikanalen.tv"  # web address, top left
    entries: list[Entry] = field(default_factory=list)
    # Level 2.5 colours and logo. The logo's pixels go on a hidden DRCS page; its
    # number needs tens digit 0 and a hex units digit so nobody can key it in.
    level25: bool = True
    drcs_page: str = "10F"
    # Scroll the programmes after the next one along the row above the colour keys.
    ticker: bool = True
    # Where the sky is, and whether to ask MET Norway for its weather.
    latitude: float = 59.91
    longitude: float = 10.75
    weather: bool = True


def _stamp(pic: Picture, x: int, y: int, art: list[str], colour: int) -> None:
    for dy, line in enumerate(art):
        for dx, ch in enumerate(line):
            if ch == "#":
                pic.sixels[y + dy][x + dx] = colour


def _fill(pic: Picture, row: int, col0: int, col1: int, colour: int) -> None:
    for y in range(row * 3, row * 3 + 3):
        for x in range(col0 * 2, col1 * 2):
            pic.sixels[y][x] = colour


def _logo(pic: Picture, tagline: str) -> None:
    """The block logo. It stands in front of the sky: nothing shows through it."""
    for y in range(MARK_Y, MARK_Y + 3):
        for x in range(WORD_X - 2):
            pic.sixels[y][x] = BLACK
    for top, bottom, colour in MARK_BANDS:
        _stamp(pic, MARK_X, MARK_Y + top, MARK[top:bottom], colour)
    _stamp(pic, WORD_X, WORD_Y, lettering("FRIKANALEN"), WHITE)
    pic.text(TAGLINE_ROW, TAGLINE_COL, fit(plain(tagline), COLS - TAGLINE_COL), YELLOW)


def _heading(pic: Picture, row: int, label: str, live: bool = False) -> None:
    """A section heading: a tab with a slanted end on a band."""
    end = len(label) + 4
    _fill(pic, row, 1, end, MAGENTA)
    _fill(pic, row, end, COLS - 1, BLUE)
    for y, width in enumerate((2, 1, 0)):
        for x in range(end * 2, end * 2 + width):
            pic.sixels[row * 3 + y][x] = MAGENTA
    pic.text(row, 3, label, WHITE, MAGENTA)
    if live:
        _fill(pic, row, COLS - 11, COLS - 1, RED)
        pic.text(row, COLS - 9, "DIREKTE", YELLOW, RED, flash=True)


def _programme(pic: Picture, row: int, label: str, p: Programme | None, flash: bool) -> None:
    _heading(pic, row, label, live=bool(p and p.live and flash))
    if p and p.live and not flash:
        _fill(pic, row, COLS - 11, COLS - 1, RED)
        pic.text(row, COLS - 9, "DIREKTE", YELLOW, RED)
    if p is None:
        pic.text(row + 1, 1, "Ingen program i sendeplanen", WHITE)
        return
    pic.text(row + 1, 1, p.start.astimezone(OSLO).strftime("%H:%M"), YELLOW)
    lines = wrap(plain(p.title), TITLE_WIDTH, 2) or [""]
    for n, line in enumerate(lines):
        pic.text(row + 1 + n, TITLE_COL, line, WHITE)
    if p.organization:
        pic.text(row + 1 + len(lines), TITLE_COL, fit(plain(p.organization), TITLE_WIDTH), GREEN)


def _buttons(pic: Picture, entries: list[Entry]) -> None:
    """Rows 23-24: a slanted chip in each key's colour with the page number on it,
    and the page's name under it."""
    for n, (e, (colour, ink)) in enumerate(zip(entries, CHIPS)):
        col = n * CHIP_WIDTH
        for y in range(3):
            for x in range(col * 2 + 4 - y, col * 2 + 20 - y):
                pic.sixels[BUTTON_ROW * 3 + y][x] = colour
        pic.text(BUTTON_ROW, col + 4, e.page.upper()[:3], ink, colour)
        pic.text(BUTTON_ROW + 1, col + 1, fit(plain(e.title), CHIP_WIDTH - 1), colour)


def directory(cfg: IndexConfig, live_pages: set[str] | None) -> list[Entry]:
    """Pages for the colour keys: those ttxd is sending, plus the ``always`` ones.
    With no page list from ttxd (``None``), every entry counts."""
    shown = [
        e
        for e in cfg.entries
        if e.page != cfg.page and (live_pages is None or e.always or e.page.upper() in live_pages)
    ]
    return shown[:BUTTONS]


def picture(
    cfg: IndexConfig,
    current: Programme | None,
    upcoming: Programme | None,
    entries: list[Entry],
    at: sky.Look,
) -> Picture:
    return _picture(cfg, current, upcoming, entries, at)[0]


def _picture(
    cfg: IndexConfig,
    current: Programme | None,
    upcoming: Programme | None,
    entries: list[Entry],
    at: sky.Look,
) -> tuple[Picture, dict[tuple[int, int], str]]:
    """The page as a picture, and the sky's stars, drops and flakes (see sky.draw)."""
    pic = Picture(ROWS)
    small = sky.draw(pic, at)
    pic.text(0, 1, fit(plain(cfg.footer), 16), WHITE)
    pic.text(0, COLS - 9, "TEKST-TV", CYAN)
    _logo(pic, cfg.tagline)
    # A Level 2.5 decoder draws the real logo over these rows, and only replaces
    # characters: the block logo must not lean on background colours.
    pic.plain_rows |= set(LOGO_ROWS)
    _programme(pic, NOW_ROW, "NÅ PÅ FRIKANALEN", current, flash=True)
    _programme(pic, NEXT_ROW, "DERETTER", upcoming, flash=False)
    _buttons(pic, entries)
    return pic, small


def render(
    cfg: IndexConfig,
    current: Programme | None,
    upcoming: Programme | None,
    entries: list[Entry],
    at: sky.Look,
) -> list[str]:
    """The page's 24 rows of markup."""
    return check(picture(cfg, current, upcoming, entries, at).markup())


def ticker(cfg: IndexConfig, later: list[Programme]) -> list[dict]:
    """ttxd's ``tickers`` for the page: the programmes after the next one, scrolling
    along the row above the colour keys. ttxd moves it; the page is sent as usual."""
    if not cfg.ticker or not later:
        return []
    parts = [f"{{yellow}}{p.start.astimezone(OSLO):%H:%M}{{white}}{plain(p.title)}" for p in later]
    return [
        {
            "row": TICKER_ROW,
            "prefix": TICKER_LABEL,
            "text": " ".join(parts) + " ",
            "cells_per_sec": TICKER_SPEED,
        }
    ]


def fastext(cfg: IndexConfig, entries: list[Entry]) -> dict:
    """The X/27/0 packet linking the colour keys to the button pages."""
    pages = [e.page.upper() for e in entries[:BUTTONS]]
    return {"packet": 27, "designation": 0, "raw": x27_0(cfg.page, pages, index=cfg.page)}


def _read(name: str) -> level25.Picture:
    return level25.read_ppm(os.path.join(ASSETS, name), level25.BRAND[0])


@functools.cache
def logo() -> level25.Graphic:
    """The real logo for Level 2.5: the mark, and the wordmark beside it."""
    mark, word = _read("mark.ppm"), _read("wordmark.ppm")
    pic = level25.Picture.blank((WORD_COL - LOGO_COL) * level25.TILE_W + word.width, 50)
    pic.paste(mark, *MARK_AT)
    pic.paste(word, (WORD_COL - LOGO_COL) * level25.TILE_W, level25.TILE_H + WORD_TOP)
    return level25.make_graphic(pic, LOGO_ROW, LOGO_COL, PALETTE, level25.BRAND[0])


def _slant(y: int) -> int:
    """How many of a cell's 12 pixels are lit in pixel row ``y`` of a slant that runs
    from a full top third to an empty bottom third, as the sixels 11/10/00 do."""
    return max(0, min(12, round(12 - (y - 1.5) * 12 / 7)))


def _glyph(lit) -> list[str]:
    return ["".join("#" if lit(x, y) else "." for x in range(12)) for y in range(10)]


# Shared glyphs: finer versions of what the Level 1 page draws with sixels. The four
# slants are named after the mosaic they stand in for (its bits, see art._BITS).
SLANTS = {
    0b0000111: ("slant-ul", lambda x, y: x < _slant(y)),
    0b1011000: ("slant-lr", lambda x, y: x >= _slant(y)),
    0b0001011: ("slant-ur", lambda x, y: 11 - x < _slant(y)),
    0b1010100: ("slant-ll", lambda x, y: 11 - x >= _slant(y)),
}
SMALL_GLYPHS = {
    "star-0": [
        "............",
        ".....#......",
        ".....#......",
        "....###.....",
        ".#########..",
        "....###.....",
        ".....#......",
        ".....#......",
        "............",
        "............",
    ],
    "star-1": [
        "............",
        "............",
        "............",
        ".....#......",
        "...#####....",
        ".....#......",
        "............",
        "............",
        "............",
        "............",
    ],
    "star-2": ["............"] * 4 + ["....##......"] + ["............"] * 5,
    "rain": [
        "............",
        "........##..",
        ".......##...",
        ".......##...",
        "......##....",
        ".....##.....",
        ".....##.....",
        "....##......",
        "............",
        "............",
    ],
    "snow": [
        ".....#......",
        "..#..#..#...",
        "...#.#.#....",
        "....###.....",
        ".#########..",
        "....###.....",
        "...#.#.#....",
        "..#..#..#...",
        ".....#......",
        "............",
    ],
}
SMALL_COLOURS = {"star": WHITE, "rain": CYAN, "snow": WHITE}


def assets() -> dict:
    """The shared Level 2.5 assets this generator keeps in ttxd: the logo as an object
    (its tiles stay on the DRCS page, which a page placing it must name as its
    ``drcs_page``), and glyphs for slanted edges, stars, rain and snow."""
    x26 = logo().object_x26(_block_logo_cells())
    glyphs = [{"name": name, "rows": _glyph(lit)} for name, lit in SLANTS.values()]
    glyphs += [{"name": name, "rows": rows} for name, rows in SMALL_GLYPHS.items()]
    return {
        "glyphs": glyphs,
        "objects": [{"name": LOGO_OBJECT, "type": "adaptive", "x26": x26}],
    }


def _finer(
    rows: list[str], small: dict[tuple[int, int], str], glyphs: dict[str, int]
) -> tuple[dict[int, list[level25.Triplet]], dict[int, list[level25.Triplet]]]:
    """Shared glyphs over cells of the Level 1 page, as X/26 triplets by display row:
    first the slanted ends of the heading tabs and the key chips, then the sky's
    stars, drops and flakes. A glyph takes the colours the cell already has, so only
    cells whose mosaic is nothing but that shape qualify."""
    slants: dict[int, list[level25.Triplet]] = {}
    for row in (NOW_ROW, NEXT_ROW, BUTTON_ROW):
        for col, (bits, _, _) in enumerate(art.shown(rows[row])):
            if bits in SLANTS and SLANTS[bits][0] in glyphs:
                slants.setdefault(row + 1, []).append((col, 0x0D, glyphs[SLANTS[bits][0]]))
    # A chip's right end is a whole cell of its colour on Level 1 (the next chip's
    # colour attribute sits there). Cut the slant out of it in the page's ink.
    if "slant-lr" in glyphs:
        for col, (bits, _, bg) in enumerate(art.shown(rows[BUTTON_ROW])):
            if col % CHIP_WIDTH == CHIP_WIDTH - 1 and bits == 0 and bg != BLACK:
                cut = [(col, 0x00, level25.INK), (col, 0x0D, glyphs["slant-lr"])]
                slants.setdefault(BUTTON_ROW + 1, []).extend(cut)
        slants.get(BUTTON_ROW + 1, []).sort()
    sky_cells: dict[int, list[level25.Triplet]] = {}
    for row in range(sky.ROWS):
        for col, (bits, fg, bg) in enumerate(art.shown(rows[row])):
            if not bits or bg != BLACK:
                continue
            lit = [(col * 2 + x, row * 3 + y) for x, y, bit in art._BITS if bits & bit]
            kinds = {small.get(at) for at in lit}
            if len(kinds) != 1 or None in kinds:
                continue
            (kind,) = kinds
            name = f"star-{(lit[0][0] * 7 + lit[0][1] * 3) % 3}" if kind == "star" else kind
            if fg == SMALL_COLOURS[kind] and name in glyphs:
                sky_cells.setdefault(row + 1, []).append((col, 0x0D, glyphs[name]))
    return slants, sky_cells


def _block_logo_cells() -> set[tuple[int, int]]:
    """Cells (display row, column) the block logo draws in."""
    pic = Picture(ROWS)
    _logo(pic, "")
    return {
        (y // 3 + 1, x // 2)
        for y, line in enumerate(pic.sixels)
        for x, colour in enumerate(line)
        if colour != BLACK
    }


def _tokens(row: str) -> list[str]:
    """A row of markup as one token per cell."""
    return re.findall(r"\{[^{}]*\}|.", row)


def _key_colours(rows: list[str]) -> dict[int, list[level25.Triplet]]:
    """X/26 colours from table 0 for the colour key rows: a foreground after each
    Level 1 attribute that picks a key colour, and a background where one becomes the
    background. The next Level 1 attribute of the same kind ends each of them."""
    out: dict[int, list[level25.Triplet]] = {}
    for row in (BUTTON_ROW + 1, BUTTON_ROW + 2):
        triplets = []
        key = None
        for col, token in enumerate(_tokens(rows[row - 1])):
            name = token[1:-1].removeprefix("g") if token.startswith("{") else ""
            if name in ("black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"):
                key = KEY_COLOURS.get(name)
                if key and col + 1 < COLS:
                    triplets.append((col + 1, 0x00, key))
            elif name == "newbg" and key:
                triplets.append((col, 0x03, key))
        out[row] = triplets
    return out


def _merge(*parts: dict[int, list[level25.Triplet]]) -> dict[int, list[level25.Triplet]]:
    out: dict[int, list[level25.Triplet]] = {}
    for part in parts:
        for row, triplets in part.items():
            out.setdefault(row, []).extend(triplets)
    return out


def enhancements(
    cfg: IndexConfig,
    rows: list[str],
    at: sky.Look,
    shared: dict | None = None,
    small: dict[tuple[int, int], str] | None = None,
) -> dict:
    """Level 2.5 data for page 100: the palette with the sky's colours, the sky's
    gradient, the link to the DRCS page, and the logo. ``rows`` is the page's markup."""
    # One colour per sky row, as the row colour and as the background from column 0.
    # A Level 1 {blackbg} ends an X/26 background, so it is set again there.
    backgrounds: dict[int, list[level25.Triplet]] = {}
    row_colours = {}
    for row in range(1, sky.ROWS + 1):
        colour = SKY_COLOUR + row - 1
        row_colours[row] = colour
        backgrounds[row] = [(0, 0x03, colour)]
        for col, token in enumerate(_tokens(rows[row - 1])):
            if token == "{blackbg}":
                backgrounds[row].append((col, 0x03, colour))
    keys = _key_colours(rows)
    objects = (shared or {}).get("objects", {})
    if LOGO_OBJECT in objects:
        # The logo is a shared object: two triplets, wherever it is placed. That
        # leaves room to draw slants and the sky's small things with shared glyphs.
        place = {(LOGO_ROW, LOGO_COL): tuple(objects[LOGO_OBJECT])}
        slants, sky_cells = _finer(rows, small or {}, shared.get("glyphs", {}))
        budget = MAX_TRIPLETS - len(
            level25.page_x26(_merge(backgrounds, keys, slants), row_colours, place)
        )
        for row in sorted(sky_cells):
            # Each row costs its glyphs; stop when the packets are full.
            if len(sky_cells[row]) > budget:
                break
            budget -= len(sky_cells[row])
            slants = _merge(slants, {row: sky_cells[row]})
        keys = _merge(keys, slants)
        # libzvbi loses the row's background in the cell before a placement unless
        # it is set again there.
        backgrounds[LOGO_ROW].append((LOGO_COL - 1, 0x03, SKY_COLOUR + LOGO_ROW - 1))
        x26 = level25.page_x26(_merge(backgrounds, keys), row_colours, place)
    else:
        blank = _block_logo_cells()
        x26 = logo().x26(extra=backgrounds | keys, blank=blank, row_colours=row_colours)
        if len(x26) > MAX_TRIPLETS:
            # Too busy a sky: keep the gradient where no background attribute cuts it.
            plain_sky = {row: triplets[:1] for row, triplets in backgrounds.items()}
            x26 = logo().x26(extra=plain_sky | keys, blank=blank, row_colours=row_colours)
    palette = level25.x28_palette(sky.gradient(at), clut2=PALETTE)
    return {
        "x26": x26,
        # No X/27/4 link to the DRCS page: ttxd puts it in the magazine's MOT
        # (drcs_page on the PUT), which leaves the MOT's global links in force.
        "packets": [{"packet": 28, "designation": 0, "triplets": palette}],
    }


def page(
    cfg: IndexConfig,
    current: Programme | None,
    upcoming: Programme | None,
    entries: list[Entry],
    at: sky.Look,
    shared: dict | None = None,
) -> dict:
    """The index subpage as sent to ttxd: rows, the fastext links, and Level 2.5 data
    if enabled."""
    pic, small = _picture(cfg, current, upcoming, entries, at)
    rows = check(pic.markup())
    out = {"rows": rows, "packets": [fastext(cfg, entries)]}
    if cfg.level25:
        extra = enhancements(cfg, rows, at, shared, small)
        out["x26"] = extra["x26"]
        out["packets"] += extra["packets"]
    return out
