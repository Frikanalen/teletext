import io
import json
import unittest
import urllib.error
from datetime import UTC, datetime

from teletext_pages import art, sky, weather
from teletext_pages.schedule import OSLO


class SunTest(unittest.TestCase):
    def test_position_over_oslo(self):
        # Midsummer noon: 90 - 59.91 + 23.44 degrees up, due south.
        elevation, azimuth = sky.sun_position(
            datetime(2026, 6, 21, 11, 17, tzinfo=UTC), 59.91, 10.75
        )
        self.assertAlmostEqual(elevation, 53.5, delta=0.3)
        self.assertAlmostEqual(azimuth, 180, delta=2)
        # Mid-November: the sun sets around 15:50 in the south-west.
        elevation, azimuth = sky.sun_position(
            datetime(2026, 11, 15, 15, 50, tzinfo=OSLO), 59.91, 10.75
        )
        self.assertAlmostEqual(elevation, -0.8, delta=1.0)
        self.assertTrue(225 < azimuth < 240)

    def test_moon_phase(self):
        # Full moon on 24 November 2026.
        self.assertAlmostEqual(
            sky.moon_phase(datetime(2026, 11, 24, 15, tzinfo=UTC)), 0.5, delta=0.03
        )

    def test_look_changes_only_with_the_picture(self):
        a = sky.look(datetime(2026, 11, 15, 12, 0, 0, tzinfo=OSLO))
        b = sky.look(datetime(2026, 11, 15, 12, 0, 20, tzinfo=OSLO))
        self.assertEqual(a, b)
        self.assertTrue(sky.look(datetime(2026, 11, 15, 9, tzinfo=OSLO)).morning)


class DrawTest(unittest.TestCase):
    def picture(self, when: str, w: sky.Weather = sky.CLEAR) -> art.Picture:
        pic = art.Picture(sky.ROWS)
        sky.draw(pic, sky.look(datetime.fromisoformat(when).replace(tzinfo=OSLO), w))
        return pic

    def colours(self, pic: art.Picture) -> set[int]:
        return {c for line in pic.sixels[: sky.HORIZON] for c in line}

    def test_day_night_and_weather(self):
        self.assertEqual(self.colours(self.picture("2026-06-21 13:00")), {art.BLACK, art.YELLOW})
        dusk = self.colours(self.picture("2026-11-15 15:35"))
        self.assertEqual(dusk, {art.BLACK, art.YELLOW, art.RED, art.MAGENTA})
        self.assertEqual(self.colours(self.picture("2026-11-15 03:00")), {art.BLACK, art.WHITE})
        rain = self.picture("2026-06-21 13:00", sky.Weather(3, rain=True))
        self.assertIn(art.CYAN, self.colours(rain))
        night_snow = self.colours(self.picture("2026-11-15 03:00", sky.Weather(3, snow=True)))
        self.assertEqual(night_snow, {art.BLACK, art.BLUE, art.WHITE})
        for pic in (rain, self.picture("2026-11-15 15:35")):
            # Column 0 can only be black, and the horizon is always there.
            self.assertTrue(all(line[0] == line[1] == art.BLACK for line in pic.sixels))
            self.assertEqual(set(pic.sixels[sky.HORIZON][sky.LEFT : sky.RIGHT]), {art.CYAN})

    def test_gradient(self):
        night = sky.gradient(sky.look(datetime(2026, 11, 15, 3, tzinfo=OSLO)))
        day = sky.gradient(sky.look(datetime(2026, 6, 21, 13, tzinfo=OSLO)))
        grey = sky.gradient(sky.look(datetime(2026, 6, 21, 13, tzinfo=OSLO), sky.Weather(3)))
        self.assertEqual(len(night), sky.STEPS)
        self.assertLess(sum(night[-1]), sum(day[0]))
        self.assertLess(max(grey[0]) - min(grey[0]), max(day[0]) - min(day[0]))


class WeatherTest(unittest.TestCase):
    def test_symbols(self):
        W = sky.Weather
        cases = {
            "clearsky_day": W(0),
            "fair_night": W(1),
            "partlycloudy_day": W(2),
            "cloudy": W(3),
            "fog": W(3, fog=True),
            "lightrainshowers_day": W(2, rain=True),
            "heavyrainandthunder": W(3, rain=True, thunder=True),
            "snow": W(3, snow=True),
            "sleet": W(3, rain=True, snow=True),
        }
        for symbol, want in cases.items():
            self.assertEqual(sky.weather_from_symbol(symbol), want, symbol)

    def reply(self, symbol: str):
        period = {"next_1_hours": {"summary": {"symbol_code": symbol}}}
        reply = io.BytesIO(json.dumps({"properties": {"timeseries": [{"data": period}]}}).encode())
        reply.headers = {"Last-Modified": "Sun, 15 Nov 2026 17:00:00 GMT"}
        return reply

    def test_forecast_is_kept_and_survives_errors(self):
        now = [0.0]
        calls = []

        def opener(req, timeout):
            calls.append(req)
            if len(calls) == 2:
                raise urllib.error.HTTPError(req.full_url, 304, "Not Modified", {}, None)
            if len(calls) >= 3:
                raise OSError("down")
            return self.reply("rain")

        f = weather.Forecast(59.91, 10.75, clock=lambda: now[0], opener=opener)
        self.assertEqual(f.now(), sky.Weather(3, rain=True))
        self.assertIn("frikanalen", calls[0].get_header("User-agent"))
        self.assertEqual(f.now(), sky.Weather(3, rain=True))
        self.assertEqual(len(calls), 1)  # kept for ten minutes
        now[0] = 700
        self.assertEqual(f.now(), sky.Weather(3, rain=True))  # 304: unchanged
        self.assertEqual(calls[1].get_header("If-modified-since"), "Sun, 15 Nov 2026 17:00:00 GMT")
        now[0] = 1400
        with self.assertRaises(OSError):
            f.now()
        self.assertEqual(f.weather, sky.Weather(3, rain=True))
        # After three hours without an answer it stops showing that rain.
        now[0] = 700 + weather.STALE_SECS + 1
        with self.assertRaises(OSError):
            f.now()
        self.assertEqual(f.weather, sky.Weather())


if __name__ == "__main__":
    unittest.main()
