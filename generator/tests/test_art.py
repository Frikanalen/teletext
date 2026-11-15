import re
import unittest

from teletext_pages import art
from teletext_pages.markup import cells

CODES = {n: i for i, n in enumerate(art.NAMES)}


def decode(row: str) -> list[tuple]:
    """What a Level 1 decoder shows: per cell, its six sixel colours (text cells
    count as background)."""
    gfx, fg, bg, hold, held = False, art.WHITE, art.BLACK, False, 0
    out = []
    for tok in re.findall(r"\{[^{}]*\}|.", row):
        name = tok[1:-1] if tok.startswith("{") else None
        bits = 0
        if name == "newbg":
            bg = fg
        elif name == "blackbg":
            bg = art.BLACK
        elif name == "hold":
            hold = True
        if name and name.startswith("x:"):
            bits = bytes.fromhex(name[2:])[0] - 0x20
            held = bits
        elif name and hold and gfx:
            bits = held
        out.append(tuple(fg if bits & bit else bg for _, _, bit in art._BITS))
        if name in CODES:
            gfx, fg, held = False, CODES[name], 0
        elif name and name[0] == "g" and name[1:] in CODES:
            if not gfx:
                held = 0
            gfx, fg = True, CODES[name[1:]]
        elif name == "release":
            hold = False
    return out + [(art.BLACK,) * 6] * (40 - len(out))


def wanted(p: art.Picture) -> list[tuple]:
    return [tuple(p.sixels[y][c * 2 + x] for x, y, _ in art._BITS) for c in range(40)]


class ArtTest(unittest.TestCase):
    def test_one_colour_needs_one_attribute(self):
        p = art.Picture(1)
        for y in range(3):
            for x in range(4, 10):
                p.sixels[y][x] = art.RED
        row = p.markup()[0]
        self.assertEqual(decode(row), wanted(p))

    def test_two_colours_meet_without_a_gap(self):
        p = art.Picture(1)
        for y in range(3):
            for x in range(4, 20):
                p.sixels[y][x] = art.RED if x < 12 else art.YELLOW
        row = p.markup()[0]
        self.assertEqual(decode(row), wanted(p))

    def test_a_diagonal_between_two_colours(self):
        p = art.Picture(1)
        for y in range(3):
            for x in range(4, 30):
                p.sixels[y][x] = art.RED if x + y < 16 else art.GREEN
        row = p.markup()[0]
        self.assertEqual(decode(row), wanted(p))
        self.assertLessEqual(cells(row), 40)

    def test_text_gets_its_colours(self):
        p = art.Picture(1)
        for y in range(3):
            for x in range(2, 40):
                p.sixels[y][x] = art.BLUE
        p.text(0, 4, "HEI", art.YELLOW, art.BLUE)
        row = p.markup()[0]
        self.assertIn("{yellow}HEI", row)
        self.assertLess(row.index("{newbg}"), row.index("{yellow}"))

    def test_plain_rows_keep_a_black_background(self):
        p = art.Picture(1)
        for y in range(3):
            for x in range(4, 20):
                p.sixels[y][x] = art.WHITE
        p.plain_rows.add(0)
        self.assertNotIn("{newbg}", p.markup()[0])

    def test_text_with_no_room_for_attributes_fails(self):
        p = art.Picture(1)
        p.text(0, 0, "X", art.RED)
        with self.assertRaises(ValueError):
            p.markup()


if __name__ == "__main__":
    unittest.main()
