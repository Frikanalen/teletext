# Changelog

## [0.3.0](https://github.com/Frikanalen/teletext/compare/v0.2.0...v0.3.0) (2026-11-15)


### ⚠ BREAKING CHANGES

* ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.

### Features

* **chart:** run the ttx-pages generator next to ttxd ([#15](https://github.com/Frikanalen/teletext/issues/15)) ([3e554bc](https://github.com/Frikanalen/teletext/commit/3e554bcc0eca3c68ff79d76fee9dc8a10b49ccb5))
* **feed:** public read-only SSE feed for the web teletext viewer ([#12](https://github.com/Frikanalen/teletext/issues/12)) ([737b6e0](https://github.com/Frikanalen/teletext/commit/737b6e0394cd479528b2bfa2d4b480698929811c))
* split the SDI output into ttx-inject and run ttxd in Kubernetes ([#14](https://github.com/Frikanalen/teletext/issues/14)) ([c2f5ab2](https://github.com/Frikanalen/teletext/commit/c2f5ab28b12cd1f0d68bd2dac368dda7c5b9f9e0))
