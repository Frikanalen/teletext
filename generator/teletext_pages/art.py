"""Level 1 teletext art: a picture in sixels, in any of the eight colours, turned into
rows of ttxd markup with foreground and background colours.

:mod:`mosaic` draws one colour on black and wants a blank cell before each change of
colour. This module does what teletext artists do by hand: every cell can show two
colours (a mosaic in the foreground colour on a ``{newbg}`` background), and with
``{hold}`` the cell that carries a colour change repeats the last mosaic instead of
going blank. Which attribute goes in which cell is found by a search over the row
that keeps the picture as close as possible to what was drawn.

A picture is 80 sixels wide and 3 sixels tall per row. A sixel is 1.25 times as wide
as it is tall on a 4:3 screen.
"""

from dataclasses import dataclass, field

from .markup import COLS

BLACK, RED, GREEN, YELLOW, BLUE, MAGENTA, CYAN, WHITE = range(8)
NAMES = ("black", "red", "green", "yellow", "blue", "magenta", "cyan", "white")
# Sixel (x, y) in a cell -> bit of the mosaic character (code = 0x20 + bits).
_BITS = ((0, 0, 1), (1, 0, 2), (0, 1, 4), (1, 1, 8), (0, 2, 16), (1, 2, 64))
# A cell that cannot be shown as asked (text in the wrong colour) costs this much.
_NEVER = 1000
_BEAM = 1200


@dataclass
class Text:
    text: str
    fg: int
    bg: int = BLACK
    flash: bool = False


@dataclass
class Picture:
    """``rows`` teletext rows of sixels, each a colour 0-7, with text laid over it.

    Text needs its colour set in the cell before it, and its background before that;
    leave the picture plain there, or the search will bend the picture to fit."""

    rows: int
    sixels: list[list[int]] = field(default_factory=list)
    texts: dict[tuple[int, int], Text] = field(default_factory=dict)
    # Rows drawn on a black background only. A Level 2.5 page that lays other
    # characters over a row needs this: it replaces the characters, not the background.
    plain_rows: set[int] = field(default_factory=set)

    def __post_init__(self) -> None:
        if not self.sixels:
            self.sixels = [[BLACK] * (COLS * 2) for _ in range(self.rows * 3)]

    def text(self, row: int, col: int, text: str, fg: int, bg: int = BLACK, flash=False) -> None:
        if col < 0 or col + len(text) > COLS:
            raise ValueError(f"text {text!r} runs off row {row}")
        self.texts[(row, col)] = Text(text, fg, bg, flash)

    def markup(self) -> list[str]:
        return [encode_row(self, row) for row in range(self.rows)]


def _wanted(pic: Picture, row: int) -> list:
    """Per cell: six sixel colours, or (character, fg, bg) for a text cell."""
    out: list = []
    for col in range(COLS):
        out.append(tuple(pic.sixels[row * 3 + y][col * 2 + x] for x, y, _ in _BITS))
    for (r, col), t in pic.texts.items():
        if r != row:
            continue
        for i, ch in enumerate(t.text):
            # A space only needs its background, so attributes can hide in it.
            out[col + i] = (t.bg,) * 6 if ch == " " else (ch, t.fg, t.bg, t.flash)
    return out


def _offer(nxt: dict, state: tuple, cost: int, path: tuple | None, token: str) -> None:
    if state not in nxt or cost < nxt[state][0]:
        nxt[state] = (cost, (path, token))


def _shown(want: tuple, state: tuple, bg: int, hold: bool) -> int:
    """How far an attribute cell is from ``want``: it shows as a space on ``bg`` or,
    with hold in graphics mode, as the held mosaic."""
    gfx, fg, _, _, held, _ = state
    if hold and gfx:
        if held is None:
            return _NEVER
        return sum(w != (fg if held & bit else bg) for w, (_, _, bit) in zip(want, _BITS))
    return sum(w != bg for w in want)


def encode_row(pic: Picture, row: int) -> str:
    """Markup for one row: a beam search over the decoder's state, cell by cell.

    The state is (graphics mode, foreground, background, hold, held mosaic, flash).
    The held mosaic is None when decoders disagree about it (after a change between
    text and graphics), and hold is not used then."""
    wanted = _wanted(pic, row)
    plain = row in pic.plain_rows
    start = (False, WHITE, BLACK, False, 0, False)
    beam: dict[tuple, tuple[int, tuple | None]] = {start: (0, None)}
    for want in wanted:
        nxt: dict[tuple, tuple[int, tuple | None]] = {}
        is_text = isinstance(want[0], str)
        for state, (cost, path) in beam.items():
            gfx, fg, bg, hold, held, flash = state
            # 1. Show a character.
            if is_text:
                ch, tfg, tbg, tflash = want
                ok = not gfx and fg == tfg and bg == tbg and flash == tflash
                _offer(nxt, state, cost + (0 if ok else _NEVER), path, ch)
                continue
            if gfx:
                bits = 0
                miss = 0
                for i, (_, _, bit) in enumerate(_BITS):
                    if want[i] == fg and fg != bg:
                        bits |= bit
                    elif want[i] != bg:
                        miss += 1
                token = f"{{x:{0x20 + bits:02x}}}"
                _offer(nxt, (gfx, fg, bg, hold, bits, flash), cost + miss, path, token)
            else:
                _offer(nxt, state, cost + sum(w != bg for w in want), path, " ")
            # 2. An attribute. Colours, release and flash take effect after their
            # cell; the backgrounds, hold and steady in it.
            base = cost + _shown(want, state, bg, hold)
            for c in range(1, 8):
                # Text colour: decoders differ on the held mosaic after the change.
                _offer(nxt, (False, c, bg, hold, None, flash), base, path, "{" + NAMES[c] + "}")
                kept = held if gfx else None
                _offer(nxt, (True, c, bg, hold, kept, flash), base, path, "{g" + NAMES[c] + "}")
            if not plain:
                at = cost + _shown(want, state, fg, hold)
                _offer(nxt, (gfx, fg, fg, hold, held, flash), at, path, "{newbg}")
            at = cost + _shown(want, state, BLACK, hold)
            _offer(nxt, (gfx, fg, BLACK, hold, held, flash), at, path, "{blackbg}")
            if hold:
                _offer(nxt, (gfx, fg, bg, False, held, flash), base, path, "{release}")
            else:
                at = cost + _shown(want, state, bg, True)
                _offer(nxt, (gfx, fg, bg, True, held, flash), at, path, "{hold}")
            flip = "{steady}" if flash else "{flash}"
            _offer(nxt, (gfx, fg, bg, hold, held, not flash), base, path, flip)
        beam = dict(sorted(nxt.items(), key=lambda kv: kv[1][0])[:_BEAM])
    cost, path = min(beam.values(), key=lambda v: v[0])
    if cost >= _NEVER:
        raise ValueError(f"row {row}: no room for the attributes its text needs")
    tokens = []
    while path:
        path, token = path
        tokens.append(token)
    return "".join(reversed(tokens)).rstrip(" ")
