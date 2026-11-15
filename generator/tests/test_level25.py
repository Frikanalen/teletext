import os
import unittest

from teletext_pages import level25 as L
from teletext_pages.index import IndexConfig, enhancements, logo


def unpack(triplets, fields):
    """Read fields back the way libzvbi's get_bits does: first bit first."""
    word = 0
    for i, (a, m, d) in enumerate(triplets):
        word |= (a | m << 6 | d << 11) << (18 * i)
    out = []
    for bits in fields:
        out.append(word & ((1 << bits) - 1))
        word >>= bits
    return out


class BitsTest(unittest.TestCase):
    def test_round_trip_across_triplets(self):
        t = L.Bits().put(5, 4).put(0x3FFF, 14).put(1, 1).triplets(2)
        self.assertEqual(unpack(t, [4, 14, 1]), [5, 0x3FFF, 1])

    def test_overflow(self):
        with self.assertRaises(ValueError):
            L.Bits().put(0, 235).triplets()


class PacketTest(unittest.TestCase):
    def test_palette(self):
        t = L.x28_palette([L.hex_rgb("#ff8000")])
        self.assertEqual(len(t), 13)
        f = unpack(t, [4, 3, 7, 7, 1, 1, 1, 4] + [12] * 16 + [5, 5, 1, 3])
        self.assertEqual(f[:4], [0, 0, 2, 2])  # LOP, coding, Swedish G0 x2
        ink = f[8]
        self.assertEqual((ink & 15, ink >> 4 & 15, ink >> 8), (1, 1, 2))  # #0f1926
        self.assertEqual(f[8 + 8], 15 | 8 << 4 | 0 << 8)  # table 3 entry 0: #ff8000
        self.assertEqual(f[-4:], [16, 16, 1, 6])

    def test_drcs_coding(self):
        f = unpack(L.x28_drcs(), [4, 3, 11] + [4] * 48)
        self.assertEqual(f[0], 5)  # normal DRCS page
        self.assertEqual(set(f[3:]), {0})  # all 12x10x1

    def test_drcs_link(self):
        f = unpack(L.x27_drcs_link("10F"), [18] * 8)
        pages = [f[2 * i] for i in range(4)]
        for slot, word in enumerate(pages):
            self.assertEqual(word & 3, slot)  # link function
        # POP (1) and DRCS (3) slots point at page x0F, the others at xFF.
        self.assertEqual([(w >> 7) & 0xFF for w in pages], [0xFF, 0x0F, 0xFF, 0x0F])
        with self.assertRaises(ValueError):
            L.x27_drcs_link("12F")

    def test_tile_bytes(self):
        bits = [[x == 0 or x == 11 for x in range(12)] for _ in range(10)]
        t = L.tile_bytes(bits)
        self.assertEqual(len(t), 20)
        self.assertEqual(t[:2], (0x40 | 0x20, 0x40 | 0x01))


class GraphicTest(unittest.TestCase):
    def test_cut_share_and_draw(self):
        red, white = (255, 0, 0), (255, 255, 255)
        # 36x10 px: a red block, an empty cell, then the same block in white.
        px = [[red] * 12 + [None] * 12 + [white] * 12 for _ in range(10)]
        g = L.make_graphic(L.Picture(px), 5, 3, L.BRAND, L.BRAND[0])
        self.assertEqual(len(g.tiles), 1)  # identical shapes share a tile
        self.assertEqual(g.cells, {(5, 3): (0, 17), (5, 5): (0, 23)})
        self.assertEqual(
            g.x26(blank={(5, 4)}),
            [
                (45, 0x04, 0),
                (3, 0x00, 17),
                (3, 0x0D, 0x40),
                (4, 0x09, 0x20),
                (5, 0x00, 23),
                (5, 0x0D, 0x40),
            ],
        )
        sub = g.drcs_subpages()
        self.assertEqual(len(sub), 1)
        self.assertEqual(len(sub[0]["raw_rows"]["1"]), 40)
        self.assertEqual(sub[0]["packets"][0]["designation"], 3)

    def test_ppm(self):
        path = os.path.join(os.path.dirname(L.__file__), "assets", "wordmark.ppm")
        pic = L.read_ppm(path, L.BRAND[0])
        self.assertEqual((pic.width, pic.height), (355, 27))


class IndexLogoTest(unittest.TestCase):
    def test_logo_fits_drcs_and_x26(self):
        g = logo()
        self.assertLessEqual(len(g.tiles), 3 * L.TILES_PER_DRCS_SUBPAGE)
        self.assertEqual(len(g.drcs_subpages()), -(-len(g.tiles) // L.TILES_PER_DRCS_SUBPAGE))
        # Each row draws from one DRCS subpage, named by its subcode (1, 2, ...).
        for row in range(1, 6):
            subs = {t // L.TILES_PER_DRCS_SUBPAGE for (r, _), (t, _) in g.cells.items() if r == row}
            self.assertEqual(len(subs), 1, row)
        modes = [d for a, m, d in g.x26() if m == 0x18]
        self.assertTrue(modes)
        self.assertTrue(all(0x41 <= d <= 0x43 for d in modes))
        e = enhancements(IndexConfig())
        # The colour key buttons (rows 23-24) use the standard colours from table 0.
        x26 = e["x26"]
        row23 = x26.index((63, 0x04, 0))
        self.assertEqual(
            x26[row23 + 1 : row23 + 5], [(1, 0, 1), (11, 0, 2), (21, 0, 3), (31, 0, 6)]
        )
        # Room left for ttxd's Æ/Ø substitutions (16 packets of 13, less a terminator).
        self.assertLess(len(e["x26"]), 160)
        self.assertEqual([p["packet"] for p in e["packets"]], [27, 28])
