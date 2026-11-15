# teletext

`ttxd` is Frikanalen's teletext server. It keeps a store of teletext pages that other
services update over HTTP. Every frame it schedules subtitles, flash updates and the
page carousel into one OP-47 (SMPTE RDD-08) packet, which goes out in the VANC of the
DeckLink SDI output on sdi-test1.

See [docs/design.md](docs/design.md) for the design and the API.

```sh
cargo test
cargo run -- ttxd.example.toml
curl -XPUT localhost:8080/v1/pages/200 -H 'authorization: Bearer change-me' \
  -H 'content-type: application/json' \
  -d '{"subpages": [["{double}{yellow}Sendeplan", "", "18:00 Nyheter"]]}'
```

The studio pages (the index on 100 for now) are rendered by a small Python generator
in [generator/](generator/README.md), which pushes them to ttxd through this API.
