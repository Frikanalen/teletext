"""Level 2.5 teletext enhancements: a colour palette, and pixel graphics drawn with
DRCS (dynamically redefinable characters).

Everything here rides on top of a normal Level 1 page, so an older decoder still
shows the plain page and a Level 2.5 decoder shows the enhanced one:

- **Palette (X/28/0 format 1).** Redefines colour tables 2 and 3 (16 colours) and
  remaps the Level 1 colour attributes onto table 2, so ``{cyan}`` on the page shows
  as Frikanalen amber, ``{white}`` as paper, black as ink, and so on.
- **DRCS.** A picture is cut into 12x10 pixel tiles. Each distinct tile becomes one
  character on a DRCS page (a hidden page with up to 48 tiles per subpage), and X/26
  triplets on the display page put those characters into cells, each with its own
  colour from the palette.
- **X/27/4** links the display page to its DRCS page.

The bit layouts follow ETS 300 706 as implemented by libzvbi (src/packet.c and
src/teletext.c), which ``tools/t42render.c`` uses to check the result.
"""

from dataclasses import dataclass, field

# A triplet before Hamming 24/18 coding: (address 0-63, mode 0-31, data 0-127).
Triplet = tuple[int, int, int]
RGB = tuple[int, int, int]

TILE_W, TILE_H = 12, 10
TILES_PER_DRCS_SUBPAGE = 48


def hex_rgb(s: str) -> RGB:
    s = s.lstrip("#")
    return int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16)


# Colour table 2, in the order of the Level 1 colours it stands in for
# (black, red, green, yellow, blue, magenta, cyan, white), from Frikanalen's on-air
# graphics palette.
BRAND = [
    hex_rgb("#0f1926"),  # black   -> ink (page background)
    hex_rgb("#f15645"),  # red     -> red
    hex_rgb("#64bc50"),  # green   -> green
    hex_rgb("#ffc709"),  # yellow  -> yellow
    hex_rgb("#18263a"),  # blue    -> ink-2 (bands)
    hex_rgb("#73284e"),  # magenta -> plum
    hex_rgb("#faaa3f"),  # cyan    -> amber (headings, page numbers)
    hex_rgb("#f5efe4"),  # white   -> paper (text)
]
INK = 16  # palette index of BRAND[0]


class Bits:
    """Packs fields into 18-bit triplets, first bit first (the order libzvbi reads)."""

    def __init__(self) -> None:
        self.value = 0
        self.n = 0

    def put(self, value: int, bits: int) -> "Bits":
        self.value |= (value & ((1 << bits) - 1)) << self.n
        self.n += bits
        return self

    def triplets(self, count: int = 13) -> list[Triplet]:
        if self.n > 18 * count:
            raise ValueError(f"{self.n} bits do not fit in {count} triplets")
        out = []
        for i in range(count):
            w = (self.value >> (18 * i)) & 0x3FFFF
            out.append((w & 0x3F, (w >> 6) & 0x1F, w >> 11))
        return out


def nibble(v: int) -> int:
    return (v * 15 + 127) // 255


def x28_palette(
    clut3: list[RGB], national: int = 2, clut2: list[RGB] | None = None
) -> list[Triplet]:
    """X/28/0 format 1 for a display page: colour tables 2 (``BRAND`` unless given)
    and 3, Level 1 colours remapped onto table 2, its black as the screen and row
    colour.

    ``national`` must match the page header's C12-C14 (2 = Swedish/Finnish/Hungarian,
    what ttxd sends), or a Level 2.5 decoder would switch character sets."""
    clut2 = clut2 or BRAND
    colours = list(clut2) + list(clut3) + [clut2[0]] * (8 - len(clut3))
    b = Bits()
    b.put(0, 4)  # page function: basic level one page
    b.put(0, 3)  # page coding: 7-bit odd parity
    b.put(national, 7)  # default G0/G2 set: Latin, same national option
    b.put(national, 7)  # second G0 set
    b.put(0, 1).put(0, 1).put(0, 1).put(0, 4)  # no side panels
    for r, g, bl in colours[:16]:
        b.put(nibble(r) | nibble(g) << 4 | nibble(bl) << 8, 12)
    b.put(INK, 5)  # default screen colour
    b.put(INK, 5)  # default row colour
    b.put(1, 1)  # black background substitution
    b.put(6, 3)  # colour table remapping: foreground and background from table 2
    return b.triplets()


def x28_drcs() -> list[Triplet]:
    """X/28/3 for a DRCS page: normal DRCS, every tile 12x10 one bit per pixel."""
    b = Bits()
    b.put(5, 4)  # page function: normal DRCS
    b.put(0, 3)  # page coding
    b.put(0, 11)  # reserved
    for _ in range(TILES_PER_DRCS_SUBPAGE):
        b.put(0, 4)  # mode 0: 12x10x1
    return b.triplets()


def x27_drcs_link(drcs_page: str) -> list[Triplet]:
    """X/27/4 linking a display page to its normal DRCS page (same magazine, subpage
    chosen by the DRCS mode triplet). Links: 0 GPOP, 1 POP, 2 GDRCS, 3 DRCS. The DRCS
    page also goes in link 1, which is where libzvbi looks for it.

    The DRCS page number must have tens digit 0 (e.g. 10F): libzvbi and our reading of
    the spec place the tens and magazine bits differently, and only a 0 there reads
    the same either way."""
    num = int(drcs_page[1:], 16)
    if num >> 4:
        raise ValueError(f"DRCS page {drcs_page}: tens digit must be 0")
    link = 3 | 3 << 2 | (num & 15) << 7  # function bits set later, valid at 2.5 and 3.5
    none = 3 << 2 | 0xFF << 7  # page xFF: no link
    b = Bits()
    for slot, function in enumerate((0, 1, 2, 3)):
        word = (link if slot in (1, 3) else none) & ~3 | function
        b.put(word, 18)  # page number
        b.put(0, 18)  # subcode
    return b.triplets()


def row_address(row: int) -> int:
    return 40 if row == 24 else 40 + row


@dataclass
class Picture:
    """A picture in teletext pixels: ``pixels[y][x]`` is an RGB colour, or None where
    the background shows through."""

    pixels: list[list[RGB | None]]

    @property
    def width(self) -> int:
        return len(self.pixels[0]) if self.pixels else 0

    @property
    def height(self) -> int:
        return len(self.pixels)

    @classmethod
    def blank(cls, width: int, height: int) -> "Picture":
        return cls([[None] * width for _ in range(height)])

    def paste(self, other: "Picture", x: int, y: int) -> None:
        """Copy ``other``'s pixels (not its background) with its corner at (x, y)."""
        for dy, row in enumerate(other.pixels):
            for dx, c in enumerate(row):
                if c is not None:
                    self.pixels[y + dy][x + dx] = c


def read_ppm(path: str, background: RGB, threshold: int = 150) -> Picture:
    """Read a binary PPM (P6). Pixels within ``threshold`` (sum of channel
    differences) of ``background`` count as background."""
    with open(path, "rb") as f:
        data = f.read()
    fields: list[bytes] = []
    pos = 0
    while len(fields) < 4:
        while data[pos : pos + 1].isspace():
            pos += 1
        if data[pos : pos + 1] == b"#":
            pos = data.index(b"\n", pos)
            continue
        end = pos
        while not data[end : end + 1].isspace():
            end += 1
        fields.append(data[pos:end])
        pos = end
    if fields[0] != b"P6" or fields[3] != b"255":
        raise ValueError(f"{path}: want an 8-bit binary PPM")
    w, h = int(fields[1]), int(fields[2])
    raw = data[pos + 1 : pos + 1 + w * h * 3]
    rows = []
    for y in range(h):
        row: list[RGB | None] = []
        for x in range(w):
            i = (y * w + x) * 3
            c = (raw[i], raw[i + 1], raw[i + 2])
            near = sum(abs(a - b) for a, b in zip(c, background)) <= threshold
            row.append(None if near else c)
        rows.append(row)
    return Picture(rows)


def nearest(c: RGB, palette: list[RGB]) -> int:
    return min(range(len(palette)), key=lambda i: sum((a - b) ** 2 for a, b in zip(c, palette[i])))


@dataclass
class Graphic:
    """A picture cut into DRCS tiles, placed on a page.

    ``cells`` maps (row, column) to (tile number, palette index). Tile numbers count
    across DRCS subpages: tile n is PTU ``n % 48`` on subpage ``n // 48``."""

    tiles: list[tuple[int, ...]] = field(default_factory=list)
    cells: dict[tuple[int, int], tuple[int, int]] = field(default_factory=dict)

    def drcs_subpages(self) -> list[dict]:
        """ttxd subpages for the DRCS page: raw rows with two tiles each, X/28/3."""
        subpages = []
        for first in range(0, max(len(self.tiles), 1), TILES_PER_DRCS_SUBPAGE):
            chunk = self.tiles[first : first + TILES_PER_DRCS_SUBPAGE]
            raw_rows = {}
            for n in range(0, len(chunk), 2):
                row = list(chunk[n]) + list(chunk[n + 1] if n + 1 < len(chunk) else chunk[n])
                raw_rows[str(n // 2 + 1)] = row
            subpages.append(
                {
                    "rows": [],
                    "raw_rows": raw_rows,
                    "packets": [{"packet": 28, "designation": 3, "triplets": x28_drcs()}],
                }
            )
        return subpages

    def x26(
        self,
        extra: dict[int, list[Triplet]] | None = None,
        blank: set[tuple[int, int]] = frozenset(),
        row_colours: dict[int, int] | None = None,
    ) -> list[Triplet]:
        """Triplets that draw the graphic: per row, the active position, the DRCS
        subpage when it changes, and per cell a colour change (when needed) and the
        DRCS character. ``extra`` adds column triplets (column, mode, data) to rows,
        merged in column order; they come before the graphic's own in the same cell.
        Cells in ``blank`` that the graphic leaves empty get a space, hiding whatever
        the Level 1 page has there. ``row_colours`` gives rows a full row colour (a
        palette index), which shows wherever the row's background is transparent."""
        extra = {r: list(t) for r, t in (extra or {}).items()}
        for row, col in blank:
            if (row, col) not in self.cells:
                extra.setdefault(row, []).append((col, 0x09, 0x20))  # G0 character
        out: list[Triplet] = []
        subcode = 0
        row_colours = row_colours or {}
        for row in sorted({r for r, _ in self.cells} | set(extra) | set(row_colours)):
            if row in row_colours:
                out.append((row_address(row), 0x01, row_colours[row]))  # this row only
            out.append((row_address(row), 0x04, 0))  # set active position: row
            subpages = {
                t // TILES_PER_DRCS_SUBPAGE for (r, _), (t, _) in self.cells.items() if r == row
            }
            if len(subpages) > 1:
                raise ValueError(f"row {row} uses tiles from DRCS subpages {sorted(subpages)}")
            # ttxd numbers subpages 1, 2, ... when there are several, and the DRCS mode
            # triplet names the DRCS subpage by that number.
            several = len(self.tiles) > TILES_PER_DRCS_SUBPAGE
            if subpages and {s + several for s in subpages} != {subcode}:
                (subcode,) = {s + several for s in subpages}
                # DRCS mode: normal DRCS (bit 6) from this subpage on.
                out.append((row_address(row), 0x18, 0x40 | subcode))
            colour = None
            pending = sorted(extra.get(row, []))
            for col in sorted(c for r, c in self.cells if r == row):
                while pending and pending[0][0] <= col:
                    out.append(pending.pop(0))
                tile, pal = self.cells[(row, col)]
                if pal != colour:
                    out.append((col, 0x00, pal))  # foreground colour
                    colour = pal
                out.append((col, 0x0D, 0x40 | tile % TILES_PER_DRCS_SUBPAGE))
            out.extend(pending)
        return out


def tile_bytes(bits: list[list[bool]]) -> tuple[int, ...]:
    """A 12x10 tile as 20 DRCS bytes: per pixel row, two bytes of six pixels, the
    leftmost in bit 5, with bit 6 set."""
    out = []
    for y in range(TILE_H):
        for half in (0, 6):
            v = 0x40
            for x in range(6):
                if bits[y][half + x]:
                    v |= 0x20 >> x
            out.append(v)
    return tuple(out)


def make_graphic(
    pic: Picture,
    top_row: int,
    left_col: int,
    palette: list[RGB],
    background: RGB,
    first_index: int = 16,
) -> Graphic:
    """Cut ``pic`` into tiles placed from (``top_row``, ``left_col``). A cell has one
    colour: the palette colour most of its pixels are nearest to, and pixels of any
    other colour are left out of it. Empty cells are left alone; identical tiles are
    shared."""
    g = Graphic()
    rows = -(-pic.height // TILE_H)
    cols = -(-pic.width // TILE_W)
    found: dict[tuple[int, int], tuple[tuple[int, ...], int]] = {}
    for cy in range(rows):
        for cx in range(cols):
            # The palette colour of each pixel, weighted by how far it is from the
            # background (so soft edges count for less).
            weights: dict[int, int] = {}
            pixels = {}
            for y in range(TILE_H):
                for x in range(TILE_W):
                    px, py = cx * TILE_W + x, cy * TILE_H + y
                    c = pic.pixels[py][px] if py < pic.height and px < pic.width else None
                    if c:
                        pixels[(x, y)] = nearest(c, palette)
                        w = sum(abs(a - b) for a, b in zip(c, background)) ** 2
                        weights[pixels[(x, y)]] = weights.get(pixels[(x, y)], 0) + w
            if not weights:
                continue
            colour = max(weights, key=weights.get)
            bits = [[pixels.get((x, y)) == colour for x in range(TILE_W)] for y in range(TILE_H)]
            found[(cy, cx)] = (tile_bytes(bits), first_index + colour)
    # A row can only use tiles from one DRCS subpage, so fill subpages row by row.
    index: dict[tuple[int, ...], int] = {}
    for cy in range(rows):
        new = list(
            dict.fromkeys(t for (r, _), (t, _) in found.items() if r == cy and t not in index)
        )
        if len(new) > TILES_PER_DRCS_SUBPAGE:
            raise ValueError(f"row {top_row + cy} needs {len(new)} tiles")
        used = sorted(
            {
                index[t] // TILES_PER_DRCS_SUBPAGE
                for (r, _), (t, _) in found.items()
                if r == cy and t in index
            }
        )
        room = TILES_PER_DRCS_SUBPAGE - len(g.tiles) % TILES_PER_DRCS_SUBPAGE
        current = len(g.tiles) // TILES_PER_DRCS_SUBPAGE
        if (used and used != [current]) or len(new) > room:
            # Start the row on a fresh subpage, repeating tiles it shares with earlier rows.
            if len(g.tiles) % TILES_PER_DRCS_SUBPAGE:
                g.tiles += [g.tiles[-1]] * room
            new = list(dict.fromkeys(t for (r, _), (t, _) in found.items() if r == cy))
            for t in new:
                index[t] = len(g.tiles)
                g.tiles.append(t)
        else:
            for t in new:
                index[t] = len(g.tiles)
                g.tiles.append(t)
        for (r, cx), (t, pal) in found.items():
            if r == cy:
                g.cells[(top_row + cy, left_col + cx)] = (index[t], pal)
    return g
