# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- Use cargo-nextest for local Rust tests and CI, with CI pinned to nextest 0.9.146.

## [0.6.0] - 2026-10-01

### Added
- Add a boot-persistent systemd timer that checks `daemon.log` every minute and clears it in place at 1 MiB without restarting sync or retaining archives.
- Add an opt-in raw NV12 centre-patch probe with read-only TV mode hints, plus selectable SDR browser patches at `/capture-patterns`; neither path writes frames or drives lights.
- Allow trusted-LAN automation, including Apple Shortcuts, to apply named dashboard presets through `POST /api/presets/{name}` without changing output trims or other controls.
- Document webOS Brew userland headers/API reference and the Buildroot source behind the native SDK.
- Record a capture colourimetry, temporal response, precision, and latency validation plan based on HyperHDR's published pipeline.

### Changed
- Leave the unverified 8-bit midtone lift off in newly generated configurations and label it accurately in the dashboard; preserve existing configuration values and the legacy JSON key.

### Fixed
- Classify capture errors with native return codes and preserved source chains; skip transient frame outages, cap automatic driver recovery at three increasing delays, and pause invalid frames until manual retry.
- Cancel capture recovery on shutdown, release queued dashboard requests before server shutdown, and terminate timed-out Luna child processes.
- Base Hue and Nanoleaf temporal smoothing and colour-step limits on elapsed time, preserving 30 FPS tuning across variable capture rates and bounding long-stall jumps.
- Validate webOS capture frame dimensions and plane lengths before conversion, keep NV12 conversion safe for callers, release partially mapped display buffers when setup fails, and remove an unverified cross-thread handle guarantee.
- Process dashboard commands while capture is paused, honor per-device output toggles on resume, and report Nanoleaf status after a successful local send.
- Reject invalid capture sizes and dashboard settings, serialize configuration read-modify-write operations across daemon, dashboard, and setup commands, and report save failures to the dashboard.

## [0.5.0] - 2026-09-29

### Changed
- Moved pairing and bridge setup commands out of the daemon entrypoint; setup now preserves an existing unreadable configuration instead of overwriting it.
- Keep Hue/Nanoleaf LAN HTTP traffic direct, reject redirects, and redact Nanoleaf token URLs from request errors; send watchdog heartbeats only when configured and at half the reported interval.
- Updated `ureq` to 3, `sd-notify` to 0.5, and `libloading` to 0.9, migrating their HTTP, notification, and library-loading call sites.
- Added a tracked Codex project environment with worktree setup and actions for dependencies, validation, webOS builds, TV transfer, and scoped Docker cache cleanup.
- Switched the Docker cross-toolchain to Rust stable and added an explicit refresh action that rebuilds its image and validates the ARM binary.

## [0.4.2] - 2026-09-24

### Changed
- Replaced per-build Debian/Rust provisioning with one cached, pinned cross-toolchain image shared by local, Codex, deployment, and CI builds.
- Added explicit compatible dependency check/update targets and ARMv7 target-build CI coverage.
- Added pull-request validation, release metadata checks, and tag-driven publication of verified ARMv7 daemon artifacts.
- Documented the external-contributor review and validation path while retaining direct-to-`main` maintainer work.

## [0.4.1] - 2026-09-23

### Changed
- Replaced independent dashboard request flags with an ordered Tokio command channel and explicit pipeline state.
- Unified Hue and Nanoleaf colour transforms in one processor while preserving device-specific geometry and protocols.
- Added bounded exponential backoff for Hue and Nanoleaf reconnection attempts.

## [0.4.0] - 2026-09-23

### Added
- Optional TV power following: pause Hue and Nanoleaf sync in standby and resume when the TV becomes active; can be disabled for manual control.
- Responsive dashboard with device pairing, Entertainment Area selection, independent outputs, themes, presets, calibration, and semantic WebMCP controls.
- Nanoleaf 4D corner, direction, and perimeter-offset alignment.
- Hue API v2 discovery that preserves grouped gradient-member identity.

### Changed
- Single-maintainer changes go directly to `main`; pull requests are opt-in.
- Deployment requires an explicit TV address and preserves existing credentials by default.
- Public runbooks now distinguish host checks, target builds, transfer, verification, rollback, uninstall, and release evidence.

### Fixed
- Hue recovery reactivates its v2 Entertainment configuration before reconnecting DTLS.
- Nanoleaf alignment remaps panel IDs without moving screen sampling coordinates.
- Device status and refresh controls are grouped with their corresponding setup controls.

## [0.3.0] - 2026-09-18

### Added
- **Nanoleaf 4D (V1) Lightstrip Integration**: High-speed binary UDP streaming on port 60222 using the `extControl` v2 protocol ported from Hyperion's battle-tested networking implementation.
- **Perimeter Edge Sampler**: Automatically generates 30+ normalized sampling zones along the TV perimeter (Left, Top, Right, Bottom) with 16:9 aspect ratio distribution, saturation-weighted dominant color extraction, and OLED near-black noise gating.
- **Unified Lockstep Dual-System Sync**: Drives both the TV perimeter Nanoleaf 4D strip and Philips Hue room surround lighting simultaneously from a single captured video frame with shared scene-cut detection.
- **`sync-hue` Dynamic Re-Sync Command**: Re-queries the Hue Bridge for updated 3D light coordinates and entertainment area selection without requiring physical button press re-pairing.
- **`pair-nanoleaf` Setup Command**: Pairs with a Nanoleaf controller on LAN via HTTP POST `/api/v1/new` and queries physical panel/segment layout.
- **`test-nanoleaf` Verification Command**: Streams a rotating rainbow test pattern to verify Nanoleaf UDP connectivity and perimeter segment sequencing.
- **3D Depth & Height Room Projection**: Maps Philips Hue 3D coordinates `[X, Y, Z]` into screen sampling zones; front-stage lights maintain tight directional focus, while rear surrounds expand into diffuse ambient reflections.

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
- Legacy 8-bit midtone lift (historically described as HDR10/Dolby Vision tone mapping; source transfer metadata was not available).
- Philips Hue Gamut C boundary clamping and CIE 1931 xy mode.
- Direct DTLS 1.2 PSK UDP streaming on port 2100.
- Automatic pushlink pairing (`pair`), test pattern (`test-pattern`), and test capture (`test-capture`) CLI commands.
