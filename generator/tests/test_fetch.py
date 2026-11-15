import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

from teletext_pages import schedule

ITEM = {
    "starttime": "2026-11-15T17:00:00+01:00",
    "endtime": "2026-11-15T18:00:00+01:00",
    "default_name": "x",
    "video": {"name": "Israel - Guds folk - del 2", "organization": {"name": "Agape"}},
    "is_live": False,
}


class Api(BaseHTTPRequestHandler):
    fail = 0  # fail this many requests first

    def do_GET(self):
        # Like frikanalen.tv: the path with a trailing slash is a 404.
        if Api.fail:
            Api.fail -= 1
            self.send_error(503)
            return
        if not self.path.startswith("/api/scheduleitems?"):
            self.send_error(404)
            return
        body = json.dumps({"count": 1, "results": [ITEM]}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


class FetchTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = HTTPServer(("127.0.0.1", 0), Api)
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()
        cls.api = f"http://127.0.0.1:{cls.server.server_port}/api/"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()

    def test_fetch(self):
        items = schedule.fetch(self.api)
        self.assertEqual(items[0].title, "Israel - Guds folk - del 2")
        self.assertEqual(items[0].organization, "Agape")

    def test_one_failure_is_retried(self):
        Api.fail = 1
        self.assertEqual(len(schedule.fetch(self.api)), 1)
        Api.fail = 2
        with self.assertRaises(OSError):
            schedule.fetch(self.api)
        Api.fail = 0
