"""What is on air, from django-api's compiled schedule (``/api/scheduleitems/``)."""

import json
import urllib.parse
import urllib.request
from dataclasses import dataclass
from datetime import datetime
from zoneinfo import ZoneInfo

OSLO = ZoneInfo("Europe/Oslo")


@dataclass(frozen=True)
class Programme:
    start: datetime
    end: datetime
    title: str
    organization: str | None = None
    live: bool = False


def parse(results: list[dict]) -> list[Programme]:
    """Schedule items from the API, in start order."""
    out = []
    for item in results:
        video = item.get("video") or {}
        org = video.get("organization") or {}
        out.append(
            Programme(
                start=datetime.fromisoformat(item["starttime"]),
                end=datetime.fromisoformat(item["endtime"]),
                title=video.get("name") or item.get("default_name") or "",
                organization=org.get("name") or None,
                live=bool(item.get("is_live")),
            )
        )
    return sorted(out, key=lambda p: p.start)


def now_and_next(items: list[Programme], at: datetime) -> tuple[Programme | None, Programme | None]:
    """The programme on air at ``at`` (if any), and the next one to start."""
    current = next((p for p in items if p.start <= at < p.end), None)
    upcoming = next((p for p in items if p.start > at), None)
    return current, upcoming


def next_change(items: list[Programme], at: datetime) -> datetime | None:
    """The next start or end after ``at``, when now/next changes."""
    times = [t for p in items for t in (p.start, p.end) if t > at]
    return min(times, default=None)


def fetch(api: str, timeout: float = 5, attempts: int = 2) -> list[Programme]:
    """Today's schedule plus the items either side, so a programme that started
    yesterday and the first one tomorrow are included. A failed request is tried again
    right away, ``attempts`` times in all."""
    for attempt in range(attempts):
        try:
            return _fetch(api, timeout)
        except (OSError, ValueError, KeyError, TypeError):
            if attempt == attempts - 1:
                raise
    return []


def _fetch(api: str, timeout: float) -> list[Programme]:
    query = urllib.parse.urlencode({"date": "today", "days": 1, "surrounding": "true"})
    # No trailing slash: frikanalen.tv answers /api/scheduleitems/ with 404.
    url = f"{api.rstrip('/')}/scheduleitems?{query}"
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        body = json.load(resp)
    return parse(body["results"] if isinstance(body, dict) else body)
