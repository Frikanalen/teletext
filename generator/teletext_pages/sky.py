"""The sky over the index page: where the sun is over Oslo right now, and the weather.

:func:`look` works out the scene from the time, the place and the weather, and
:func:`draw` paints it in sixels: a sliced sun that grows as it nears the horizon, a
moon in its phase and stars at night, and clouds, rain, snow, fog and lightning.
:func:`gradient` gives the colours a Level 2.5 decoder puts behind it, one per row.

The sun's position follows the low-precision formulas of the Astronomical Almanac,
good to a fraction of a degree, which is finer than a sixel.
"""

import itertools
import math
from dataclasses import dataclass
from datetime import UTC, datetime

from .art import BLACK, BLUE, CYAN, MAGENTA, RED, WHITE, YELLOW, Picture

RGB = tuple[int, int, int]

ROWS = 8  # teletext rows of sky
HORIZON = 23  # sixel row of the horizon line, the last of the sky
LEFT, RIGHT = 2, 78  # sixel columns drawn in: column 0 can only be black
STEPS = 8  # gradient colours, top to horizon
J2000 = datetime(2000, 1, 1, 12, tzinfo=UTC)
NEW_MOON = datetime(2000, 1, 6, 18, 14, tzinfo=UTC)
SYNODIC_DAYS = 29.530588853


@dataclass(frozen=True)
class Weather:
    clouds: int = 0  # 0 clear, 1 a cloud, 2 partly cloudy, 3 overcast
    rain: bool = False
    snow: bool = False
    fog: bool = False
    thunder: bool = False


CLEAR = Weather()


def weather_from_symbol(symbol: str) -> Weather:
    """MET Norway's ``symbol_code`` (``partlycloudy_day``, ``heavyrainandthunder``,
    ``lightsnowshowers_night`` ...) as a :class:`Weather`."""
    name = symbol.split("_")[0]
    wet = "rain" in name or "sleet" in name
    snow = "snow" in name or "sleet" in name
    if name == "clearsky":
        clouds = 0
    elif name == "fair":
        clouds = 1
    elif name == "partlycloudy" or "showers" in name:
        clouds = 2
    else:
        clouds = 3
    return Weather(clouds, wet, snow, name == "fog", "thunder" in name)


def sun_position(at: datetime, lat: float, lon: float) -> tuple[float, float]:
    """The sun's elevation above the horizon and its azimuth (clockwise from north),
    in degrees."""
    n = (at - J2000).total_seconds() / 86400
    mean_longitude = math.radians((280.460 + 0.9856474 * n) % 360)
    anomaly = math.radians((357.528 + 0.9856003 * n) % 360)
    longitude = mean_longitude + math.radians(
        1.915 * math.sin(anomaly) + 0.020 * math.sin(2 * anomaly)
    )
    obliquity = math.radians(23.439 - 0.0000004 * n)
    right_ascension = math.atan2(math.cos(obliquity) * math.sin(longitude), math.cos(longitude))
    declination = math.asin(math.sin(obliquity) * math.sin(longitude))
    sidereal = math.radians((18.697374558 + 24.06570982441908 * n) % 24 * 15 + lon)
    hour_angle = sidereal - right_ascension
    phi = math.radians(lat)
    elevation = math.asin(
        math.sin(phi) * math.sin(declination)
        + math.cos(phi) * math.cos(declination) * math.cos(hour_angle)
    )
    azimuth = math.atan2(
        -math.sin(hour_angle),
        math.tan(declination) * math.cos(phi) - math.sin(phi) * math.cos(hour_angle),
    )
    return math.degrees(elevation), math.degrees(azimuth) % 360


def moon_phase(at: datetime) -> float:
    """0 at new moon, 0.5 at full moon, towards 1 as it wanes."""
    return (at - NEW_MOON).total_seconds() / 86400 / SYNODIC_DAYS % 1


@dataclass(frozen=True)
class Look:
    """Everything the picture depends on, rounded so that it only changes when the
    picture would."""

    elevation: float  # of the sun, in half degrees
    sun_x: int  # sixel column of the sun's centre
    morning: bool
    moon_x: int
    moon: float  # phase, in sixteenths
    weather: Weather = CLEAR


def look(at: datetime, weather: Weather = CLEAR, lat: float = 59.91, lon: float = 10.75) -> Look:
    elevation, azimuth = sun_position(at, lat, lon)
    # South is the middle of the picture, east on the left.
    sun_x = 40 + round(max(-1.0, min(1.0, (azimuth - 180) / 110)) * 24)
    # The moon crosses the picture once a night, opposite the sun.
    night = ((azimuth + 180) % 360 - 180) / 110
    moon_x = 40 + round(max(-1.0, min(1.0, night)) * 28)
    return Look(
        elevation=round(elevation * 2) / 2,
        sun_x=sun_x,
        morning=azimuth < 180,
        moon_x=moon_x,
        moon=round(moon_phase(at) * 16) / 16 % 1,
        weather=weather,
    )


# --- drawing ---------------------------------------------------------------------

STARS = [(6, 7), (14, 12), (9, 19), (70, 8), (64, 14), (74, 20), (57, 5), (23, 6), (3, 15)]
STARS += [(76, 13), (33, 10), (47, 8), (27, 16), (52, 17), (40, 4), (18, 20), (61, 20)]
# Clouds as (centre x, centre y, size), in the order they appear as it clouds over.
CLOUDS = [(60, 7, 1.0), (22, 12, 1.2), (44, 5, 1.1), (70, 15, 1.0), (10, 5, 0.9), (38, 14, 1.4)]
CLOUDS_SHOWN = {0: 0, 1: 1, 2: 3, 3: 6}


def _put(pic: Picture, x: int, y: int, colour: int) -> None:
    if LEFT <= x < RIGHT and 0 <= y < HORIZON:
        pic.sixels[y][x] = colour


def _sun(pic: Picture, at: Look) -> None:
    e = at.elevation
    if e <= -4:
        return
    # A sixel is 1.25 times as wide as it is tall, so a round sun is taller in sixels.
    # Big while it sits on the horizon, smaller as it climbs clear of it.
    room = HORIZON - 1
    ry = min(room, room / (1 + max(e, 0) / 8))
    if e > 8:
        ry = max(7.5, ry - (e - 8) * 0.2)
    rx = ry / 1.25
    if e < 0:
        cy = HORIZON - e * 6  # sinking: gone at -4 degrees
    elif e <= 8:
        cy = HORIZON - ry * e / 8
    else:
        cy = HORIZON - ry - min(HORIZON - 1 - 2 * ry, (e - 8) * 0.4)
    top = max(0, math.ceil(cy - ry))
    bottom = min(HORIZON, math.floor(cy + ry) + 1)
    high = e >= 12
    for y in range(top, bottom):
        # Slits, closer together towards the bottom of the disc.
        below = bottom - 1 - y
        if not high and below in (0, 1, 3, 4, 7, 8, 11) and y > cy - ry / 2:
            continue
        t = (y + 0.5 - cy) / ry
        if abs(t) >= 1:
            continue
        half = rx * math.sqrt(1 - t * t)
        part = (y - top) / max(bottom - top, 1)
        colour = YELLOW if high or part < 0.42 else (RED if part < 0.72 else MAGENTA)
        for x in range(round(at.sun_x - half), round(at.sun_x + half)):
            _put(pic, x, y, colour)


def _moon(pic: Picture, at: Look) -> None:
    rx, ry, cy = 4.0, 5.0, 7.0
    lit = math.cos(2 * math.pi * at.moon)  # 1 at new moon, -1 at full
    for y in range(int(cy - ry), int(cy + ry) + 1):
        t = (y + 0.5 - cy) / ry
        if abs(t) >= 1:
            continue
        half = rx * math.sqrt(1 - t * t)
        # The terminator is an ellipse; waxing moons are lit on the right.
        edge = lit * half
        lo, hi = (edge, half) if at.moon < 0.5 else (-half, -edge)
        if hi - lo < 1:  # always leave a sliver to see
            lo, hi = (half - 1, half) if at.moon < 0.5 else (-half, -half + 1)
        for x in range(round(at.moon_x + lo), round(at.moon_x + hi)):
            _put(pic, x, y, WHITE)


def _cloud(pic: Picture, cx: int, cy: int, size: float, colour: int) -> tuple[int, int, int]:
    """Three overlapping puffs with a flat base. Returns its span and base."""
    puffs = [(-5, 1, 5, 2.4), (0, -1, 5.5, 3.6), (6, 1, 5, 2.4)]
    base = cy + round(2 * size)
    for dx, dy, prx, pry in puffs:
        px, py, prx, pry = cx + dx * size, cy + dy * size, prx * size, pry * size
        for y in range(int(py - pry), base + 1):
            t = (y + 0.5 - py) / pry
            if abs(t) >= 1:
                continue
            half = prx * math.sqrt(1 - t * t)
            for x in range(round(px - half), round(px + half)):
                _put(pic, x, y, colour)
    return round(cx - 10 * size), round(cx + 11 * size), base


def draw(pic: Picture, at: Look) -> None:
    """Paint the sky into the top :data:`ROWS` rows of ``pic``."""
    w = at.weather
    dark = at.elevation < -6
    if w.clouds < 3 and not w.fog:
        if at.elevation < -3:
            for x, y in STARS:
                _put(pic, x, y, WHITE)
        if dark:
            _moon(pic, at)
    _sun(pic, at)
    spans = []
    for cx, cy, size in CLOUDS[: CLOUDS_SHOWN[w.clouds]]:
        spans.append(_cloud(pic, cx, cy, size, BLUE if dark else WHITE))
    # Rain and snow fall through open sky under each cloud, not across what is there.
    for x0, x1, base in spans:
        for y in range(base + 1, HORIZON - 1):
            for x in range(max(x0, LEFT), min(x1, RIGHT)):
                if pic.sixels[y][x] != BLACK:
                    continue
                if w.rain and (x + y) % 5 == 0 and y % 3 != 2:
                    pic.sixels[y][x] = CYAN
                elif w.snow and (x * 37 + y * 101 + x * y * 7) % 13 == 0:
                    pic.sixels[y][x] = WHITE
    if w.thunder and spans:
        x0, x1, base = spans[0]
        x = (x0 + x1) // 2
        for dy in range(HORIZON - 2 - base):
            dx = (3, 2, 1, 0, 3, 2, 1, 0, -1, 2, 1, 0)[dy % 12]
            for n in range(3):
                _put(pic, x + dx + n, base + 1 + dy, YELLOW)
    if w.fog:
        for n, y in enumerate((13, 16, 19)):
            for x in range(LEFT, RIGHT):
                if (x + n * 7) % 16 < 11:
                    _put(pic, x, y, WHITE)
    for x in range(LEFT, RIGHT):
        pic.sixels[HORIZON][x] = CYAN


# --- Level 2.5 colours -----------------------------------------------------------


def _ramp(*stops: str) -> list[RGB]:
    """:data:`STEPS` colours from top to horizon, through the given ``#rrggbb`` stops."""
    rgb = [tuple(int(s[i : i + 2], 16) for i in (1, 3, 5)) for s in stops]
    out = []
    for n in range(STEPS):
        pos = n / (STEPS - 1) * (len(rgb) - 1)
        i = min(int(pos), len(rgb) - 2)
        out.append(_mix(rgb[i], rgb[i + 1], pos - i))
    return out


def _mix(a: RGB, b: RGB, t: float) -> RGB:
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


# Sun elevation -> sky, for evenings and for mornings (cooler, pinker).
# Colours are sent as 4 bits a channel, so the night keeps red and green equal: a
# step that rounds only one of them shows as a greenish band.
NIGHT = _ramp("#00001a", "#11113a", "#22225a")
DAY = _ramp("#1c4f9c", "#3b84c8", "#80bbe2")
EVENING = [
    (-12.0, NIGHT),
    (-5.0, _ramp("#0c0a2e", "#22104a", "#4a1a5e", "#80285f")),
    (0.0, _ramp("#140c3a", "#331456", "#641f62", "#9c3358", "#b8434f")),
    (5.0, _ramp("#1e1c5c", "#4a2a70", "#8a3a6c", "#c8524c", "#ee8a3a")),
    (12.0, DAY),
]
MORNING = [
    (-12.0, NIGHT),
    (-5.0, _ramp("#0a0e30", "#1c2458", "#3f3470", "#7a4a7c")),
    (0.0, _ramp("#16204c", "#33306c", "#6a4684", "#b05f80", "#e08070")),
    (5.0, _ramp("#23408a", "#4a5aa0", "#8a70a8", "#d08a90", "#f7b070")),
    (12.0, DAY),
]


def gradient(at: Look) -> list[RGB]:
    """The sky's colours on a Level 2.5 decoder, from the top row to the horizon:
    blended between the keyframes by the sun's elevation, greyer under cloud."""
    frames = MORNING if at.morning else EVENING
    e = max(frames[0][0], min(frames[-1][0], at.elevation))
    for (e0, a), (e1, b) in itertools.pairwise(frames):
        if e <= e1:
            t = (e - e0) / (e1 - e0)
            colours = [_mix(x, y, t) for x, y in zip(a, b)]
            break
    dull = {0: 0.0, 1: 0.0, 2: 0.25, 3: 0.6}[at.weather.clouds]
    if at.weather.fog:
        dull = max(dull, 0.6)
    out = []
    for r, g, b in colours:
        grey = round(0.3 * r + 0.59 * g + 0.11 * b)
        out.append(_mix((r, g, b), (grey, grey, round(grey * 1.08)), dull))
    return [tuple(min(255, v) for v in c) for c in out]
