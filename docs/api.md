# ttxd publisher API (draft v1)

This is for services that put content on teletext, such as the programme schedule
generator. It describes what `src/api.rs` does today. Things marked *(proposed)* are
not implemented.

## Basics

- Base URL: `http://<ttxd-host>:8080/v1`. In the staging cluster that is
  `http://ttxd.teletext.svc.cluster.local:8080/v1`.
- JSON in and out, UTF-8.
- Every write needs `Authorization: Bearer <token>`. Each token belongs to one
  *publisher*. The config gives each publisher its page ranges (for example `200-29F`),
  whether it may send subtitles, and a flash budget in packets/s.
- Errors come back as `{"error": "..."}` with these status codes:
  - `400` bad page number, markup or row
  - `401` missing or unknown token
  - `403` page outside your ranges, or subtitles not allowed
  - `404` no such page
- Reads (`GET`) need no token.

## Page model

- **Page number**: three characters, `100` to `8FF`. The first digit is the magazine
  (1–8). Use decimal digits (`200`–`299`) for pages viewers should key in. Hex pages
  (`2A0`…) cannot be reached from a remote, so they suit only machine use.
- **Subpages**: a page holds 1–99 subpages. The carousel shows one at a time and moves
  to the next every `cycle_secs`. Use subpages when a day's schedule does not fit on one
  screen.
- **Rows**: each subpage is a list of up to 24 strings, for display rows 1–24. Row 0 is
  the header ("FRIKANALEN dd.mm hh:mm:ss"), which ttxd owns. Rows you leave out are
  blank.
- **Cells**: a row is at most **40 cells**. Each character takes one cell, and so does
  each `{attribute}`. Longer rows give 400.

### Row markup

Text is ordinary Unicode, and attributes go in braces. An attribute takes effect from its
own cell to the end of the row (or until another attribute changes it), and the attribute
cell itself shows as a space.

| Markup | Effect |
|---|---|
| `{white}` `{yellow}` `{cyan}` `{green}` `{magenta}` `{red}` `{blue}` | text colour (`{black}` exists but is only useful with `{newbg}`) |
| `{gwhite}` … `{gblue}` | mosaic graphics in that colour, with `{contiguous}`/`{separated}` and `{hold}`/`{release}` |
| `{newbg}` | the current foreground colour becomes the background, e.g. `{blue}{newbg}{yellow}Title` |
| `{blackbg}` | back to a black background |
| `{double}` / `{normal}` | double height. The row **below** a double-height row is not displayed, so leave it empty |
| `{flash}` / `{steady}` | blinking text |
| `{conceal}` | hidden until the viewer presses Reveal |
| `{box}` / `{endbox}` | only matters on subtitle and newsflash pages |
| `{x:HH}` | raw code `00`–`7F` |

Every row starts as white text on black, with no attributes carried over from the row
above.

**Characters.** Æ Ø Å æ ø å É Ü and all of ASCII work, apart from the exceptions below.
Æ and Ø show as Ä and Ö on very old (Level 1) decoders. The characters
`@ [ \ ] ^ ` { | } ~ $` are taken over by the Swedish national option, so they come out as
`?`. Write `{{` and `}}` to get a literal brace, which then also shows as `?`. Any other
character (en dash, curly quotes, emoji) also comes out as `?`, so convert those to ASCII
first.

## Endpoints

### `PUT /pages/{page}`: store or replace a page

```json
{
  "subpages": [
    ["{double}{yellow}Sendeplan i dag", "", "{cyan}18:00{white} Nyheter", "..."],
    ["{double}{yellow}Sendeplan i dag  2/2", "", "..."]
  ],
  "cycle_secs": 15,
  "ttl_secs": 93600,
  "flash": true
}
```

| Field | Default | Meaning |
|---|---|---|
| `subpages` | required | 1–99 subpages, each a list of row markup (first entry = row 1), or an object with Level 2.5 data (see below) |
| `cycle_secs` | 10 | how long each subpage stays up (minimum 1) |
| `ttl_secs` | none | the page leaves the carousel this long after the PUT. It stays in the store, and `GET /pages` shows `"expired": true` |
| `flash` | false | send subpage 1 at once, ahead of the carousel, instead of waiting for its turn |
| `repeat_secs` | none | send the page at most once per this many seconds in the carousel. Meant for hidden data pages (DRCS, objects) that rarely change, so they don't take a full share of the ~10 pages/s |
| `flags` | none | header bits: `{"inhibit_display": true}` (C10) for pages that hold data rather than text, such as DRCS pages; `{"suppress_header": true}` (C7) |

A PUT replaces the whole page, so it is idempotent: sending the same page again does
nothing harmful. The reply is `{"page":"200","ok":true}`. Stored pages survive a ttxd
restart.

#### Level 2.5 data

A subpage can also be an object. Every field is optional:

```json
{
  "rows": ["{yellow}F R I K A N A L E N"],
  "raw_rows": { "1": [64, 64, "... 40 values 0-127"] },
  "x26": [[41, 4, 0], [2, 0, 17], [2, 13, 64]],
  "packets": [
    { "packet": 28, "designation": 0, "triplets": [[0, 0, 0], "... up to 13"] },
    { "packet": 27, "designation": 4, "triplets": ["..."] },
    { "packet": 27, "designation": 0, "raw": ["... 39 coded bytes"] }
  ]
}
```

| Field | Meaning |
|---|---|
| `rows` | row markup, as in the plain form |
| `raw_rows` | row number (`"1"`–`"24"`) to 40 raw 7-bit codes, replacing that row. ttxd adds the parity. For DRCS and object pages |
| `x26` | X/26 enhancement triplets `[address, mode, data]` in active-position order, without a termination marker. ttxd merges its own Æ/Ø substitutions into them in position order, Hamming 24/18 codes them, and sends up to 16 packets |
| `packets` | X/27 and X/28 packets: up to 13 `triplets` (Hamming 24/18 coded by ttxd), or 39 already coded `raw` bytes (for X/27/0–3 links, which are Hamming 8/4) |

The enhancement packets go out after the header and before the rows, in the order X/27,
X/28, X/26. They go out with every carousel pass and every flash, including row flashes.
ttxd doesn't interpret them; the generator in `generator/` builds them (see
`generator/teletext_pages/level25.py`). Older decoders ignore them, so the rows must stand
on their own as a Level 1 page.

### `POST /pages/{page}/flash`: change some rows now

```json
{ "rows": { "3": "{green}NÅ  {white}18:00 Nyheter" }, "subpage": 0 }
```

This replaces the given rows (keys `"1"`–`"24"`) in the stored page, and sends just those
rows to viewers within a frame or two. No full page is resent and the screen doesn't
blank. `subpage` is 0-based. Use it for a "now on air" marker or a clock. A flash costs one
packet for the header plus one per row, counted against your flash budget (the default is 100 packets/s). If you go
over budget, flashes are delayed, not dropped.

### `DELETE /pages/{page}`

Removes the page.

### `GET /pages`

Lists every page with these fields: `page`, `owner`, `subpages`, `updated`, `expires`
and `expired`. Times are Unix seconds.

### `GET /pages/{page}`

Returns the page's settings plus `preview`: for each subpage, 24 plain-text rows with
attributes shown as spaces. Use it to check that your layout fits.

### `GET /status`

`{"pages": n, "stats": {"frames", "carousel_packets", "flash_packets", "subtitle_packets", "filler_packets", "service_data_packets"}}`.
`frames` goes up by 50 every second while the SDI output runs.

### `POST /subtitles`

Subtitles, not schedule pages: `{"lines": ["…"], "page": "888"}`. It takes up to 3 lines
of about 35 characters each, and `[]` clears the subtitle. The token needs
`subtitles = true`.

### `PUT /status_display`

`{"text": "Nyheter kl 18"}` sets the status display in packet 8/30. That's up to 20
characters, which many decoders show as the channel or programme name. `""` resets it
to the service name. The token needs `status_display = true`. The value is not kept
across restarts.

ttxd sends packet 8/30 (format 1) once a second. Besides the status display it carries
the initial page (`initial_page` in the config, default 100), the network identification
code (`network_id`, default 0 since Frikanalen has none registered), and the date and
UTC time with the local offset, which some decoders use to set their clock.

## Bandwidth

The whole service has about 250 packets/s, and a full page is about 26 packets. With N
pages in the carousel, each page comes round about every N/10 seconds. That works out to
roughly 2 s for 20 pages and 5 s for 50. A viewer who keys in a page waits half that on
average. Blank rows still cost a packet in the carousel, so short pages do not make the
cycle faster.

## Advice for a schedule generator

- Own one range, e.g. `200`–`2FF` for "Sendeplan". For example: `200` index or today,
  `201` tomorrow, `202`–`207` the days after.
- PUT the day pages whenever the schedule changes and once after midnight, with
  `ttl_secs` running a little past the end of that day. Then a dead generator leaves no
  stale schedule behind.
- To mark what is on air, use `POST /flash` on one or two rows when the programme
  changes. Don't PUT the whole page every minute.
- Leave row 24 free if you want room for a fastext-style footer later. Fastext links
  are X/27/0 packets, which you can send as `raw` packets.
- A 40-cell row fits `{cyan}HH:MM{white} ` (7 cells) plus 33 characters of title, so
  shorten long titles yourself. ttxd rejects a row that is too long. It does not
  truncate it.

## Not in v1 *(proposed)*

- M/29 magazine-wide enhancements, and packet 8/30 format 2 (PDC programme labels).
- `If-Match`/ETag on PUT, and bulk PUT of several pages in one request.
- An OpenAPI document.
