import unittest
from datetime import datetime

from teletext_pages import sky
from teletext_pages.index import Entry, IndexConfig, directory, fastext, render
from teletext_pages.markup import ROWS, cells, preview
from teletext_pages.schedule import next_change, now_and_next, parse

T = datetime.fromisoformat

ITEMS = parse(
    [
        {
            "starttime": "2026-11-15T19:04:00+01:00",
            "endtime": "2026-11-15T19:30:00+01:00",
            "default_name": "Fallback",
            "video": {"name": "Været", "organization": None},
            "is_live": False,
        },
        {
            "starttime": "2026-11-15T18:00:00+01:00",
            "endtime": "2026-11-15T19:04:00+01:00",
            "default_name": "x",
            "video": {
                "name": "Debatt: Hva skjer med kanalen? En svært lang tittel som må kuttes",
                "organization": {"name": "Frikanalen {red}"},
            },
            "is_live": True,
        },
        {
            "starttime": "2026-11-15T20:00:00+01:00",
            "endtime": "2026-11-15T21:00:00+01:00",
            "default_name": "Uten video",
            "video": None,
            "is_live": False,
        },
    ]
)

CFG = IndexConfig(
    entries=[
        Entry("100", "Denne siden"),
        Entry("200", "Sendeplan"),
        Entry("300", "Oppslag"),
        Entry("888", "Teksting", always=True),
    ]
)


class ScheduleTest(unittest.TestCase):
    def test_parse_sorts_and_falls_back(self):
        self.assertEqual([p.title[:6] for p in ITEMS], ["Debatt", "Været", "Uten v"])
        self.assertEqual(ITEMS[0].organization, "Frikanalen {red}")
        self.assertIsNone(ITEMS[1].organization)

    def test_now_and_next(self):
        cur, nxt = now_and_next(ITEMS, T("2026-11-15T18:30:00+01:00"))
        self.assertEqual((cur.title[:6], nxt.title), ("Debatt", "Været"))
        cur, nxt = now_and_next(ITEMS, T("2026-11-15T19:45:00+01:00"))
        self.assertIsNone(cur)
        self.assertEqual(nxt.title, "Uten video")
        self.assertEqual(now_and_next(ITEMS, T("2026-11-15T22:00:00+01:00")), (None, None))

    def test_next_change(self):
        at = T("2026-11-15T19:30:00+01:00")
        self.assertEqual(next_change(ITEMS, at), T("2026-11-15T20:00:00+01:00"))


DUSK = sky.look(T("2026-11-15T15:35:00+01:00"))


class IndexTest(unittest.TestCase):
    def test_directory_lists_live_and_always(self):
        shown = [e.page for e in directory(CFG, {"100", "200"})]
        self.assertEqual(shown, ["200", "888"])
        self.assertEqual([e.page for e in directory(CFG, None)], ["200", "300", "888"])

    def test_render(self):
        at = T("2026-11-15T18:30:00+01:00")
        rows = render(CFG, *now_and_next(ITEMS, at), directory(CFG, {"200"}), DUSK)
        self.assertEqual(len(rows), ROWS)
        self.assertTrue(all(cells(r) <= 40 for r in rows))
        text = [preview(r).rstrip() for r in rows]
        self.assertTrue(text[0].startswith(" frikanalen.tv"))
        self.assertTrue(text[0].endswith(" TEKST-TV"))
        self.assertIn("{gyellow}", rows[1])  # the sun
        self.assertIn("{gcyan}", rows[7])  # the horizon
        self.assertIn("{gwhite}", rows[9])  # the lettering
        self.assertEqual(text[11].strip(), "Sivilsamfunnets kanal")
        self.assertEqual(text[12][:19], "   NÅ PÅ FRIKANALEN")
        self.assertRegex(rows[12], r"\{flash\}.*DIREKTE")
        self.assertEqual(text[13], " 18:00  Debatt: Hva skjer med kanalen?")
        self.assertEqual(text[14], "        En svært lang tittel som må.")
        # Outside text cannot inject markup.
        self.assertEqual(text[15], "        Frikanalen (red)")
        self.assertEqual(text[17][:11], "   DERETTER")
        self.assertEqual(text[18], " 19:04  Været")
        self.assertEqual(text[19], "")
        self.assertIn("200", text[22])
        self.assertIn("888", text[22])
        self.assertEqual(rows[23], "{red}Sendeplan{green}Teksting")

    def test_render_empty_schedule(self):
        rows = render(CFG, None, None, [], DUSK)
        self.assertEqual(preview(rows[13]).strip(), "Ingen program i sendeplanen")
        self.assertEqual(len(rows), ROWS)
        self.assertEqual(rows[22:], ["", ""])

    def test_logo_rows_keep_a_black_background(self):
        # A Level 2.5 decoder draws the real logo over them.
        rows = render(CFG, None, None, [], DUSK)
        for row in rows[7:12]:
            self.assertNotIn("{newbg}", row)


class FastextTest(unittest.TestCase):
    def test_links(self):
        # Decode as libzvbi does (packet.c, unham_page_link).
        ham = [
            0x15,
            0x02,
            0x49,
            0x5E,
            0x64,
            0x73,
            0x38,
            0x2F,
            0xD0,
            0xC7,
            0x8C,
            0x9B,
            0xA1,
            0xB6,
            0xFD,
            0xEA,
        ]
        raw = fastext(CFG, [Entry("200", "A"), Entry("888", "B")])["raw"]
        self.assertEqual(len(raw), 39)
        n = [ham.index(b) for b in raw[:37]]
        pages = []
        for i in range(6):
            b1 = n[6 * i] | n[6 * i + 1] << 4
            b2 = n[6 * i + 2] | n[6 * i + 3] << 4
            b3 = n[6 * i + 4] | n[6 * i + 5] << 4
            m = ((b3 >> 5) & 6) + (b2 >> 7)
            pages.append(f"{(1 ^ m) or 8}{b1:02X}")
        self.assertEqual(pages, ["200", "888", "8FF", "8FF", "8FF", "100"])
        self.assertEqual(n[36] >> 3, 1)  # show row 24
