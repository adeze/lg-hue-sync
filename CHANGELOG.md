# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.9.0] - 2026-10-28

### Added
- Generate and serve `/openapi.json` with `utoipa` and `utoipa-axum`, covering all 16 API routes, request/response types, validation bounds and error responses. Cache the description at server startup.
- Add opt-in Bonjour HTTP advertising with dashboard and OpenAPI links. Discovery defaults to off for new and existing configurations; disabling it shuts down its mDNS thread and sockets.
- Add a dashboard Bonjour toggle and `POST /api/bonjour` to persist and apply discovery changes without restarting capture or lighting. Report the saved preference through `/api/status`; failed saves leave discovery unchanged.

### Changed
- Group Bonjour discovery and Restart Pipeline in a collapsed Maintenance panel at the bottom of the dashboard. Move Save to Disk to an accessible header glyph beside the theme control.
- Register API routes and OpenAPI metadata together while preserving existing URLs and request formats. Document plain-text JSON extraction errors alongside application JSON errors.

### Validation
- All 104 Rust tests, formatting, warnings-denied Clippy, dashboard syntax and release metadata checks passed. Native ARMv7 builds passed ELF32, little-endian ARM EABI5 soft-float, loader and glibc <= 2.28 checks.
- Before the version bump, feature builds passed on-TV binary hash, service, dashboard and OpenAPI checks. Bonjour enable/disable persisted correctly, started/stopped its mDNS thread without changing the daemon PID, and restored the original configuration byte-for-byte; discovery was left off.
- Published daemon assets are built independently by Docker CI. Third-party client discovery, multicast visibility, resource impact and physical lighting output were not validated for this release. Launcher-only IPKs and manifests remain unpublished.

## [0.8.0] - 2026-10-22

### Added
- Track Hue ownership and partial configuration changes through a bounded SSE subscription; refresh the selected layout before first output and after topology changes.
- Discover bridges through local mDNS with a rate-limited cloud fallback; recover changed addresses only for the saved Bridge ID.
- Repeat cached Hue packets at 50 Hz independently of capture cadence. Adaptive deadband still suppresses new colour-update submissions; no additional sampling is performed for repeats.

### Changed
- Pair exclusively over verified HTTPS using both official Hue root CAs and the Bridge ID. Migrate existing leaf pins only after an exact match, then accept signed certificate renewal for that identity.
- Use the separate application ID from `/auth/v1` for DTLS authentication and stream ownership. Preserve pairing credentials when area selection is still required.
- Delegate the deprecated Python pairing helper to the host Rust CLI; remove plaintext pairing and direct Hyperion mutation.
- Update architecture, operations, agent and skill guidance with recovery rules and current Hue documentation findings.

### Fixed
- Refuse automatic takeover and avoid deliberately stopping another application's Hue stream. Suspend Hue independently of Nanoleaf on ownership loss.
- Stop cached Hue output on revoked authorization, deleted areas or failed topology refresh; keep stale mappings blocked across manual Start, Restart and output re-enable.
- Use configured Hue channel IDs in the test pattern instead of inventing fixed channels.

### Validation
- All 97 Rust tests, formatting, warnings-denied Clippy, dashboard syntax and release metadata checks passed. Launcher packaging and native-build safety tests passed; launcher packages remain unpublished.
- The native v0.8.0 artifact passed on-TV SHA-256, service, verified HTTPS pairing, legacy-pin identity migration, DTLS and SSE checks with zero automatic service restarts. HDMI capture returned to 60 FPS; Home released capture and paused outputs; returning to HDMI resumed them. Manual Stop survived input changes.
- Disabling Hue left Nanoleaf and capture running; disabling Nanoleaf left Hue running. Both outputs were restored. Pairing replaced only Hue credentials/identity and preserved Nanoleaf configuration.
- Competing-owner runtime testing was intentionally omitted at the maintainer's request; ownership guards have host-test coverage only. Colour accuracy, CPU impact and packet-loss performance were not measured.
- The maintainer confirmed both Hue and Nanoleaf visibly syncing after restoration; this was a functional visual check, not a colour-accuracy or latency measurement.
- The cached Docker fallback also compiled. Published daemon assets are built independently by Docker CI and are not the TV-tested native artifact.

## [0.7.0] - 2026-10-22

### Added
- Automatically pause capture on known built-in apps/Home and retry when HDMI returns, while preserving manual Stop. Show an explicit unsupported-input state instead of repeated driver initialization attempts.
- Add macOS-native ARMv7 cross-builds using the webOS Brew compiler and a Buster/glibc 2.28 sysroot bootstrapped once from the cached builder. Validate ELF class, endianness, ARM EABI5 soft-float ABI, loader and glibc requirements before atomic artifact replacement.
- Add Rust Ares launcher packaging and explicit-device install, launch and app-list commands, preserving the Homebrew manifest and selecting the exact current package instead of a stale filename.
- Expose requested sync state, actual lifecycle state, readiness and capture failures separately; release capture resources before reporting stopped and show lifecycle transitions in the dashboard.
- Add packaging and native-build safety tests, Codex build/package actions, and an isolated cargo-zigbuild experiment with verified ARM/glibc output.

### Changed
- Make `make build` use the native macOS toolchain after successful TV service/capture validation. Keep `make build-docker` as the explicit fallback and Linux CI path; separate native and Docker toolchain update commands.
- Remove unused hard-float Cross/Cargo settings; preserve `armv7-unknown-linux-gnueabi` and the glibc 2.28 ceiling.
- Improve dashboard spacing, contrast and status presentation while keeping Glasshouse and LG Hue Sync separately owned.
- Package the launcher before full deployment mutates the TV. Keep paired configuration, checksum verification and a recoverable previous daemon binary.

### Fixed
- Keep Luna query stdin open until the asynchronous response arrives; bound reply size and process lifetime, share transport across source/power/video queries, and limit source frame-rate polling to once every two seconds.
- Preserve Faulted/error state when an explicit Stop cancels a capture Restart, including commands received during the restart delay.
- Run periodic Hue Bridge status polling in a separate worker, reject superseded results, and wait for stop cleanup before checking for app-driven resume.
- Move Hue reconnect and session cleanup to a serial background worker; discard superseded results and keep Nanoleaf and control processing running while Hue reconnects.
- Reject malformed Hue client keys without UTF-8 slicing panics, and share a five-second DTLS handshake deadline across retries and socket operations.
- Bound Nanoleaf pairing requests, response bodies, polling delays and panel-layout retrieval by the remaining pairing deadline.

### Validation
- Native v0.7.0 passed a live HDMI → app/Home → HDMI transition: hardware capture released and both rates reached zero on the unsupported source, then 60 FPS hardware capture resumed automatically and the error cleared. Binary SHA-256 matched and paired configuration stayed unchanged; all 89 Rust tests and packaging/build safety tests passed.
- A preliminary native v0.6.1 build passed TV binary checksum, unchanged configuration, dashboard and service checks, with zero restarts, 320x180/60 FPS hardware capture, and Hue/Nanoleaf connected. Subsequent built-in app tests motivated the explicit input policy. Physical lighting quality was not measured.
- The Zig candidate passed local ELF/ABI/glibc inspection only; it was not deployed. Published daemon assets are built by the Docker CI fallback and are not the TV-tested native artifact. Launcher-only IPKs and Homebrew manifests remain unpublished.

## [0.6.1] - 2026-10-08

### Added
- Add optional geometry-based spatial gradient blending for Hue and Nanoleaf, controlled by the dashboard and persisted as `spatial_blend`; default off and preserve black regions.

### Changed
- Give ambient styles explicit spatial blending: Neutral and Fast Response 0%, High Chroma 15%, Neon Contrast 8%, Dark-Scene Detail 10%, Low Stimulation 30%. Selecting a style now resets spatial blending; manual tuning remains available.
- Replace the dashboard theme dropdown with an accessible glyph button cycling system, dark and light; retain the status subtitle and remove its duplicate update-rate counter.
- Collapse Devices and Advanced tuning by default on the dashboard, matching Calibration while keeping status, sync controls and presets visible.
- Replace handwritten RGB/HSV conversions, encoded-RGB mixing and sRGB decoding with Palette 0.7.7; retain device-specific gamut handling and response controls.
- Replace the NV12 pixel loop with yuv 0.8.19 using explicit BT.601 limited-range conversion and existing buffer checks. Sample differences stay within one RGB count; normalize below-black luma to preserve legacy behaviour across scalar and SIMD paths.
- Retain fractional spatial averages, colour transforms, temporal state, and colour-step limits until output rounding, preventing small fades from stalling at 8-bit boundaries.

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
