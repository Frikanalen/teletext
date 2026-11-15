"""Level 1 mosaic ("block") graphics: draw in sixels, get rows of ttxd markup.

A mosaic cell is 2 sixels wide and 3 tall, so a 40x24 page is 80x72 sixels. A cell
has one colour, and a colour change takes an attribute cell, which shows as blank.
:class:`Canvas` places those attribute cells itself, in the blank cell before a
change of colour, and raises if a drawing leaves no room for one.

Art is given as strings, ``#`` for a set sixel and anything else for an empty one::

    canvas.draw(0, 0, ["##..", ".##."], "red")
"""

from dataclasses import dataclass, field

from .markup import cells

# Bit for each sixel (x, y) in a mosaic cell; the character code is 0x20 + the bits.
_BITS = {(0, 0): 1, (1, 0): 2, (0, 1): 4, (1, 1): 8, (0, 2): 16, (1, 2): 64}

COLOURS = ("red", "green", "yellow", "blue", "magenta", "cyan", "white")


def code(sixels: set[tuple[int, int]]) -> int:
    """Mosaic character for the set sixels of one cell."""
    return 0x20 + sum(_BITS[s] for s in sixels)


@dataclass
class Canvas:
    """``cols`` x ``rows`` cells of mosaic graphics, with text laid over them."""

    cols: int
    rows: int
    sixels: dict[tuple[int, int], str] = field(default_factory=dict)
    texts: dict[tuple[int, int], str] = field(default_factory=dict)

    def draw(self, x: int, y: int, art: list[str], colour: str) -> "Canvas":
        """Set sixels from ``art`` with its top left corner at sixel (x, y)."""
        if colour not in COLOURS:
            raise ValueError(f"unknown colour {colour}")
        for dy, line in enumerate(art):
            for dx, ch in enumerate(line):
                if ch == "#":
                    sx, sy = x + dx, y + dy
                    if not (0 <= sx < self.cols * 2 and 0 <= sy < self.rows * 3):
                        raise ValueError(f"sixel ({sx}, {sy}) is off the canvas")
                    self.sixels[(sx, sy)] = colour
        return self

    def text(self, row: int, col: int, markup: str) -> "Canvas":
        """Lay markup over cells from (row, col). It should start with its own colour
        attribute; the cells it covers must hold no graphics."""
        if col + cells(markup) > self.cols:
            raise ValueError(f"text {markup!r} runs off the canvas")
        self.texts[(row, col)] = markup
        return self

    def cell(self, row: int, col: int) -> tuple[str | None, set[tuple[int, int]]]:
        """The colour and set sixels of one cell."""
        found = {}
        for dy in range(3):
            for dx in range(2):
                c = self.sixels.get((col * 2 + dx, row * 3 + dy))
                if c:
                    found[(dx, dy)] = c
        colours = set(found.values())
        if len(colours) > 1:
            raise ValueError(f"cell ({row}, {col}) mixes {sorted(colours)}")
        return (colours.pop() if colours else None), set(found)

    def filled(self) -> set[tuple[int, int]]:
        """Cells (row, col) that show graphics or text."""
        out = {(sy // 3, sx // 2) for sx, sy in self.sixels}
        for (row, col), markup in self.texts.items():
            out |= {(row, c) for c in range(col, col + cells(markup))}
        return out

    def markup(self) -> list[str]:
        """One row of markup per canvas row."""
        out = []
        for row in range(self.rows):
            parts: list[str] = []
            mode = None  # graphics colour in force, or None after text
            col = 0
            while col < self.cols:
                if (row, col) in self.texts:
                    t = self.texts[(row, col)]
                    parts.append(t)
                    col += cells(t)
                    mode = None
                    continue
                colour, sixels = self.cell(row, col)
                if not sixels:
                    parts.append(" ")
                elif colour == mode:
                    parts.append(f"{{x:{code(sixels):02x}}}")
                else:
                    if not parts or parts[-1] != " ":
                        raise ValueError(f"no blank cell for {colour} before ({row}, {col})")
                    parts[-1] = "{g" + colour + "}"
                    parts.append(f"{{x:{code(sixels):02x}}}")
                    mode = colour
                col += 1
            out.append("".join(parts).rstrip())
        return out


# A pixel font for big lettering: 9 sixels tall, 5 wide (I is 2). F, R and I are
# bold like the logo's FRI.
FONT = {
    "F": ["#####", "#####", "##...", "##...", "####.", "####.", "##...", "##...", "##..."],
    "R": ["####.", "#####", "##.##", "##.##", "#####", "####.", "##.##", "##.##", "##.##"],
    "I": ["##"] * 9,
    "K": ["#...#", "#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#", "#...#"],
    "A": ["..#..", ".#.#.", ".#.#.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "N": ["#...#", "##..#", "##..#", "#.#.#", "#.#.#", "#.#.#", "#..##", "#..##", "#...#"],
    "L": ["#...."] * 8 + ["#####"],
    "E": ["#####", "#....", "#....", "#....", "####.", "#....", "#....", "#....", "#####"],
    "S": [".####", "#....", "#....", "#....", ".###.", "....#", "....#", "....#", "####."],
    "D": ["####."] + ["#...#"] * 7 + ["####."],
    "P": ["####.", "#...#", "#...#", "#...#", "####.", "#....", "#....", "#....", "#...."],
    "O": [".###."] + ["#...#"] * 7 + [".###."],
    "M": ["#...#", "##.##", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#", "#...#"],
    "J": ["....#"] * 6 + ["#...#", "#...#", ".###."],
}


def lettering(text: str, gap: int = 1) -> list[str]:
    """Art for ``text`` in :data:`FONT`."""
    lines = [""] * 9
    for n, ch in enumerate(text):
        glyph = FONT[ch]
        for i in range(9):
            lines[i] += ("." * gap if n else "") + glyph[i]
    return lines
