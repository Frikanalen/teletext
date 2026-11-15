"""The sky over the index page: the southern sky over Oslo right now, and the weather.

The picture is a window to the south, east on the left and west on the right, from the
horizon up to about 50 degrees. :func:`look` works out the scene from the time, the
place and the weather, and :func:`draw` paints it in sixels: a sliced sun that grows
as it nears the horizon; at night the brightest stars, the planets and the moon in
its phase, each where it really is (:mod:`astro`); and clouds that drift with the
wind, rain, snow, fog and lightning. :func:`gradient` gives the colours a Level 2.5
decoder puts behind it, one per row.

The sun is drawn large and stylised, so only its direction and whether it is up are
to scale. Everything else is placed to the sixel, which is about 3 degrees.
"""

import itertools
import math
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta

from . import astro
from .art import BLACK, BLUE, CYAN, MAGENTA, RED, WHITE, YELLOW, Picture

RGB = tuple[int, int, int]

ROWS = 8  # teletext rows of sky
HORIZON = 23  # sixel row of the horizon line, the last of the sky
LEFT, RIGHT = 2, 78  # sixel columns drawn in: column 0 can only be black
STEPS = 8  # gradient colours, top to horizon
# The window: this many degrees either side of south across the picture's half
# width, and degrees of altitude per sixel row (a sixel is 1.25 times as wide as tall).
MIDDLE, HALF_WIDTH, AZIMUTH_SPAN = 40, 36, 100.0
DEGREES_PER_ROW = AZIMUTH_SPAN / HALF_WIDTH / 1.25
# The stars turn a sixel in about ten minutes. They are placed for the last whole
# step of this many minutes, so the page is not sent again for every star that moves.
SKY_STEP_MINUTES = 5
MOON_TOP = 8  # the highest sixel row for the moon's centre
# Clouds drift a step of this many sixels at a time, and wrap round this far apart.
DRIFT_STEP, DRIFT_SPAN = 2, 104


@dataclass(frozen=True)
class Weather:
    clouds: int = 0  # 0 clear, 1 a cloud, 2 partly cloudy, 3 overcast
    rain: bool = False
    snow: bool = False
    fog: bool = False
    thunder: bool = False
    # How many of the six clouds to draw (None: by ``clouds``), how hard it rains or
    # snows (0 light, 1 moderate, 2 heavy), and the wind along the picture in m/s
    # (positive blows the clouds to the right, westwards).
    cover: int | None = None
    intensity: int = 1
    wind: int = 0


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
    return astro.sun(at, lat, lon)


def moon_phase(at: datetime) -> float:
    """0 at new moon, 0.5 at full moon, towards 1 as it wanes."""
    return astro.moon_phase(at)


def place(altitude: float, azimuth: float) -> tuple[int, int]:
    """The sixel (x, y) where a point of the sky falls. It may be outside the picture."""
    south = azimuth % 360 - 180  # degrees west of south
    x = MIDDLE + round(south / AZIMUTH_SPAN * HALF_WIDTH)
    y = HORIZON - 1 - round(altitude / DEGREES_PER_ROW)
    return x, y


@dataclass(frozen=True)
class Body:
    """A star or a planet in the picture."""

    x: int
    y: int
    kind: str  # "star-0" (brightest) to "star-2", or "planet"
    colour: int


@dataclass(frozen=True)
class Look:
    """Everything the picture depends on, rounded so that it only changes when the
    picture would."""

    elevation: float  # of the sun, in half degrees
    sun_x: int  # sixel column of the sun's centre
    morning: bool
    moon_at: tuple[int, int] | None  # sixel of the moon's centre, if it is in the window
    moon: float  # phase, in sixteenths
    weather: Weather = CLEAR
    bodies: tuple[Body, ...] = ()
    drift: int = 0  # how far the clouds have drifted, in sixels


# Spectral class to the nearest teletext colour: hot stars bluish, cool ones red.
STAR_COLOURS = {"O": CYAN, "B": CYAN, "A": WHITE, "F": WHITE, "G": YELLOW, "K": YELLOW, "M": RED}
PLANET_COLOURS = {"Venus": WHITE, "Mars": RED, "Jupiter": YELLOW, "Saturn": YELLOW}
# The sun must be this far below the horizon before a body shows, by how bright it
# is: planets and the brightest stars come out first.
COMES_OUT = {"planet": -2.0, "star-0": -3.0, "star-1": -5.0, "star-2": -7.0}


def _in_window(x: int, y: int) -> bool:
    return LEFT <= x < RIGHT and 0 <= y < HORIZON


def bodies(at: datetime, lat: float, lon: float, elevation: float) -> tuple[Body, ...]:
    """The stars and planets in the window that are out, brightest last (so that it
    is the brightest that shows where two fall on one sixel)."""
    out = []
    for _name, ra, dec, magnitude, spectral in reversed(astro.STARS):
        altitude, azimuth = astro.star(ra, dec, at, lat, lon)
        kind = "star-0" if magnitude <= 0.5 else ("star-1" if magnitude <= 1.8 else "star-2")
        x, y = place(altitude, azimuth)
        if altitude > 1 and _in_window(x, y) and elevation < COMES_OUT[kind]:
            out.append(Body(x, y, kind, STAR_COLOURS[spectral]))
    for name in astro.PLANETS:
        altitude, azimuth = astro.planet(name, at, lat, lon)
        x, y = place(altitude, azimuth)
        if altitude > 1 and _in_window(x, y) and elevation < COMES_OUT["planet"]:
            out.append(Body(x, y, "planet", PLANET_COLOURS[name]))
    return tuple(out)


def look(
    at: datetime,
    weather: Weather = CLEAR,
    lat: float = 59.91,
    lon: float = 10.75,
    drift: float = 0.0,
) -> Look:
    elevation, azimuth = sun_position(at, lat, lon)
    # The sun is too big to follow its azimuth all the way to the edges.
    sun_x = max(MIDDLE - 30, min(MIDDLE + 30, place(0, azimuth)[0]))
    step = timedelta(minutes=SKY_STEP_MINUTES)
    then = datetime.min.replace(tzinfo=UTC) + (at - datetime.min.replace(tzinfo=UTC)) // step * step
    moon_altitude, moon_azimuth = astro.moon(then, lat, lon)
    moon_at = place(moon_altitude, moon_azimuth)
    # The moon is drawn ten sixels tall: it counts while any of it is in the window.
    seen = LEFT - 4 <= moon_at[0] < RIGHT + 4 and -5 <= moon_at[1] < HORIZON + 5
    return Look(
        elevation=round(elevation * 2) / 2,
        sun_x=sun_x,
        morning=azimuth < 180,
        # A moon higher than the window shows stays whole at its top, under the text.
        moon_at=(moon_at[0], max(MOON_TOP, moon_at[1])) if seen else None,
        moon=round(moon_phase(at) * 16) / 16 % 1,
        weather=weather,
        bodies=bodies(then, lat, lon, elevation),
        drift=round(drift / DRIFT_STEP) * DRIFT_STEP % DRIFT_SPAN,
    )


# --- drawing ---------------------------------------------------------------------

# Clouds as (centre x, centre y, size), in the order they appear as it clouds over.
CLOUDS = [(60, 7, 1.0), (22, 12, 1.2), (44, 5, 1.1), (70, 15, 1.0), (10, 5, 0.9), (38, 14, 1.4)]
CLOUDS_SHOWN = {0: 0, 1: 1, 2: 3, 3: 6}
# One sixel in this many is a raindrop or a snowflake, by intensity.
RAIN_EVERY = (8, 5, 3)
SNOW_EVERY = (19, 13, 8)


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
    rx, ry = 4.0, 5.0
    moon_x, cy = at.moon_at[0], at.moon_at[1] + 0.5
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
        for x in range(round(moon_x + lo), round(moon_x + hi)):
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


def draw(pic: Picture, at: Look) -> dict[tuple[int, int], str]:
    """Paint the sky into the top :data:`ROWS` rows of ``pic``. Returns the sixels
    (x, y) that are a lone star (``"star-0"`` the brightest to ``"star-2"``), a
    ``"planet"``, a ``"rain"`` drop or a ``"snow"`` flake, which
    a Level 2.5 page can draw as something finer than a block."""
    w = at.weather
    dark = at.elevation < -6
    stars = w.clouds < 3 and not w.fog
    if stars:
        for b in at.bodies:
            _put(pic, b.x, b.y, b.colour)
    if at.moon_at and at.elevation < -4 and w.clouds < 3 and not w.fog:
        _moon(pic, at)
    _sun(pic, at)
    spans = []
    shown = CLOUDS_SHOWN[w.clouds] if w.cover is None else max(0, min(len(CLOUDS), w.cover))
    for cx, cy, size in CLOUDS[:shown]:
        # Drifting with the wind, and round again from the other side.
        cx = (cx + at.drift + 12) % DRIFT_SPAN - 12
        spans.append(_cloud(pic, cx, cy, size, BLUE if dark else WHITE))
    small: dict[tuple[int, int], str] = {}
    # Rain and snow fall through open sky under each cloud, not across what is there.
    for x0, x1, base in spans:
        for y in range(base + 1, HORIZON - 1):
            for x in range(max(x0, LEFT), min(x1, RIGHT)):
                if pic.sixels[y][x] != BLACK:
                    continue
                if w.rain and (x + y) % RAIN_EVERY[w.intensity] == 0 and y % 3 != 2:
                    pic.sixels[y][x] = CYAN
                    small[(x, y)] = "rain"
                elif w.snow and (x * 37 + y * 101 + x * y * 7) % SNOW_EVERY[w.intensity] == 0:
                    pic.sixels[y][x] = WHITE
                    small[(x, y)] = "snow"
    if w.thunder and spans:
        x0, x1, base = spans[0]
        x = (x0 + x1) // 2
        for dy in range(HORIZON - 2 - base):
            dx = (3, 2, 1, 0, 3, 2, 1, 0, -1, 2, 1, 0)[dy % 12]
            for n in range(3):
                _put(pic, x + dx + n, base + 1 + dy, YELLOW)
                small.pop((x + dx + n, base + 1 + dy), None)
    if w.fog:
        for n, y in enumerate((13, 16, 19)):
            for x in range(LEFT, RIGHT):
                if (x + n * 7) % 16 < 11:
                    _put(pic, x, y, WHITE)
                    small.pop((x, y), None)
    for x in range(LEFT, RIGHT):
        pic.sixels[HORIZON][x] = CYAN
    if stars:
        # A star still counts if nothing was drawn over or right beside it.
        for b in at.bodies:
            x, y = b.x, b.y
            around = [
                pic.sixels[y + dy][x + dx]
                for dy in (-1, 0, 1)
                for dx in (-1, 0, 1)
                if (dx or dy) and 0 <= y + dy < HORIZON and 0 <= x + dx < len(pic.sixels[0])
            ]
            if pic.sixels[y][x] == b.colour and all(c == BLACK for c in around):
                small[(x, y)] = b.kind
    return small


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
