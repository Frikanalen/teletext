import unittest
from datetime import datetime

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


class IndexTest(unittest.TestCase):
    def test_directory_lists_live_and_always(self):
        shown = [e.page for e in directory(CFG, {"100", "200"})]
        self.assertEqual(shown, ["200", "888"])
        self.assertEqual([e.page for e in directory(CFG, None)], ["200", "300", "888"])

    def test_render(self):
        at = T("2026-11-15T18:30:00+01:00")
        rows = render(CFG, *now_and_next(ITEMS, at), directory(CFG, {"200"}))
        self.assertEqual(len(rows), ROWS)
        self.assertTrue(all(cells(r) <= 40 for r in rows))
        text = [preview(r).rstrip() for r in rows]
        self.assertEqual(text[0], "          frikanalen.tv        TEKST-TV")
        self.assertTrue(text[4].endswith("Sivilsamfunnets kanal"))
        self.assertIn("{gwhite}", rows[2])
        self.assertTrue(text[7].startswith("   NÅ PÅ FRIKANALEN"))
        self.assertTrue(rows[7].endswith("{flash}DIREKTE "))
        self.assertEqual(text[8], " 18:00 Debatt: Hva skjer med kanalen? En")
        self.assertEqual(text[9], "       svært lang tittel som må kuttes")
        # Outside text cannot inject markup.
        self.assertEqual(text[10], "       Frikanalen (red)")
        self.assertEqual(text[13], " 19:04 Været")
        self.assertEqual(text[14], "")
        self.assertTrue(text[17].startswith("   SIDER"))
        self.assertEqual(text[18], " Sendeplan " + "." * 25 + " 200")
        self.assertEqual(text[19], " Teksting " + "." * 26 + " 888")
        self.assertEqual(rows[5].count("{g"), 3)  # three bars
        self.assertEqual(rows[22].count("{g"), 4)  # four buttons
        self.assertEqual(rows[23], "{red}Sendeplan{green}Teksting")

    def test_render_empty_schedule(self):
        rows = render(CFG, None, None, [])
        self.assertEqual(preview(rows[8]).strip(), "Ingen program i sendeplanen")
        self.assertEqual(len(rows), ROWS)


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
