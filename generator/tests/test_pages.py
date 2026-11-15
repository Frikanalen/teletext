import unittest
from datetime import datetime, timedelta

from teletext_pages import pages
from teletext_pages.index import Entry, IndexConfig
from teletext_pages.markup import preview
from teletext_pages.schedule import OSLO, Programme

HOME = IndexConfig(
    entries=[Entry("200", "Sendeplan"), Entry("300", "Info"), Entry("888", "Teksting")]
)
NOW = datetime(2026, 11, 15, 21, 10, tzinfo=OSLO)


def programmes(count: int) -> list[Programme]:
    return [
        Programme(
            NOW + timedelta(minutes=35 * n - 10),
            NOW + timedelta(minutes=35 * n + 25),
            f"Program {n}",
            "Foreningen" if n % 2 else None,
            live=n == 0,
        )
        for n in range(count)
    ]


def text(subpage: dict) -> list[str]:
    return [preview(row) for row in subpage["rows"]]


class ScheduleTest(unittest.TestCase):
    def test_from_now_on_with_a_subpage_per_day(self):
        subs = pages.schedule_pages(pages.ScheduleConfig(), HOME, programmes(14), NOW, HOME.entries)
        first, second = text(subs[0]), text(subs[1])
        self.assertIn("I DAG søndag 15. november", first[5])
        self.assertIn("1/2", first[4])
        # The programme on air comes first, with DIREKTE under it.
        self.assertIn("21:00  Program 0", first[6])
        self.assertIn("DIREKTE", first[7])
        self.assertIn("Foreningen", first[9])
        # Six programmes start before midnight; the next day gets its own subpage.
        self.assertIn("23:55  Program 5", first[16])
        self.assertIn("I MORGEN mandag 16. november", second[5])
        self.assertIn("00:30  Program 6", second[6])
        # The keys lead back to the index and on to the other pages.
        self.assertIn("100", first[22])
        self.assertNotIn("200", first[22])
        self.assertEqual(subs[0]["packets"][0]["packet"], 27)

    def test_limits(self):
        cfg = pages.ScheduleConfig(per_page=3, max_subpages=2)
        subs = pages.schedule_pages(cfg, HOME, programmes(14), NOW, HOME.entries)
        self.assertEqual(len(subs), 2)
        self.assertIn("Program 3", text(subs[1])[6])
        empty = pages.schedule_pages(cfg, HOME, [], NOW, HOME.entries)
        self.assertIn("Ingen program", text(empty[0])[7])

    def test_level_1_only(self):
        home = IndexConfig(level25=False)
        (sub,) = pages.schedule_pages(pages.ScheduleConfig(), home, programmes(2), NOW, [])
        self.assertNotIn("x26", sub)


class InfoTest(unittest.TestCase):
    def test_default_text_fits_one_page(self):
        (sub,) = pages.info_pages(pages.InfoConfig(), HOME, HOME.entries)
        rows = text(sub)
        self.assertIn("OM FRIKANALEN", rows[5])
        self.assertIn("sivilsamfunnets", rows[6])
        self.assertIn("200", rows[22])
        self.assertNotIn("300", rows[22])

    def test_long_text_goes_on_more_subpages(self):
        cfg = pages.InfoConfig(sections=[pages.Section(f"Del {n}", "ord " * 100) for n in range(3)])
        subs = pages.info_pages(cfg, HOME, HOME.entries)
        self.assertEqual(len(subs), 3)
        self.assertIn("2/3", text(subs[1])[4])
        self.assertIn("DEL 1", text(subs[1])[5])
        # Outside text cannot bring markup along.
        cfg = pages.InfoConfig(sections=[pages.Section("{red}", "{flash}hei")])
        (sub,) = pages.info_pages(cfg, HOME, HOME.entries)
        self.assertNotIn("{flash}", sub["rows"][6])


if __name__ == "__main__":
    unittest.main()
