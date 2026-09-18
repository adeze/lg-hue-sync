# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-09-18

### Added
- **Dynamic Letterbox / Aspect Ratio Auto-Detection**: Real-time row-variance analysis detecting top/bottom black bars on 2.39:1 / 2.0:1 anamorphic content and re-anchoring zone coordinates to the active film frame.
- **Saturation-Weighted Dominant Color Sampling**: Non-linear chroma saturation weighting ($1.0 + \gamma \cdot S^2$) preventing bright accent colors from being diluted by neutral background noise.
- **Scene-Cut Instant Snap & Adaptive EMA**: Global color difference tracking snapping instantly ($\alpha = 1.0$) on hard camera cuts while maintaining smooth ($\alpha = 0.35$) temporal tracking during camera pans.
- **OLED Near-Black Noise Gate**: Suppresses video compression dither and near-black artifacts ($< 2\%$ luminance) to true zero to eliminate distracting bias light flicker.
- **Adaptive Deadband Throttling**: Reduces UDP transmission to a 2 Hz keep-alive heartbeat when the video is paused or static, slashing Bridge load and WiFi airtime by 90%.
- **systemd Watchdog & Status Integration**: Integrated `sd-notify` with `WatchdogSec=10` and live status reporting (`systemctl status lg-hue-sync`).
- **Dynamic libvtcapture Loading**: Uses `libloading` for dynamic runtime resolution of `/usr/lib/libvtcapture.so`.

## [0.1.0] - 2026-09-17

### Added
- Initial release of headless Rust sync daemon for LG C1 webOS Smart TVs.
- Zero-copy display frame capture via `libvtcapture.so` hardware scaler.
- Reinhard OLED HDR10 and Dolby Vision tone-mapping.
- Philips Hue Gamut C boundary clamping and CIE 1931 xy mode.
- Direct DTLS 1.2 PSK UDP streaming on port 2100.
- Automatic pushlink pairing (`pair`), test pattern (`test-pattern`), and test capture (`test-capture`) CLI commands.
