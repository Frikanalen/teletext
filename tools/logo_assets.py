"""Draw the generator's logo assets from the logo's vector paths.

    python3 tools/logo_assets.py ../on-air-graphics/src/core/components/Logo.tsx

Writes generator/teletext_pages/assets/wordmark.ppm (355x27) and mark.ppm (56x45) in
teletext pixels, which are about 0.69 times as wide as they are tall. Each pixel is on
when the shape covers at least half of it, which gives diagonals the most regular
steps one bit per pixel allows. Needs Pillow; the generator itself does not.
"""

import os
import re
import sys

from PIL import Image, ImageChops, ImageDraw

INK = (15, 25, 38)  # the background level25.read_ppm looks for
PAPER = (245, 239, 228)
BLADES = [(241, 86, 69), (255, 199, 9), (100, 188, 80)]  # red, yellow, green
ASSETS = os.path.join(os.path.dirname(__file__), "..", "generator", "teletext_pages", "assets")
SUPERSAMPLE = 8


def flatten(d: str) -> list[list[tuple[float, float]]]:
    """An SVG path (absolute M, L, H, V, C and Z only) as closed polygons."""
    tokens = re.findall(r"[MLHVCZ]|-?[\d.]+(?:e-?\d+)?", d)
    subpaths, points, cur, cmd, i = [], [], (0.0, 0.0), "", 0

    def num() -> float:
        nonlocal i
        i += 1
        return float(tokens[i - 1])

    while i < len(tokens):
        if tokens[i] in "MLHVCZ":
            cmd = tokens[i]
            i += 1
            if cmd == "Z":
                subpaths.append(points)
                points = []
            continue
        if cmd == "M":
            cur = (num(), num())
            points = [cur]
            cmd = "L"
        elif cmd == "L":
            cur = (num(), num())
            points.append(cur)
        elif cmd == "H":
            cur = (num(), cur[1])
            points.append(cur)
        elif cmd == "V":
            cur = (cur[0], num())
            points.append(cur)
        elif cmd == "C":
            p1, p2, p3 = (num(), num()), (num(), num()), (num(), num())
            for k in range(1, 13):
                t, u = k / 12, 1 - k / 12
                points.append(
                    tuple(
                        u**3 * cur[j] + 3 * u * u * t * p1[j] + 3 * u * t * t * p2[j] + t**3 * p3[j]
                        for j in (0, 1)
                    )
                )
            cur = p3
    if points:
        subpaths.append(points)
    return subpaths


def cover(paths: list[str], x0: float, y0: float, sx: float, sy: float, size) -> Image.Image:
    """Which pixels of a ``size`` image the paths cover by half or more (even-odd fill)."""
    w, h = size
    big = Image.new("1", (w * SUPERSAMPLE, h * SUPERSAMPLE), 0)
    for d in paths:
        for sub in flatten(d):
            layer = Image.new("1", big.size, 0)
            scaled = [((x - x0) * sx * SUPERSAMPLE, (y - y0) * sy * SUPERSAMPLE) for x, y in sub]
            ImageDraw.Draw(layer).polygon(scaled, fill=1)
            big = ImageChops.logical_xor(big, layer)
    return big.convert("L").resize(size, Image.BOX).point(lambda v: 255 if v >= 128 else 0)


def main() -> None:
    paths = re.findall(r'd="([^"]+)"', open(sys.argv[1]).read())
    blades, letters = paths[:3], paths[3:]

    size = (355, 27)  # the wordmark is 184.66 units wide and its capitals 20.23 tall
    word = Image.new("RGB", size, INK)
    word.paste(PAPER, mask=cover(letters, 55.19, 15.78, size[0] / 184.66, size[1] / 20.23, size))
    word.save(os.path.join(ASSETS, "wordmark.ppm"))

    # The mark is 45 units wide and 52 tall. At this size, and where index.logo() puts
    # it, no blade shares a cell with more than a sliver of another (a cell has one colour).
    size = (56, 45)
    mark = Image.new("RGB", size, INK)
    for d, colour in zip(blades, BLADES):
        mark.paste(colour, mask=cover([d], 0, 0, size[0] / 45, size[1] / 52, size))
    mark.save(os.path.join(ASSETS, "mark.ppm"))


if __name__ == "__main__":
    main()
