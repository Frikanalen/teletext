# ttx-pages

Renders Frikanalen's studio teletext pages and keeps them up to date in
[ttxd](../docs/design.md#api). Python 3.11+, standard library only.

So far there is one page:

- **100, the index.** What is on now and next (from django-api's schedule, with
  DIREKTE flashing when the programme is live), and a directory of the other pages.
  A directory entry is listed only while ttxd is sending that page, unless it has
  `always = true`.

## Level 2.5

On a Level 2.5 decoder, page 100 gets Frikanalen's colours and the logo in pixels;
older decoders show the plain Level 1 page (see `teletext_pages/level25.py`):

- **Palette.** An X/28/0 packet redefines colour table 2 as the on-air graphics
  palette and maps the Level 1 colours onto it: black is ink, white is paper, cyan is
  amber, blue is ink-2, magenta is plum. So every page could use it without changing
  its markup.
- **Logo.** `teletext_pages/assets/logo.ppm` (204x30 teletext pixels, cut from the
  on-air graphics logo) is cut into 12x10 tiles. The 48 distinct tiles go on a hidden
  DRCS page (`drcs_page`, default `10F`), and X/26 triplets place them on rows 1-3,
  each cell coloured from colour table 3 (the logo's gradient steps).
- Set `level25 = false` under `[index]` to send a plain Level 1 page.

The banner has no double height, because a Level 1 double-height row hides the row
below it on every decoder, which would cut the logo in half.

To see what a decoder makes of it, dump ttxd's T42 debug stream and draw it with
libzvbi (`tools/t42render.c`):

```sh
nc 127.0.0.1 4242 > dump.t42      # a few seconds
t42render dump.t42 100 2.5 page.ppm
```

## Running

```sh
cd generator
python3 -m teletext_pages.main pages.example.toml --dry-run   # print page 100 and exit
TTXD_TOKEN=... python3 -m teletext_pages.main pages.toml       # run forever
python3 -m unittest                                            # tests
```

ttxd needs a publisher for the generator's pages, for example:

```toml
[[publisher]]
name = "studio"
token = "change-me-studio"
pages = ["100-1FF"]
flash_per_sec = 50
```

## How it talks to ttxd

- Every page is sent with `ttl_secs` (default 10 minutes), so if the generator dies
  its pages drop out of the carousel instead of going stale. The whole page is sent
  again every `refresh_secs` (default 2 minutes) to renew it.
- New content goes out as a flash. When only a few rows change (a programme change
  touches 5), only those rows are sent with `POST /v1/pages/{page}/flash`; a bigger
  change sends the whole page with `"flash": true`.
- The schedule is read every `poll_secs`, and the generator also wakes on the second
  a programme starts or ends, so now/next changes on air within about a second.
- If django-api is unreachable it keeps using the last schedule it had. If ttxd is
  unreachable it sends the whole page again once ttxd answers.

## Adding a page

Write a module with a `render(...) -> list[str]` that returns up to 24 rows of ttxd
markup. Use `markup.plain()` on any outside text so it cannot inject `{attributes}`,
`markup.fit()` to cut it to width, and `markup.check()` to catch rows over 40 cells.
Then give it a `Pusher` in `main.py`.
