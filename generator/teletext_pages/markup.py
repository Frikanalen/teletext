"""Helpers for building rows in ttxd's markup (see docs/design.md#api).

A row is at most 40 cells. Every character is one cell and every ``{attribute}`` is
one cell (shown as a space). Text from outside (programme titles, organisation names)
goes through :func:`plain` first, so it can never inject markup.
"""

import re

COLS = 40
ROWS = 24

_TYPOGRAPHY = str.maketrans(
    {
        "{": "(",
        "[": "(",
        "}": ")",
        "]": ")",
        "‘": "'",
        "’": "'",
        "‚": "'",
        "`": "'",
        "´": "'",
        "“": '"',
        "”": '"',
        "„": '"',
        "«": '"',
        "»": '"',
        "–": "-",
        "—": "-",
        "‐": "-",
        "−": "-",
        "…": "...",
    }
)
_ATTR = re.compile(r"\{\{|\}\}|\{[^{}]*\}")


def plain(text: str) -> str:
    """Clean outside text for a row: no braces, no control characters, single spaces,
    and common typographic characters replaced by ones teletext can show."""
    text = text.translate(_TYPOGRAPHY)
    text = "".join(" " if c.isspace() or not c.isprintable() else c for c in text)
    return " ".join(text.split())


def cells(markup: str) -> int:
    """Number of cells a row of markup takes."""
    return len(_ATTR.sub("#", markup))


def fit(text: str, width: int) -> str:
    """Cut plain text (from :func:`plain`) to at most ``width`` cells.

    A cut ends in a full stop (unless it already ends a sentence) and prefers the last word boundary when that keeps at
    least two thirds of the width."""
    if len(text) <= width:
        return text
    if width < 2:
        return text[:width]
    head = text[: width - 1]
    space = head.rfind(" ")
    if space >= 0 and space * 3 >= width * 2:
        head = head[:space]
    head = head.rstrip(" ,;:-")
    return head if head.endswith((".", "!", "?")) else head + "."


def spread(left: str, right: str) -> str:
    """Pad ``left`` so that ``right`` ends in the last column. Both are markup.
    If they do not fit together, ``right`` is dropped."""
    used = cells(left) + cells(right)
    if not right or used > COLS:
        return left
    return left + " " * (COLS - used) + right


def leader(title: str, page: str) -> str:
    """A directory row: ``Sendeplan .........200`` with the number in cyan."""
    room = COLS - 2 - len(page)  # two attribute cells
    title = fit(plain(title), room - 3)
    return f"{{white}}{title} {'.' * (room - len(title) - 1)}{{cyan}}{page}"


def preview(row: str) -> str:
    """Roughly what a row looks like on screen: attributes become spaces."""
    return _ATTR.sub(lambda m: m[0][0] if m[0] in ("{{", "}}") else " ", row)


def check(rows: list[str]) -> list[str]:
    """Raise if a page would be rejected by ttxd for its size."""
    if len(rows) > ROWS:
        raise ValueError(f"{len(rows)} rows, max {ROWS}")
    for n, row in enumerate(rows, 1):
        if cells(row) > COLS:
            raise ValueError(f"row {n} is {cells(row)} cells, max {COLS}: {row!r}")
    return rows
