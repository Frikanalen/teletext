import io
import json
import unittest
import urllib.error
from datetime import UTC, datetime

from teletext_pages import art, astro, sky, weather
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


class StarsTest(unittest.TestCase):
    LAT, LON = 59.91, 10.75

    def test_positions(self):
        at = datetime(2026, 11, 16, 1, 0, tzinfo=UTC)
        stars = {
            name: astro.star(ra, dec, at, self.LAT, self.LON) for name, ra, dec, *_ in astro.STARS
        }
        # The pole star stands as high as the latitude, due north.
        self.assertAlmostEqual(stars["Polaris"][0], self.LAT, delta=1)
        self.assertTrue(stars["Polaris"][1] < 3 or stars["Polaris"][1] > 357)
        # Checked against an ephemeris (PyEphem) for this place and time.
        want = {
            "sirius": (11.52, 159.78),
            "betelgeuse": (37.13, 169.67),
            "moon": (-37.48, 296.48),
            "Jupiter": (22.94, 102.79),
            "Saturn": (9.71, 254.79),
        }
        got = {
            "sirius": stars["Sirius"],
            "betelgeuse": stars["Betelgeuse"],
            "moon": astro.moon(at, self.LAT, self.LON),
            "Jupiter": astro.planet("Jupiter", at, self.LAT, self.LON),
            "Saturn": astro.planet("Saturn", at, self.LAT, self.LON),
        }
        for name, (altitude, azimuth) in want.items():
            self.assertAlmostEqual(got[name][0], altitude, delta=0.6, msg=name)
            self.assertAlmostEqual(got[name][1], azimuth, delta=0.8, msg=name)

    def test_orion_in_the_window(self):
        at = sky.look(datetime(2026, 11, 16, 2, 0, tzinfo=OSLO))
        by_colour = {}
        for b in at.bodies:
            by_colour.setdefault((b.kind, b.colour), []).append((b.x, b.y))
        # Sirius, the one very bright bluish-white star, is just east of south and low;
        # red Betelgeuse is higher and to its right, bluish Rigel below Betelgeuse.
        (sirius,) = by_colour[("star-0", art.WHITE)][-1:]
        (betelgeuse,) = by_colour[("star-0", art.RED)]
        (rigel,) = by_colour[("star-0", art.CYAN)]
        self.assertLess(sirius[0], sky.MIDDLE + 1)
        self.assertGreater(sirius[1], betelgeuse[1])
        self.assertGreater(betelgeuse[0], sirius[0])
        self.assertGreater(rigel[1], betelgeuse[1])
        # By day there are none, and they keep their places for five minutes.
        self.assertEqual(sky.look(datetime(2026, 6, 21, 13, tzinfo=OSLO)).bodies, ())
        self.assertEqual(at, sky.look(datetime(2026, 11, 16, 2, 4, 30, tzinfo=OSLO)))

    def test_moon_in_the_window(self):
        # A waxing crescent low in the south-west in the evening of 15 November.
        at = sky.look(datetime(2026, 11, 15, 18, 30, tzinfo=OSLO))
        self.assertGreater(at.moon_at[0], sky.MIDDLE)
        self.assertGreater(at.moon_at[1], 12)
        self.assertLess(at.moon, 0.5)
        # Below the horizon in the small hours.
        self.assertIsNone(sky.look(datetime(2026, 11, 16, 2, 0, tzinfo=OSLO)).moon_at)

    def test_aiming_the_view(self):
        lat, lon = self.LAT, self.LON
        # By day the view is to the south; a midsummer evening sun in the north-west
        # pulls it round just far enough to stay in the picture.
        self.assertEqual(sky.best_heading(datetime(2026, 11, 15, 12, tzinfo=OSLO), lat, lon), 180)
        evening = datetime(2026, 6, 21, 21, 0, tzinfo=OSLO)
        _, azimuth = sky.sun_position(evening, lat, lon)
        heading = sky.best_heading(evening, lat, lon)
        self.assertGreater(heading, 180)
        self.assertLessEqual(abs(azimuth - heading), sky.SUN_ROOM)
        # A February evening: Orion, whole, a little east of south.
        night = datetime(2027, 2, 10, 21, 0, tzinfo=OSLO)
        heading = sky.best_heading(night, lat, lon)
        self.assertTrue(150 <= heading <= 195, heading)
        names = {b.name for b in sky.look(night, heading=heading).bodies}
        self.assertTrue(names.issuperset(sky.FIGURES["Orion"]))
        # It stays where it is unless another view is clearly better.
        self.assertEqual(sky.best_heading(night, lat, lon, current=heading + 15), heading + 15)
        self.assertTrue(150 <= sky.best_heading(night, lat, lon, current=0) <= 195)

    def test_the_view_turns_a_step_at_a_time(self):
        self.assertEqual(sky.turn_towards(180, 90), 165)
        self.assertEqual(sky.turn_towards(345, 30), 0)  # the shorter way, through north
        self.assertEqual(sky.turn_towards(170, 180), 180)
        north = sky.look(datetime(2026, 11, 16, 2, 0, tzinfo=OSLO), heading=0)
        self.assertEqual(north.direction, "NORD")
        # The pole star is in the middle, as high as the latitude.
        polaris = next(b for b in north.bodies if b.name == "Polaris")
        self.assertLessEqual(abs(polaris.x - sky.MIDDLE), 1)
        self.assertLessEqual(abs(sky.HORIZON - 1 - polaris.y - 20), 1)

    def test_clouds_drift(self):
        def clouds(drift):
            pic = art.Picture(sky.ROWS)
            at = datetime(2026, 6, 21, 13, tzinfo=OSLO)
            sky.draw(pic, sky.look(at, sky.Weather(1, cover=1), drift=drift))
            return [x for line in pic.sixels[:20] for x, c in enumerate(line) if c == art.WHITE]

        still, moved = clouds(0), clouds(10)
        self.assertEqual(min(moved) - min(still), 10)
        self.assertEqual(clouds(0.9), still)  # in steps of two sixels
        self.assertEqual(clouds(sky.DRIFT_SPAN), still)  # and round again


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
        # A clear night: stars in white, bluish, yellow and red.
        night = self.colours(self.picture("2026-11-15 03:00"))
        self.assertEqual(night, {art.BLACK, art.WHITE, art.CYAN, art.YELLOW, art.RED})
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

    def test_details(self):
        data = {
            "instant": {
                "details": {
                    "cloud_area_fraction": 52.0,
                    "wind_speed": 8.0,
                    "wind_from_direction": 270.0,
                }
            },
            "next_1_hours": {
                "summary": {"symbol_code": "rain"},
                "details": {"precipitation_amount": 3.1},
            },
        }
        got = weather.parse({"properties": {"timeseries": [{"data": data}]}})
        # Half covered, raining hard, and a westerly blows the clouds to the left.
        self.assertEqual(got, sky.Weather(3, rain=True, cover=3, intensity=2, wind=-8))

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
