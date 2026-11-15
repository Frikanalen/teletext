# ttx-pages

Renders Frikanalen's studio teletext pages and keeps them up to date in
[ttxd](../docs/design.md#api). Python 3.11+, standard library only.

There are three pages:

- **100, the index.** The top third is the sky to the south of Oslo right now
  (`teletext_pages/sky.py`, `teletext_pages/astro.py`): east on the left, west on the
  right, from the horizon to 66 degrees up. The view is aimed at the most there
  is to see (`sky_follows = false` keeps it to the south): by day south, or as near
  south as keeps the sun in the picture; at night the direction where the stars, the
  planets and the moon add up to most, with a bonus for a whole Orion, Karlsvogna or
  Summer Triangle. It turns 15 degrees every five minutes towards that, stays unless
  another view is clearly better. `sky_label = true` shows which way it looks on
  the top row. By day a sliced sun, as on an 80s
  poster, stands in the sun's direction: big and banded yellow, red and pink while it
  rises and sets, small and yellow at midday. At night the 70 brightest stars, Venus,
  Mars, Jupiter and Saturn and the moon in its phase are each where they really are,
  to the sixel (about 3 degrees): stars by brightness and tinted by their kind, the
  brightest out first at dusk. They step every five minutes. The weather from
  MET Norway is drawn over it (`teletext_pages/weather.py`): as many clouds as the
  cloud cover says, drifting with the wind, rain or snow as dense as it falls, fog
  and lightning. Under the horizon come the logo in block graphics,
  what is on now and next (from django-api's schedule, with DIREKTE flashing when the
  programme is live), and four chips in the colours of the remote's keys with the
  numbers and names of other pages. A chip is shown only while ttxd is sending its
  page, unless the entry has `always = true`, and an X/27/0 packet
  (`teletext_pages/fastext.py`) links the chips to the red, green, yellow and cyan
  keys.

- **200, the programme schedule** (`teletext_pages/pages.py`, `[schedule]`). From
  the programme on air (in cyan) onwards, through tomorrow: time, title and the
  organisation, eight programmes to a subpage and a new subpage for each day.
- **300, information** (`[info]`). Short sections about the channel, each under a
  heading tab. The text comes from `[[info.section]]` (`heading`, `text`), with a
  default about Frikanalen; what does not fit goes on further subpages.

200 and 300 are the pages to name as "programme schedule page" and "additional
information page" in the DVB teletext descriptor. They share a frame: the name in
block letters over three bars in the logo's colours, and colour keys that lead back to
the index. On Level 2.5 they take page 100's palette and slanted glyphs.

The page is drawn as one picture in sixels (`teletext_pages/art.py`). A search over
each row decides where the colour attributes go, using foreground and background
colours together and hold graphics, so colours meet without the blank cell a change
of colour normally costs. Column 0 is always black, because a row starts white on
black and the first attribute needs a cell.

## Level 2.5

On a Level 2.5 decoder, page 100 gets more; older decoders show the plain Level 1
page (see `teletext_pages/level25.py`):

- **Palette.** An X/28/0 packet redefines colour table 2 as the page's own colours
  and maps the Level 1 colours onto it. Colour table 3 holds the sky.
- **Sky.** Each of the sky's eight rows gets its own colour (an X/26 full row colour
  and background), a gradient that follows the sun: near black at night, plum to red
  at dusk, cooler and pinker at dawn, blue by day, and greyer under cloud.
- **Logo.** The mark (`assets/mark.ppm`, 56x45 teletext pixels) and the wordmark
  (`assets/wordmark.ppm`, 355x27) are cut into 12x10 tiles and drawn over the block
  logo on rows 8-12. The tiles go on a hidden DRCS page (`drcs_page`, default `10F`),
  48 per subpage and each row from one subpage. The logo itself is a shared object in
  ttxd (`PUT /assets`, which needs a token with `assets = true`): the page places it
  with two X/26 triplets, and so can any other page that names the same `drcs_page`.
  With a token that may not set assets the page carries the logo cell by cell instead,
  which uses most of its X/26 room. The DRCS page goes round at most every 8
  seconds (ttxd's `repeat_secs`), so a TV that tunes in has the whole logo within
  about half a minute. `tools/logo_assets.py` draws both assets from the logo's vector
  paths: a pixel is on when the shape covers half of it or more.
- **Finer shapes.** With the logo out of the way there is room for shared glyphs
  (ttxd's global DRCS page): the slanted ends of the heading tabs and key chips become
  straight diagonals, and the sky's lone stars, raindrops and snowflakes are drawn as
  such instead of as blocks. This generator owns ttxd's shared assets: a `PUT /assets`
  replaces them all.
- **Colour keys.** The chips are forced to the standard red, green, yellow and cyan of
  colour table 0, so they still match the remote.
- Set `level25 = false` under `[index]` to send a plain Level 1 page.

## Sprites

On a Level 2.5 decoder the sky moves: stars twinkle, and snow and rain fall a few
pixels a step, on into the cell below where that is empty sky. The generator sends the
page's X/26 triplets for each of six frames (`animation.x26` in docs/api.md) and ttxd
steps through them, every `sprite_ms` (500 by default) under `[index]`. Each step
is the page's header and its X/26-28 packets, about 20 packets a second of the flash
budget at that pace. Set `sprites = false` to keep the sky still. A ttxd too old for it gets
the page without the animation.

Level 2.5 DRCS is one bit per pixel and a cell has one foreground colour, so the logo
cannot be anti-aliased there. The four-colour DRCS that could do it is Level 3.5.

Nothing on the page is double height, because a Level 1 double-height row hides the
row below it on every decoder.

To see what a decoder makes of it, dump ttxd's T42 debug stream and draw it with
libzvbi (`tools/t42render.c`):

```sh
nc 127.0.0.1 4242 > dump.t42      # a few seconds
t42render dump.t42 100 2.5 page.ppm
```

libzvbi applies the X/28 palette at every level, so for what an old TV shows, drop the
X/26-X/28 packets from the dump before drawing it at level 1. libzvbi also ignores
the X/28 "black background substitution" bit, which is why the sky sets its
background with X/26 as well as the row colour.

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
- The weather is asked for every ten minutes, with the User-Agent MET requires. If
  MET is unreachable the last weather stays for three hours, then the sky is clear.
  The generator needs to reach `api.met.no`; set `weather = false` under `[index]`
  to leave the weather out.
- If django-api is unreachable it keeps using the last schedule it had. If ttxd is
  unreachable it sends the whole page again once ttxd answers.

## Adding a page

Write a module with a `render(...) -> list[str]` that returns up to 24 rows of ttxd
markup. For a page of text, build the rows with `markup.py`: `plain()` on any outside
text so it cannot inject `{attributes}`, `fit()` to cut it to width, and `check()` to
catch rows over 40 cells. For a page with art, draw into an `art.Picture` and lay the
text over it. Then give it a `Pusher` in `main.py`.
