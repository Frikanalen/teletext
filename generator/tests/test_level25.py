import os
import unittest
from datetime import datetime

from teletext_pages import level25 as L
from teletext_pages import sky
from teletext_pages.index import (
    MAX_TRIPLETS,
    SPRITE_FRAMES,
    Entry,
    IndexConfig,
    animated_page,
    assets,
    logo,
    page,
)
from teletext_pages.schedule import OSLO


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


WET = [(False, False), (True, False), (False, True)]


class IndexLogoTest(unittest.TestCase):
    def test_logo_fits_drcs_and_x26(self):
        g = logo()
        self.assertLessEqual(len(g.tiles), 3 * L.TILES_PER_DRCS_SUBPAGE)
        self.assertEqual(len(g.drcs_subpages()), -(-len(g.tiles) // L.TILES_PER_DRCS_SUBPAGE))
        # Each row draws from one DRCS subpage, named by its subcode (1, 2, ...).
        for row in range(8, 13):
            subs = {t // L.TILES_PER_DRCS_SUBPAGE for (r, _), (t, _) in g.cells.items() if r == row}
            self.assertEqual(len(subs), 1, row)
        modes = [d for a, m, d in g.x26() if m == 0x18]
        self.assertTrue(modes)
        self.assertTrue(all(0x41 <= d <= 0x43 for d in modes))

    def test_every_sky_fits_x26(self):
        cfg = IndexConfig(entries=[Entry(str(n), "Side") for n in (200, 300, 400, 888)])
        skies = [sky.Weather(c, rain=r, snow=s, thunder=r) for c in (0, 2, 3) for r, s in WET]
        skies.append(sky.Weather(1, fog=True))
        shared = {
            "objects": {"logo": [56, 18, 0]},
            "glyphs": {g["name"]: n for n, g in enumerate(assets()["glyphs"])},
        }
        for hour in (3, 9, 15):
            for weather in skies:
                at = sky.look(datetime(2026, 11, 15, hour, 20, tzinfo=OSLO), weather)
                for held in (None, shared):
                    p = page(cfg, None, None, cfg.entries, at, held)
                    # Room left for ttxd's Æ/Ø substitutions (16 packets of 13, less
                    # a terminator).
                    self.assertLessEqual(len(p["x26"]), MAX_TRIPLETS, (hour, weather))
                    self.assertEqual([k["packet"] for k in p["packets"]], [27, 28])
                    # Rows and columns in rising order, as a decoder reads them.
                    position = (0, -1)
                    for address, mode, _ in p["x26"]:
                        if address >= 40 and mode != 0x12:
                            position = (24 if address == 40 else address - 40, -1)
                        elif address < 40:
                            self.assertGreaterEqual(address, position[1])
                            position = (position[0], address)

    def test_shared_logo_and_glyphs(self):
        cfg = IndexConfig(entries=[Entry(str(n), "Side") for n in (200, 300, 400, 888)])
        glyphs = {g["name"]: n for n, g in enumerate(assets()["glyphs"])}
        shared = {"objects": {"logo": [56, 18, 0]}, "glyphs": glyphs}
        night = sky.look(datetime(2026, 11, 15, 3, 0, tzinfo=OSLO))
        own = page(cfg, None, None, cfg.entries, night)["x26"]
        x26 = page(cfg, None, None, cfg.entries, night, shared)["x26"]
        # The logo is placed, not carried: the active position, then the object.
        at = x26.index((56, 0x12, 0))
        self.assertEqual(x26[at - 1], (48, 0x04, 2))
        self.assertLess(len(x26), len(own) - 80)
        # No characters from the page's own DRCS page are left; stars and the slanted
        # ends of the tabs and chips come from the shared glyphs.
        drawn = [d for _, mode, d in x26 if mode == 0x0D]
        self.assertTrue(all(d < 0x40 for d in drawn))
        stars = {glyphs[f"star-{n}"] for n in range(3)}
        self.assertGreater(sum(d in stars for d in drawn), 5)
        slants = {n for name, n in glyphs.items() if name.startswith("slant")}
        self.assertEqual(sum(d in slants for d in drawn), 2 + 2 * 4)  # tabs, chip ends

    def test_sky_sprites(self):
        cfg = IndexConfig(entries=[Entry(str(n), "Side") for n in (200, 300, 400, 888)])
        glyphs = {g["name"]: n for n, g in enumerate(assets()["glyphs"])}
        shared = {"objects": {"logo": [56, 18, 0]}, "glyphs": glyphs}
        snow = sky.look(datetime(2026, 11, 15, 19, 0, tzinfo=OSLO), sky.Weather(3, snow=True))
        subpage, animation = animated_page(cfg, None, None, cfg.entries, snow, shared)
        # Frames that change no rows, each with the page's triplets a step on; the
        # page itself starts as the first frame.
        self.assertEqual(animation["frames"], [{}] * SPRITE_FRAMES)
        self.assertEqual(animation["frame_ms"], cfg.sprite_ms)
        frames = animation["x26"]
        self.assertEqual(subpage["x26"], frames[0])
        self.assertGreater(len({tuple(f) for f in frames}), 2)
        flakes = {glyphs[f"snow-{n}"] for n in range(3)}
        blanked = 0
        for f in frames:
            self.assertLessEqual(len(f), MAX_TRIPLETS)
            self.assertGreater(sum(m == 0x0D and d in flakes for _, m, d in f), 5)
            blanked += sum((m, d) == (0x01, 0x20) for _, m, d in f)
            # Only the sky's sprites differ from frame to frame.
            still = [t for t in f if not (t[1] == 0x0D and t[2] in flakes) and t[1:] != (1, 0x20)]
            self.assertEqual(
                still,
                [
                    t
                    for t in frames[0]
                    if not (t[1] == 0x0D and t[2] in flakes) and t[1:] != (1, 0x20)
                ],
            )
        # Some flakes leave their cell for the one below, and blank their own.
        self.assertGreater(blanked, 0)
        # Without sprites, or without the glyphs for them, the sky stands still.
        cfg.sprites = False
        self.assertIsNone(animated_page(cfg, None, None, cfg.entries, snow, shared)[1])
        cfg.sprites = True
        old = {"objects": shared["objects"], "glyphs": {"snow": 0, "star-0": 1}}
        self.assertIsNone(animated_page(cfg, None, None, cfg.entries, snow, old)[1])

    def test_logo_as_an_object(self):
        g = logo()
        obj = g.object_x26()
        # Same cells as on the page, counted from the logo's top left corner.
        self.assertEqual(sum(mode == 0x0D for _, mode, _ in obj), len(g.cells))
        self.assertEqual(obj[0][1], 0x18)  # the DRCS subpage of the first row
        self.assertIn((41, 0x04, 0), obj)
        self.assertTrue(all(a < 40 or 40 <= a <= 44 for a, _, _ in obj))
        for part in assets()["glyphs"]:
            self.assertEqual([len(r) for r in part["rows"]], [12] * 10, part["name"])

    def test_sky_and_keys(self):
        cfg = IndexConfig(entries=[Entry("200", "Sendeplan")])
        at = sky.look(datetime(2026, 11, 15, 15, 35, tzinfo=OSLO))
        x26 = page(cfg, None, None, cfg.entries, at)["x26"]
        # Each sky row: its colour as the row colour and as the background.
        self.assertEqual(
            x26[:6], [(41, 1, 24), (41, 4, 0), (0, 3, 24), (42, 1, 25), (42, 4, 0), (0, 3, 25)]
        )
        # The red key keeps the standard red from colour table 0.
        row23 = x26.index((63, 0x04, 0))
        self.assertEqual(x26[row23 + 1 : row23 + 3], [(1, 0x00, 1), (2, 0x03, 1)])
