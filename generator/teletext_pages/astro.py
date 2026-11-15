"""Where things are in the sky: the brightest stars, the moon and the bright planets,
as altitude and azimuth for a place and a time.

Low-precision formulas throughout (the Astronomical Almanac's for the sun and the
moon, JPL's approximate orbital elements for the planets): good to a fraction of a
degree for the sun and the stars and about a degree for the moon and the planets,
which is finer than the sixels they are drawn with.
"""

import math
from datetime import UTC, datetime

J2000 = datetime(2000, 1, 1, 12, tzinfo=UTC)

# The brightest stars that rise over southern Norway: name, right ascension in hours,
# declination in degrees (equinox of 2026), magnitude, spectral class.
STARS = [
    ("Sirius", 6.772, -16.75, -1.44, "A"),
    ("Arcturus", 14.282, 19.05, -0.05, "K"),
    ("Vega", 18.631, 38.81, 0.03, "A"),
    ("Capella", 5.311, 46.02, 0.08, "G"),
    ("Rigel", 5.263, -8.17, 0.18, "B"),
    ("Procyon", 7.678, 5.16, 0.40, "F"),
    ("Betelgeuse", 5.943, 7.41, 0.45, "M"),
    ("Altair", 19.868, 8.94, 0.76, "A"),
    ("Aldebaran", 4.624, 16.56, 0.87, "K"),
    ("Spica", 13.443, -11.30, 0.98, "B"),
    ("Antares", 16.518, -26.49, 1.06, "M"),
    ("Pollux", 7.782, 27.96, 1.16, "K"),
    ("Deneb", 20.706, 45.37, 1.25, "A"),
    ("Regulus", 10.163, 11.84, 1.36, "B"),
    ("Castor", 7.605, 31.83, 1.58, "A"),
    ("Bellatrix", 5.442, 6.37, 1.64, "B"),
    ("Elnath", 5.466, 28.63, 1.65, "B"),
    ("Alnilam", 5.626, -1.18, 1.69, "B"),
    ("Alnitak", 5.701, -1.93, 1.74, "O"),
    ("Alioth", 12.920, 55.82, 1.76, "A"),
    ("Mirfak", 3.437, 49.95, 1.79, "F"),
    ("Dubhe", 11.089, 61.61, 1.81, "F"),
    ("Wezen", 7.157, -26.44, 1.83, "F"),
    ("Alcaid", 13.810, 49.18, 1.85, "B"),
    ("Menkalinan", 6.024, 44.95, 1.90, "A"),
    ("Alhena", 6.654, 16.38, 1.93, "A"),
    ("Polaris", 3.086, 89.37, 1.97, "F"),
    ("Mirzam", 6.398, -17.97, 1.98, "B"),
    ("Alphard", 9.481, -8.77, 1.99, "K"),
    ("Algieba", 10.357, 19.71, 2.01, "K"),
    ("Hamal", 2.144, 23.59, 2.01, "K"),
    ("Diphda", 0.749, -17.84, 2.04, "K"),
    ("Nunki", 18.949, -26.26, 2.05, "B"),
    ("Alpheratz", 0.163, 29.24, 2.07, "B"),
    ("Kochab", 14.845, 74.05, 2.07, "K"),
    ("Mirach", 1.187, 35.76, 2.07, "M"),
    ("Saiph", 5.817, -9.66, 2.07, "B"),
    ("Rasalhague", 17.603, 12.54, 2.08, "A"),
    ("Algol", 3.165, 41.06, 2.09, "B"),
    ("Almach", 2.092, 42.45, 2.10, "B"),
    ("Denebola", 11.840, 14.42, 2.14, "A"),
    ("Alphecca", 15.597, 26.63, 2.22, "A"),
    ("Mizar", 13.417, 54.79, 2.23, "A"),
    ("Sadr", 20.387, 40.34, 2.23, "F"),
    ("Eltanin", 17.954, 51.49, 2.24, "K"),
    ("Schedar", 0.700, 56.68, 2.24, "K"),
    ("Mintaka", 5.556, -0.28, 2.25, "O"),
    ("Caph", 0.177, 59.29, 2.28, "F"),
    ("Merak", 11.057, 56.24, 2.34, "A"),
    ("Izar", 14.769, 26.96, 2.35, "A"),
    ("Enif", 21.758, 10.00, 2.38, "K"),
    ("Phecda", 11.920, 53.55, 2.41, "A"),
    ("Sabik", 17.199, -15.76, 2.43, "A"),
    ("Scheat", 23.085, 28.23, 2.44, "M"),
    ("Alderamin", 21.321, 62.69, 2.45, "A"),
    ("Markab", 23.102, 15.35, 2.49, "B"),
    ("Menkar", 3.061, 4.19, 2.54, "M"),
    ("Arneb", 5.565, -17.80, 2.58, "F"),
    ("Gienah", 12.286, -17.69, 2.58, "B"),
    ("Unukalhai", 15.760, 6.34, 2.63, "K"),
    ("Tarazed", 19.792, 10.68, 2.72, "K"),
    ("Zubenelgenubi", 14.873, -16.15, 2.75, "A"),
    ("Cebalrai", 17.747, 4.56, 2.76, "K"),
    ("Rasalgethi", 17.265, 14.36, 2.78, "M"),
    ("Nihal", 5.489, -20.74, 2.81, "G"),
    ("Algenib", 0.244, 15.33, 2.83, "B"),
    ("Alcyone", 3.818, 24.19, 2.85, "B"),
    ("Vindemiatrix", 13.058, 10.82, 2.85, "G"),
    ("Sadalmelik", 22.119, -0.19, 2.95, "G"),
    ("Zaurak", 3.988, -13.43, 2.97, "M"),
    ("Minkar", 12.192, -22.77, 3.02, "K"),
    ("Albireo", 19.530, 28.01, 3.05, "K"),
]

# Keplerian elements at J2000 and their change per century (JPL, valid 1800-2050):
# semi-major axis (au), eccentricity, inclination, mean longitude, longitude of
# perihelion, longitude of the ascending node (degrees).
ELEMENTS = {
    "Venus": (
        (0.72333566, 0.00677672, 3.39467605, 181.97909950, 131.60246718, 76.67984255),
        (0.00000390, -0.00004107, -0.00078890, 58517.81538729, 0.00268329, -0.27769418),
    ),
    "Earth": (
        (1.00000261, 0.01671123, -0.00001531, 100.46457166, 102.93768193, 0.0),
        (0.00000562, -0.00004392, -0.01294668, 35999.37244981, 0.32327364, 0.0),
    ),
    "Mars": (
        (1.52371034, 0.09339410, 1.84969142, -4.55343205, -23.94362959, 49.55953891),
        (0.00001847, 0.00007882, -0.00813131, 19140.30268499, 0.44441088, -0.29257343),
    ),
    "Jupiter": (
        (5.20288700, 0.04838624, 1.30439695, 34.39644051, 14.72847983, 100.47390909),
        (-0.00011607, -0.00013253, -0.00183714, 3034.74612775, 0.21252668, 0.20469106),
    ),
    "Saturn": (
        (9.53667594, 0.05386179, 2.48599187, 49.95424423, 92.59887831, 113.66242448),
        (-0.00125060, -0.00050991, 0.00193609, 1222.49362201, -0.41897216, -0.28867794),
    ),
}
PLANETS = ("Venus", "Mars", "Jupiter", "Saturn")


def days(at: datetime) -> float:
    """Days since J2000."""
    return (at - J2000).total_seconds() / 86400


def _obliquity(n: float) -> float:
    return math.radians(23.439 - 0.0000004 * n)


def equatorial(longitude: float, latitude: float, n: float) -> tuple[float, float]:
    """Ecliptic longitude and latitude (radians) as right ascension and declination
    (radians)."""
    e = _obliquity(n)
    ra = math.atan2(
        math.sin(longitude) * math.cos(e) - math.tan(latitude) * math.sin(e),
        math.cos(longitude),
    )
    dec = math.asin(
        math.sin(latitude) * math.cos(e) + math.cos(latitude) * math.sin(e) * math.sin(longitude)
    )
    return ra, dec


def horizontal(ra: float, dec: float, at: datetime, lat: float, lon: float) -> tuple[float, float]:
    """Right ascension and declination (radians) as altitude above the horizon and
    azimuth clockwise from north, in degrees, seen from ``lat``, ``lon`` at ``at``."""
    n = days(at)
    sidereal = math.radians((18.697374558 + 24.06570982441908 * n) % 24 * 15 + lon)
    hour_angle = sidereal - ra
    phi = math.radians(lat)
    altitude = math.asin(
        math.sin(phi) * math.sin(dec) + math.cos(phi) * math.cos(dec) * math.cos(hour_angle)
    )
    azimuth = math.atan2(
        -math.sin(hour_angle),
        math.tan(dec) * math.cos(phi) - math.sin(phi) * math.cos(hour_angle),
    )
    return math.degrees(altitude), math.degrees(azimuth) % 360


def sun_longitude(n: float) -> float:
    """The sun's ecliptic longitude, in radians."""
    mean = math.radians((280.460 + 0.9856474 * n) % 360)
    anomaly = math.radians((357.528 + 0.9856003 * n) % 360)
    return mean + math.radians(1.915 * math.sin(anomaly) + 0.020 * math.sin(2 * anomaly))


def sun(at: datetime, lat: float, lon: float) -> tuple[float, float]:
    """The sun's altitude and azimuth, in degrees."""
    n = days(at)
    return horizontal(*equatorial(sun_longitude(n), 0.0, n), at, lat, lon)


def star(ra_hours: float, dec: float, at: datetime, lat: float, lon: float) -> tuple[float, float]:
    """A star's altitude and azimuth, in degrees."""
    return horizontal(math.radians(ra_hours * 15), math.radians(dec), at, lat, lon)


def _moon_ecliptic(n: float) -> tuple[float, float, float]:
    """The moon's ecliptic longitude and latitude (radians) and its horizontal
    parallax (degrees)."""
    t = n / 36525

    def s(a: float, b: float) -> float:
        return math.sin(math.radians(a + b * t))

    def c(a: float, b: float) -> float:
        return math.cos(math.radians(a + b * t))

    longitude = (
        218.32
        + 481267.881 * t
        + 6.29 * s(135.0, 477198.87)
        - 1.27 * s(259.3, -413335.36)
        + 0.66 * s(235.7, 890534.22)
        + 0.21 * s(269.9, 954397.74)
        - 0.19 * s(357.5, 35999.05)
        - 0.11 * s(186.5, 966404.03)
    )
    latitude = (
        5.13 * s(93.3, 483202.02)
        + 0.28 * s(228.2, 960400.89)
        - 0.28 * s(318.3, 6003.15)
        - 0.17 * s(217.6, -407332.21)
    )
    parallax = (
        0.9508
        + 0.0518 * c(135.0, 477198.87)
        + 0.0095 * c(259.3, -413335.36)
        + 0.0078 * c(235.7, 890534.22)
        + 0.0028 * c(269.9, 954397.74)
    )
    return math.radians(longitude % 360), math.radians(latitude), parallax


def moon(at: datetime, lat: float, lon: float) -> tuple[float, float]:
    """The moon's altitude and azimuth, in degrees, as seen from the ground (the
    moon is near enough that it stands up to a degree lower than from the Earth's
    centre)."""
    n = days(at)
    longitude, latitude, parallax = _moon_ecliptic(n)
    altitude, azimuth = horizontal(*equatorial(longitude, latitude, n), at, lat, lon)
    return altitude - parallax * math.cos(math.radians(altitude)), azimuth


def moon_phase(at: datetime) -> float:
    """0 at new moon, 0.5 at full moon, towards 1 as it wanes: how far the moon is
    ahead of the sun along the ecliptic."""
    n = days(at)
    return (_moon_ecliptic(n)[0] - sun_longitude(n)) / (2 * math.pi) % 1


def _heliocentric(name: str, n: float) -> tuple[float, float, float]:
    """A planet's position in the ecliptic plane's coordinates, in au."""
    t = n / 36525
    a, e, incl, mean_long, peri, node = (x + dx * t for x, dx in zip(*ELEMENTS[name], strict=True))
    arg_peri = math.radians(peri - node)
    anomaly = math.radians((mean_long - peri + 180) % 360 - 180)
    ecc = anomaly
    for _ in range(8):  # Kepler's equation
        ecc -= (ecc - e * math.sin(ecc) - anomaly) / (1 - e * math.cos(ecc))
    x = a * (math.cos(ecc) - e)
    y = a * math.sqrt(1 - e * e) * math.sin(ecc)
    incl, node = math.radians(incl), math.radians(node)
    cw, sw, co, so, ci, si = (
        math.cos(arg_peri),
        math.sin(arg_peri),
        math.cos(node),
        math.sin(node),
        math.cos(incl),
        math.sin(incl),
    )
    return (
        (cw * co - sw * so * ci) * x + (-sw * co - cw * so * ci) * y,
        (cw * so + sw * co * ci) * x + (-sw * so + cw * co * ci) * y,
        sw * si * x + cw * si * y,
    )


def planet(name: str, at: datetime, lat: float, lon: float) -> tuple[float, float]:
    """A planet's altitude and azimuth, in degrees."""
    n = days(at)
    px, py, pz = _heliocentric(name, n)
    ex, ey, ez = _heliocentric("Earth", n)
    x, y, z = px - ex, py - ey, pz - ez
    longitude = math.atan2(y, x)
    latitude = math.atan2(z, math.hypot(x, y))
    return horizontal(*equatorial(longitude, latitude, n), at, lat, lon)
