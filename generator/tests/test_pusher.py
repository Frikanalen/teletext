import unittest

from teletext_pages.main import Pusher
from teletext_pages.ttxd import TtxdError


class FakeTtxd:
    def __init__(self):
        self.calls = []
        self.fail = False
        self.old = False  # a ttxd that turns animations down
        self.animations = []

    def put_page(self, page, subpages, *, ttl_secs, flash=False, **more):
        if self.fail or (self.old and more.get("animation")):
            raise TtxdError("down")
        self.animations.append(more.get("animation"))
        self.calls.append(("put", page, flash, ttl_secs))

    def flash_rows(self, page, rows, subpage=0):
        if self.fail:
            raise TtxdError("down")
        self.calls.append(("flash", page, sorted(rows)))


class PusherTest(unittest.TestCase):
    def setUp(self):
        self.t = 0.0
        self.ttxd = FakeTtxd()
        self.p = Pusher(self.ttxd, "100", ttl_secs=600, refresh_secs=120, clock=lambda: self.t)

    def test_first_push_is_whole_page_with_flash(self):
        self.p.push(["a", "b"])
        self.assertEqual(self.ttxd.calls, [("put", "100", True, 600)])

    def test_few_changed_rows_are_flashed(self):
        self.p.push(["a", "b", "c"])
        self.t = 10
        self.p.push(["a", "B", "c"])
        self.t = 20
        self.p.push(["a", "B", "c"])  # unchanged: nothing sent
        self.assertEqual(self.ttxd.calls[1:], [("flash", "100", [2])])

    def test_many_changed_rows_resend_the_page(self):
        self.p.push(["a"] * 10)
        self.t = 10
        self.p.push(["b"] * 10)
        self.assertEqual(self.ttxd.calls[1], ("put", "100", True, 600))

    def test_refresh_renews_ttl_without_flash(self):
        self.p.push(["a"])
        self.t = 130
        self.p.push(["a"])
        self.assertEqual(self.ttxd.calls[1], ("put", "100", False, 600))

    def test_level25_change_resends_the_page(self):
        self.p.push({"rows": ["a"], "x26": [(41, 4, 0)]})
        self.t = 10
        self.p.push({"rows": ["b"], "x26": [(41, 4, 0)]})
        self.assertEqual(self.ttxd.calls[1], ("flash", "100", [1]))
        self.t = 20
        self.p.push({"rows": ["b"], "x26": [(42, 4, 0)]})
        self.assertEqual(self.ttxd.calls[2], ("put", "100", True, 600))

    def test_ticker_change_resends_the_page(self):
        tick = [{"row": 22, "text": "20:00 Nyheter "}]
        self.p.push(["a"], tick)
        self.p.push(["a"], tick)
        self.p.push(["a"], [{"row": 22, "text": "21:00 Film "}])
        self.assertEqual([c[0] for c in self.ttxd.calls], ["put", "put"])

    def test_animation_is_dropped_for_a_ttxd_that_refuses_it(self):
        anim = {"frame_ms": 500, "frames": [{}, {}], "x26": [[], [[0, 13, 1]]]}
        self.ttxd.old = True
        self.p.push(["a"], None, anim)
        self.p.push(["a"], None, anim)
        self.assertEqual(self.ttxd.animations, [None])
        # Offered again with the next refresh, and kept once ttxd takes it.
        self.ttxd.old = False
        self.t = 130
        self.p.push(["a"], None, anim)
        self.p.push(["a"], None, anim)
        self.assertEqual(self.ttxd.animations, [None, anim])

    def test_several_subpages(self):
        self.p.push((["a"], ["b"]))
        self.t = 10
        self.p.push((["a"], ["b"]))
        self.t = 20
        self.p.push((["a"], ["c"]))
        self.assertEqual([c[0] for c in self.ttxd.calls], ["put", "put"])

    def test_error_resends_whole_page(self):
        self.p.push(["a", "b"])
        self.ttxd.fail = True
        self.t = 10
        self.p.push(["a", "c"])
        self.ttxd.fail = False
        self.t = 20
        self.p.push(["a", "c"])
        self.assertEqual(self.ttxd.calls[-1], ("put", "100", True, 600))
