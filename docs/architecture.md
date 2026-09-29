# Architecture and device ownership

## Runtime flow

1. `src/capture/` loads a webOS capture API and publishes the newest downscaled frame.
2. `src/color/` detects active content, samples Hue zones, and applies bounded colour processing.
3. `src/hue/` activates the selected Hue v2 Entertainment configuration, establishes DTLS, and streams returned channel IDs.
4. `src/nanoleaf/` maps perimeter samples to discovered panel IDs and sends UDP frames.
5. `src/web/` serves the LAN dashboard and configuration API on port 8088.

`src/setup.rs` owns the pairing and bridge setup commands; `src/main.rs` owns daemon orchestration.
Existing invalid configuration files stop setup instead of being replaced with defaults.
Daemon, dashboard, and setup writes use `Config::update`: a sidecar file lock covers the latest read, field mutation, and atomic save. Network discovery completes before the lock is taken.

Dashboard actions enter the daemon through a bounded typed command channel. `src/runtime.rs`
coalesces desired start/stop state while preserving restart, reconfiguration, save, and Bridge
refresh actions. Runtime output state remains distinct from the persisted configuration.

Hue and Nanoleaf sampling share `ColorProcessor` for peak weighting, noise gating, saturation,
gamma, HDR tone mapping, temporal response, strict black, and scene-change limits. Device modules
retain only geometry and wire-protocol responsibilities. Failed output reconnections use the same
bounded exponential-backoff policy so a disconnected device cannot trigger frame-rate retries.

## Ownership boundaries

- Hue app: Entertainment Area membership and physical 3D placement.
- Hue Bridge API v2: configuration, channel IDs, grouped gradient members, and stream ownership.
- This daemon: sampling rectangles, processing, output enablement, and reconnection.
- Nanoleaf controller: token and discovered panel IDs.
- This daemon: Nanoleaf edge mapping and alignment offset/direction.

Hue v2 may group several physical gradient segments into one Entertainment channel. Preserve that grouping. Dashboard overlays describe daemon sampling; they are not additional Bridge channels.

## Security

- Dashboard is intended for a trusted LAN and has no remote authentication layer.
- Preset automation uses the same unauthenticated LAN API as the dashboard; do not expose port 8088 to the Internet.
- Pairing requires physical access to the Bridge or controller.
- `config.json` contains secrets and must be mode `0600` on the TV.
- Hue v2 HTTPS uses the fingerprint captured during push-link pairing; never disable verification globally.
- Never log credentials, tokens, certificate pins, or complete configuration payloads.

## Native API references

[`webosbrew/webos-userland`](https://github.com/webosbrew/webos-userland/tree/main) provides community headers and stub libraries for LG userland APIs; its [generated reference](https://www.webosbrew.org/webos-userland/index.html) is useful when reviewing capture bindings. Treat availability and ABI behavior as firmware-specific and keep defensive dynamic loading and frame checks in `src/capture/`.

[`HyperHDR`](https://github.com/awawa-dev/HyperHDR) is an ambient-light design reference. Its [colour engine](https://github.com/awawa-dev/HyperHDR/wiki/Infinite-color-engine) retains float precision through averaging, correction, and smoothing; its [LUT calibration](https://github.com/awawa-dev/HyperHDR/wiki/lut-calibration) first identifies the source conversion. This daemon currently assumes BT.601 limited-range NV12 in both capture paths, then repeatedly converts processing results to 8-bit RGB. Hue xy output linearizes sRGB, but Hue Direct RGB expands each 8-bit channel to 16 bits without restoring precision. The legacy `hdr_tone_mapping` setting lifts midtones in 8-bit RGB and has no capture transfer or HDR metadata; new configurations leave it off, while existing configurations preserve their setting. It is not a validated HDR10/Dolby Vision conversion.

Colour investigation order:

1. Compare captured known SDR and HDR colour patches with source values to establish NV12 range, matrix, and transfer behavior on this TV. Keep the current conversion until that evidence supports a change; do not import HyperHDR's large LUT pipeline speculatively.
2. Compare the same colour step and ramp at 20, 30, and 60 processed frames per second. Require similar wall-clock transition times and preserve strict black and scene-cut limits.
3. Record capture, processing, and successful-send timings and drops separately before attributing delay. Software timings are not capture-to-light latency; that claim needs a synchronized optical measurement.
4. Trial float colour storage through sampling and smoothing only if captured data or output shows quantization, banding, or slow-fade instability. Compare output bytes, Hue/Nanoleaf appearance, and ARMv7 CPU use before adopting it.

Keep device-specific geometry and protocols. HyperHDR's USB grabber and direct RGBW dithering do not solve a demonstrated problem in this rooted-TV and controller setup.
