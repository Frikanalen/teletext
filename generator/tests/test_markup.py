import unittest

from teletext_pages.markup import COLS, cells, check, fit, leader, plain, preview, spread, wrap


class MarkupTest(unittest.TestCase):
    def test_plain_strips_markup_and_typography(self):
        self.assertEqual(plain("  {red}Hei – «du»\n…  "), '(red)Hei - "du" ...')

    def test_cells_counts_attributes_as_one(self):
        self.assertEqual(cells("{blue}{newbg}Nå"), 4)
        self.assertEqual(cells("{{x}}"), 3)

    def test_fit_cuts_at_words(self):
        self.assertEqual(fit("kort", 10), "kort")
        self.assertEqual(
            fit("Debatt om kommunestyrevalget i Oslo", 30), "Debatt om kommunestyrevalget."
        )
        # A word boundary that would throw away too much is ignored.
        self.assertEqual(fit("Debatt om kommunestyrevalget", 20), "Debatt om kommunest.")

    def test_wrap(self):
        self.assertEqual(wrap("kort", 10, 2), ["kort"])
        self.assertEqual(wrap("en to tre fire fem", 9, 2), ["en to tre", "fire fem"])
        self.assertEqual(wrap("en to tre fire fem seks", 9, 2), ["en to tre", "fire fem."])
        self.assertEqual(wrap("abcdefghijkl", 5, 2), ["abcde", "fghi."])
        self.assertEqual(wrap("", 5, 2), [])

    def test_spread_right_aligns(self):
        row = spread("{white}A", "B{red}")
        self.assertEqual(cells(row), COLS)
        self.assertTrue(row.endswith("B{red}"))
        self.assertEqual(spread("x", ""), "x")

    def test_leader(self):
        row = leader("Sendeplan", "200")
        self.assertEqual(cells(row), COLS)
        self.assertEqual(preview(row), " Sendeplan " + "." * 25 + " 200")
        self.assertEqual(cells(leader("x" * 60, "200")), COLS)

    def test_check(self):
        check(["x" * 40])
        with self.assertRaises(ValueError):
            check(["{red}" + "x" * 40])
        with self.assertRaises(ValueError):
            check([""] * 25)
