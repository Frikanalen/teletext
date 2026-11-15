# teletext

`ttxd` is Frikanalen's teletext server. It keeps a store of teletext pages that other
services update over HTTP, and schedules subtitles, flash updates and the page
carousel frame by frame. It runs in Kubernetes. `ttx-inject` is the small SDI side on
sdi-test1: the DeckLink card's frame clock asks ttxd for each frame's packets, and
ttx-inject sends them as OP-47 (SMPTE RDD-08) in the VANC of the SDI output.

See [docs/design.md](docs/design.md) for the design and [docs/api.md](docs/api.md) for the publisher API.

```sh
cargo test
cargo run --bin ttxd -- ttxd.example.toml
curl -XPUT localhost:8080/v1/pages/200 -H 'authorization: Bearer change-me' \
  -H 'content-type: application/json' \
  -d '{"subpages": [["{double}{yellow}Sendeplan", "", "18:00 Nyheter"]]}'
```

The studio pages (the index on 100 for now) are rendered by a small Python generator
in [generator/](generator/README.md), which pushes them to ttxd through this API.

## Releases

release-please keeps a release PR open on main, built from the Conventional Commit
messages. Merging it bumps `Cargo.toml`, updates `CHANGELOG.md`, tags `vX.Y.Z` and
creates the GitHub release. GitHub Actions then builds `ttx-inject` with DeckLink
output on Ubuntu 24.04, attaches `ttx-inject-x86_64-linux-gnu` and its `.sha256` to the
release, and publishes the ttxd image as `ghcr.io/frikanalen/ttxd:vX.Y.Z`. infra's
Ansible installs ttx-inject on the SDI host; the chart in `chart/` runs ttxd. To
rebuild a release, run the Release workflow by hand with its tag.

The DeckLink SDK's Linux headers are vendored in `vendor/decklink-sdk/include` under the
licence at the top of each file; see `vendor/decklink-sdk/NOTICE`.
