# Teletext for Frikanalen: design

## Signal path

```
publishers (django-api schedule, bulletins, infographics, live subtitles)
        │  HTTP/JSON, bearer token per publisher
        ▼
   ttxd on sdi-test1 ──► DeckLink Duo 2 SDI out (720p50 filler frames + OP-47 VANC)
                                   │
                         merged with programme downstream
                                   ▼
                     encoding appliance (OP-47 VANC mode, single packet)
                                   ▼
                     DVB teletext PID (EN 300 472) in the transport stream
```

sdi-test1 carries no programme video. It produces a data-only SDI signal, and the
teletext is merged with the programme further down the chain.

## Why OP-47

720p has no vertical blanking lines for teletext the way 625-line SD does. In HD-SDI,
WST teletext is carried in VANC as OP-47 (SMPTE RDD-08): ANC packet DID 0x43, SDID 0x02,
holding one *Subtitling Distribution Packet* (SDP) per frame with up to 5 teletext
lines. At 50 frames/s that is 250 teletext packets/s, roughly a third of a full SD VBI
allocation. A page is about 25 packets, so about 10 pages/s.

## ttxd

One Rust process with four parts.

1. **Page store.** Pages `100`–`8FF`, each with up to 99 subpages, an owner, a subpage
   cycle time and an optional TTL. It is written to a JSON snapshot so a restart comes
   back with the same pages. There is no database.
2. **Scheduler.** It fills each frame's 5 lines in three tiers:
   1. **Subtitles** (page 888 by default). Each cue is a fresh page: the header has
      erase, subtitle and suppress-header set, and the rows are double height and boxed.
      The rows follow in the next frame (decoders need time to erase). Only the newest
      cue is kept, so a cue reaches the wire within about 2 frames.
   2. **Flash.** A publisher can push a whole page or only some rows ahead of the
      carousel. A row-only flash sends the header with the update flag and no erase, so
      a clock or "on air now" line costs 2–3 packets. Each publisher has a flash budget
      (a token bucket, in packets/s), so a chatty module cannot starve the carousel.
   3. **Carousel.** It cycles all live pages in order and rotates subpages. Expired
      pages drop out.

   Magazines are sent in parallel mode (C11 = 0). A page ends at the next header of the
   *same* magazine, so a page is never interleaved with another page of its own
   magazine. A higher tier that needs a busy magazine preempts the lower tier's page,
   which is simply sent again on the next cycle. When there is nothing to send, a
   time-filling header (page xFF) goes out, which keeps decoders' clocks running.
   Headers are built at send time, so the clock in them is always current.
3. **Encoder.** It produces WST packets (ETS 300 706): Hamming 8/4 addresses and
   control bits, and odd parity on text. It uses the Swedish/Finnish/Hungarian national
   option (C12–C14 = 010), which gives Å, Ä, Ö, É and Ü. WST has no Danish/Norwegian
   option, so Æ, Ø, æ and ø are sent as Ä, Ö, ä and ö for Level 1 decoders, plus an X/26
   enhancement that puts the real letter from the Latin G2 set into the cell on Level 1.5
   and later decoders.
4. **Outputs.**
   - The OP-47 SDP for each frame, to the DeckLink (output thread still to come; see below).
   - A raw T42 stream over TCP (42-byte records) for debugging with standard teletext tools.
   - Page previews over the API.

The output must never stall. If the scheduler has nothing to send, the frame goes out
with an empty or filler-only packet, and the SDI signal itself never stops.

## API

| Method | Path | Body |
|---|---|---|
| GET | `/v1/status` | |
| GET | `/v1/pages` | |
| GET | `/v1/pages/{page}` | (returns plain-text preview) |
| PUT | `/v1/pages/{page}` | `{"subpages": [["row 1", "row 2"]], "cycle_secs": 10, "ttl_secs": 3600, "flash": false}` |
| DELETE | `/v1/pages/{page}` | |
| POST | `/v1/pages/{page}/flash` | `{"rows": {"5": "Nå: Nyheter"}, "subpage": 0}` |
| POST | `/v1/subtitles` | `{"lines": ["Første linje", "andre"], "page": "888"}`, `[]` clears |

Writes need `Authorization: Bearer <token>`. Each publisher is configured with its page
ranges, whether it may send subtitles, and its flash budget.

Row markup is Unicode text with spacing attributes in braces: `{red}`, `{green}`,
`{yellow}`, `{blue}`, `{magenta}`, `{cyan}`, `{white}`, `{gred}`…`{gwhite}` (mosaic
colours), `{double}`, `{normal}`, `{flash}`, `{steady}`, `{box}`, `{endbox}`, `{newbg}`,
`{blackbg}`, `{conceal}`, `{hold}`, `{release}`, `{contiguous}`, `{separated}`, and
`{x:HH}` for a raw code. Each attribute takes one cell. A row is at most 40 cells.

## DeckLink output (next step)

A dedicated thread does scheduled playback through the Desktop Video SDK (a thin C++
shim bound with the `cxx` crate): a constant filler frame, with each frame's SDP
attached through `IDeckLinkVideoFrameAncillaryPackets`. The SDK's frame-completion
callback becomes the frame clock, replacing the 50 Hz timer in `main.rs`. Reference
lock comes later.

**Verification plan.** Loop a Duo 2 output into one of its inputs and capture with
FFmpeg's DeckLink input, which can extract OP-47 into a DVB teletext stream. Then check
the result against the encoding appliance's output.

## Still to verify

- RDD-08 SDP details against the spec and the appliance: the descriptor byte (field
  bit, line numbers) and the length and footer fields. `src/op47.rs` documents what we
  assume.
- The X/26 Æ/Ø substitution on a real Level 1.5 decoder.
- Packet 8/30 (network ID and time) is not sent yet.
- The host time zone must be Europe/Oslo, because the header clock uses local time.

## Deployment

A single static binary with a systemd unit and `/etc/ttxd/ttxd.toml` (see
`ttxd.example.toml`). Publisher tokens come from OpenBao through the Ansible role in
`infra`.
