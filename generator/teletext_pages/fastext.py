"""Fastext: packet X/27/0, which ties the remote's red, green, yellow and cyan keys
(and the index key) to pages. Level 1.5 decoders and up use it; ttxd sends it as a
raw packet (``{"packet": 27, "designation": 0, "raw": [...39 bytes]}``).
"""

HAM84 = [
    0x15,
    0x02,
    0x49,
    0x5E,
    0x64,
    0x73,
    0x38,
    0x2F,
    0xD0,
    0xC7,
    0x8C,
    0x9B,
    0xA1,
    0xB6,
    0xFD,
    0xEA,
]
NO_PAGE = "8FF"


def _link(page: str, magazine: int) -> list[int]:
    """Six Hamming 8/4 bytes for one link: page units and tens, then subcode 3F7F
    (any) carrying the link's magazine as bits relative to the current one."""
    mag = (int(page[0]) ^ magazine) & 7
    num = int(page[1:], 16)
    nibbles = [
        num & 15,
        num >> 4,
        0xF,  # S1
        0x7 | (mag & 1) << 3,  # S2, M1
        0xF,  # S3
        0x3 | (mag >> 1) << 2,  # S4, M2, M3
    ]
    return [HAM84[n] for n in nibbles]


def x27_0(own_page: str, colours: list[str | None], index: str | None = None) -> list[int]:
    """39 coded bytes after the designation: links for red, green, yellow and cyan
    (None for no link), the next-page link (unused), the index key, then the link
    control byte (display row 24) and a CRC, which decoders ignore."""
    magazine = int(own_page[0]) & 7
    pages = (list(colours) + [None] * 4)[:4] + [None, index]
    out = []
    for p in pages:
        out += _link(p or NO_PAGE, magazine)
    out.append(HAM84[0xF])  # link control: show row 24
    out += [0, 0]  # CRC
    return out
