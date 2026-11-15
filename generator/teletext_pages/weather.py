"""The weather right now, from MET Norway's Locationforecast (api.met.no).

MET asks every client to identify itself in the User-Agent and not to ask more often
than the forecast changes, so :class:`Forecast` keeps what it has for ten minutes and
sends If-Modified-Since after that.
"""

import json
import time
import urllib.error
import urllib.request

from .sky import Weather, weather_from_symbol

URL = "https://api.met.no/weatherapi/locationforecast/2.0/compact?lat={lat:.2f}&lon={lon:.2f}"
USER_AGENT = "frikanalen-teletext/0.3 github.com/Frikanalen/teletext"
KEEP_SECS = 600
# A forecast this old says nothing about the sky outside; show a clear one instead.
STALE_SECS = 3 * 3600


def parse(body: dict) -> Weather:
    """The weather for the coming hour from a Locationforecast reply."""
    now = body["properties"]["timeseries"][0]["data"]
    period = now.get("next_1_hours") or now.get("next_6_hours") or {}
    return weather_from_symbol(period["summary"]["symbol_code"])


class Forecast:
    def __init__(self, lat: float, lon: float, clock=time.monotonic, opener=urllib.request.urlopen):
        self.url = URL.format(lat=lat, lon=lon)
        self.clock = clock
        self.opener = opener
        self.weather = Weather()
        self.asked_at: float | None = None
        self.got_at: float | None = None
        self.last_modified: str | None = None

    def now(self) -> Weather:
        """The current weather, asking MET when what we have is ten minutes old.
        Raises OSError or ValueError when MET cannot be reached or understood; the last
        weather is still in ``weather`` for three hours after that."""
        t = self.clock()
        if self.got_at is not None and t - self.got_at > STALE_SECS:
            self.weather = Weather()
        if self.asked_at is not None and t - self.asked_at < KEEP_SECS:
            return self.weather
        self.asked_at = t
        headers = {"User-Agent": USER_AGENT, "Accept": "application/json"}
        if self.last_modified:
            headers["If-Modified-Since"] = self.last_modified
        try:
            with self.opener(urllib.request.Request(self.url, headers=headers), timeout=5) as resp:
                self.weather = parse(json.load(resp))
                self.last_modified = resp.headers.get("Last-Modified")
        except urllib.error.HTTPError as e:
            if e.code != 304:
                raise
        except (KeyError, IndexError, TypeError) as e:
            raise ValueError(f"unexpected forecast: {e!r}") from None
        self.got_at = t
        return self.weather
