import unittest

from teletext_pages.mosaic import Canvas, code, lettering


class MosaicTest(unittest.TestCase):
    def test_code(self):
        self.assertEqual(code(set()), 0x20)
        self.assertEqual(code({(0, 0), (1, 0), (0, 1), (1, 1), (0, 2), (1, 2)}), 0x7F)
        self.assertEqual(code({(1, 2)}), 0x60)

    def test_colour_attributes_go_in_blank_cells(self):
        c = Canvas(6, 1)
        c.draw(2, 0, ["##", "##", "##"], "red").draw(8, 0, ["#"], "green")
        self.assertEqual(c.markup(), ["{gred}{x:7f} {ggreen}{x:21}"])

    def test_no_room_for_an_attribute(self):
        c = Canvas(3, 1).draw(0, 0, ["#"], "red")
        with self.assertRaises(ValueError):
            c.markup()
        c = Canvas(3, 1).draw(2, 0, ["##"], "red").draw(4, 0, ["#"], "green")
        with self.assertRaises(ValueError):
            c.markup()

    def test_text_and_filled(self):
        c = Canvas(10, 1).draw(2, 0, ["#"], "white").text(0, 3, "{cyan}AB")
        self.assertEqual(c.markup(), ["{gwhite}{x:21} {cyan}AB"])
        self.assertEqual(c.filled(), {(0, 1), (0, 3), (0, 4), (0, 5)})

    def test_lettering(self):
        art = lettering("FRI")
        self.assertEqual(len(art), 9)
        self.assertEqual(len(art[0]), 5 + 1 + 5 + 1 + 2)
