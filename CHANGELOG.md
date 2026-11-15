# Changelog

## [0.5.0](https://github.com/Frikanalen/teletext/compare/v0.4.0...v0.5.0) (2026-11-15)


### ⚠ BREAKING CHANGES

* ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.

### Features

* OP-47 ANC Multipacket output for encoders in multi-packet mode ([#32](https://github.com/Frikanalen/teletext/issues/32)) ([387b735](https://github.com/Frikanalen/teletext/commit/387b73521706053dcbefb451aafd4081524dba52))

## [0.4.0](https://github.com/Frikanalen/teletext/compare/v0.3.0...v0.4.0) (2026-11-15)


### ⚠ BREAKING CHANGES

* ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.
* ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.
* **chart:** ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.
* **ttxd:** ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.
* **ttxd:** ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.
* **ttxd:** ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.
* **ttxd:** ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.

### Features

* **chart:** optional ingress for the public teletext feed ([#16](https://github.com/Frikanalen/teletext/issues/16)) ([25b820d](https://github.com/Frikanalen/teletext/commit/25b820d3c5a3232cfb37bb990fe5ef408df4858c))
* **generator:** page 100 as a sunset that follows Oslo's sun and weather ([#18](https://github.com/Frikanalen/teletext/issues/18)) ([a2bf73f](https://github.com/Frikanalen/teletext/commit/a2bf73fba4f81ed1f14c224583e1d3644d3c7883))
* page 100 places its logo as a shared object and draws with shared glyphs ([#23](https://github.com/Frikanalen/teletext/issues/23)) ([6bfa7f5](https://github.com/Frikanalen/teletext/commit/6bfa7f55ea5d0144f0e847628995a50e118ce5ae))
* page 100 ticker for later programmes, with the ttxd fixes it needs ([#24](https://github.com/Frikanalen/teletext/issues/24)) ([83bab95](https://github.com/Frikanalen/teletext/commit/83bab9511763faef36037262fd61fe8e634ecd4b))
* **pages:** drop the SENERE ticker from page 100 ([#29](https://github.com/Frikanalen/teletext/issues/29)) ([6cc73eb](https://github.com/Frikanalen/teletext/commit/6cc73ebfabdffa7ced0d419e2b246eadc3a1989c))
* **pages:** programme schedule on page 200 and information on page 300 ([#28](https://github.com/Frikanalen/teletext/issues/28)) ([f78e73b](https://github.com/Frikanalen/teletext/commit/f78e73b60310df130c47cadbcf5434d74f63c783))
* **pages:** the real sky over Oslo on page 100, aimed at the best view ([#30](https://github.com/Frikanalen/teletext/issues/30)) ([1006d0a](https://github.com/Frikanalen/teletext/commit/1006d0a0d3a3fa3bdfbabf70351852eac01ae5e8))
* return-path monitor, and a header clock that ticks every second in VLC ([#27](https://github.com/Frikanalen/teletext/issues/27)) ([551d6e6](https://github.com/Frikanalen/teletext/commit/551d6e6a85d64445c04280573234b85d778002e7))
* sky sprites on page 100, from animation frames that carry X/26 triplets ([#26](https://github.com/Frikanalen/teletext/issues/26)) ([ad176d6](https://github.com/Frikanalen/teletext/commit/ad176d699610cd5a8250860b2c7f0696b95378dc))
* **ttxd:** declarative Level 2.5 enhancements (cells, glyphs, G3 mosaics, objects, fastext, palette) ([#20](https://github.com/Frikanalen/teletext/issues/20)) ([8626b5b](https://github.com/Frikanalen/teletext/commit/8626b5b881362eb5e7cfb1dd9960cc201b26cd36))
* **ttxd:** PNG page previews at Level 1, 1.5 and 2.5 ([#21](https://github.com/Frikanalen/teletext/issues/21)) ([921d4c4](https://github.com/Frikanalen/teletext/commit/921d4c4b82412331f5a633f6a2d50b782e3e6b59))
* **ttxd:** shared Level 2.5 assets (magazine palette, global DRCS and objects) ([#19](https://github.com/Frikanalen/teletext/issues/19)) ([aa065f5](https://github.com/Frikanalen/teletext/commit/aa065f594df34ee615832aa3f2707d35927df07e))
* **ttxd:** ticker rows and frame animation driven by ttxd's frame clock ([#22](https://github.com/Frikanalen/teletext/issues/22)) ([0d95937](https://github.com/Frikanalen/teletext/commit/0d9593763b3d64ec984cf6409346a67da3ddfe5c))
* up to three OP-47 packets per frame for 750 teletext lines/s ([#31](https://github.com/Frikanalen/teletext/issues/31)) ([a395500](https://github.com/Frikanalen/teletext/commit/a3955009b483713e752ff3a7d6cd511b2e2faa11))


### Bug Fixes

* **chart:** read the schedule from the production API ([#25](https://github.com/Frikanalen/teletext/issues/25)) ([6aee097](https://github.com/Frikanalen/teletext/commit/6aee097910bfec5d55ee8622da6ee5dfa986d522))

## [0.3.0](https://github.com/Frikanalen/teletext/compare/v0.2.0...v0.3.0) (2026-11-15)


### ⚠ BREAKING CHANGES

* ttxd's decklink_device, vanc_line and op47_lines options are gone; they move to ttx-inject.toml.

### Features

* **chart:** run the ttx-pages generator next to ttxd ([#15](https://github.com/Frikanalen/teletext/issues/15)) ([3e554bc](https://github.com/Frikanalen/teletext/commit/3e554bcc0eca3c68ff79d76fee9dc8a10b49ccb5))
* **feed:** public read-only SSE feed for the web teletext viewer ([#12](https://github.com/Frikanalen/teletext/issues/12)) ([737b6e0](https://github.com/Frikanalen/teletext/commit/737b6e0394cd479528b2bfa2d4b480698929811c))
* split the SDI output into ttx-inject and run ttxd in Kubernetes ([#14](https://github.com/Frikanalen/teletext/issues/14)) ([c2f5ab2](https://github.com/Frikanalen/teletext/commit/c2f5ab28b12cd1f0d68bd2dac368dda7c5b9f9e0))
