"""ttx-pages: keep Frikanalen's studio teletext pages up to date in ttxd.

    ttx-pages pages.toml            run forever
    ttx-pages pages.toml --dry-run  print the pages once and exit

The ttxd token comes from ``$TTXD_TOKEN`` or ``token`` in the config file.
"""

import argparse
import logging
import os
import sys
import time
import tomllib
from dataclasses import dataclass, field
from datetime import datetime, timedelta

from . import schedule
from .index import Entry, IndexConfig, directory, render
from .markup import COLS, preview
from .ttxd import Ttxd, TtxdError

log = logging.getLogger("ttx-pages")

# A change to more rows than this is sent as a whole page instead of a row flash.
MAX_FLASH_ROWS = 6
# At startup, how long to wait for a first schedule before sending the page anyway,
# and how often to try meanwhile.
FIRST_SCHEDULE_WAIT = 60
FIRST_SCHEDULE_RETRY = 5


@dataclass
class Config:
    ttxd_url: str = "http://127.0.0.1:8080"
    token: str = ""
    api_url: str = "https://frikanalen.tv/api"
    # How often to look at the schedule. Programme changes are also hit on the second.
    poll_secs: int = 30
    # How often to send the whole page again, which renews its TTL.
    refresh_secs: int = 120
    # If this generator stops, ttxd drops its pages after this long.
    ttl_secs: int = 600
    index: IndexConfig = field(default_factory=IndexConfig)


def load_config(path: str) -> Config:
    with open(path, "rb") as f:
        raw = tomllib.load(f)
    index = raw.pop("index", {})
    entries = [Entry(**e) for e in index.pop("entry", [])]
    cfg = Config(**raw, index=IndexConfig(**index, entries=entries))
    cfg.token = os.environ.get("TTXD_TOKEN", cfg.token)
    if cfg.refresh_secs >= cfg.ttl_secs:
        raise ValueError("refresh_secs must be shorter than ttl_secs")
    return cfg


class Pusher:
    """Sends one page to ttxd: the whole page with flash when it is new or much
    changed, only the changed rows as a flash when a few rows change, and the
    whole page again every ``refresh_secs`` so its TTL never runs out."""

    def __init__(
        self, ttxd: Ttxd, page: str, ttl_secs: int, refresh_secs: int, clock=time.monotonic
    ):
        self.ttxd = ttxd
        self.page = page
        self.ttl_secs = ttl_secs
        self.refresh_secs = refresh_secs
        self.clock = clock
        self.sent: list[str] | None = None
        self.sent_at = 0.0

    def push(self, rows: list[str]) -> None:
        changed = {}
        if self.sent is not None:
            changed = {
                n: r for n, r in enumerate(rows, 1) if n > len(self.sent) or self.sent[n - 1] != r
            }
        stale = self.clock() - self.sent_at >= self.refresh_secs
        try:
            if self.sent is None or stale or len(changed) > MAX_FLASH_ROWS:
                flash = self.sent is None or bool(changed)
                self.ttxd.put_page(self.page, [rows], ttl_secs=self.ttl_secs, flash=flash)
                self.sent_at = self.clock()
                log.info("page %s sent%s", self.page, " with flash" if flash else "")
            elif changed:
                self.ttxd.flash_rows(self.page, changed)
                log.info("page %s rows %s flashed", self.page, sorted(changed))
            self.sent = rows
        except TtxdError as e:
            # Start over with a whole page next time; ttxd may have lost it.
            self.sent = None
            log.warning("page %s: %s", self.page, e)


class IndexGenerator:
    def __init__(self, cfg: Config, ttxd: Ttxd):
        self.cfg = cfg
        self.ttxd = ttxd
        self.items: list[schedule.Programme] = []
        self.have_schedule = False
        self.live_pages: set[str] | None = None

    def refresh(self) -> None:
        try:
            self.items = schedule.fetch(self.cfg.api_url)
            self.have_schedule = True
        except (OSError, ValueError, KeyError, TypeError) as e:  # keep the last schedule
            log.warning("schedule: %s", e)
        try:
            self.live_pages = self.ttxd.live_pages()
        except TtxdError as e:
            log.warning("page list: %s", e)

    def rows(self, at: datetime) -> list[str]:
        current, upcoming = schedule.now_and_next(self.items, at)
        entries = directory(self.cfg.index, self.live_pages)
        return render(self.cfg.index, current, upcoming, entries)

    def wake_at(self, at: datetime) -> datetime:
        wake = at + timedelta(seconds=self.cfg.poll_secs)
        change = schedule.next_change(self.items, at)
        if change and change < wake:
            wake = change + timedelta(milliseconds=200)
        return wake


def print_page(page: str, rows: list[str]) -> None:
    print(f"+{'-' * COLS}+ {page}")
    for row in rows:
        print(f"|{preview(row).ljust(COLS)}|")
    print(f"+{'-' * COLS}+")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="ttx-pages", description=__doc__.splitlines()[0])
    ap.add_argument("config")
    ap.add_argument("--dry-run", action="store_true", help="print the pages once and exit")
    args = ap.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")

    cfg = load_config(args.config)
    ttxd = Ttxd(cfg.ttxd_url, cfg.token)
    gen = IndexGenerator(cfg, ttxd)

    if args.dry_run:
        gen.refresh()
        print_page(cfg.index.page, gen.rows(datetime.now(schedule.OSLO)))
        return 0

    if not cfg.token:
        log.error("no ttxd token: set TTXD_TOKEN or token in %s", args.config)
        return 2
    pusher = Pusher(ttxd, cfg.index.page, cfg.ttl_secs, cfg.refresh_secs)
    started = time.monotonic()
    while True:
        gen.refresh()
        if not gen.have_schedule and time.monotonic() - started < FIRST_SCHEDULE_WAIT:
            # Don't put "Ingen program" on air just because the first fetch failed.
            time.sleep(FIRST_SCHEDULE_RETRY)
            continue
        now = datetime.now(schedule.OSLO)
        pusher.push(gen.rows(now))
        delay = (gen.wake_at(now) - datetime.now(schedule.OSLO)).total_seconds()
        time.sleep(max(delay, 0.1))


if __name__ == "__main__":
    sys.exit(main())
