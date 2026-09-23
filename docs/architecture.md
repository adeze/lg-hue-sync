# Architecture and device ownership

## Runtime flow

1. `src/capture/` loads a webOS capture API and publishes the newest downscaled frame.
2. `src/color/` detects active content, samples Hue zones, and applies bounded colour processing.
3. `src/hue/` activates the selected Hue v2 Entertainment configuration, establishes DTLS, and streams returned channel IDs.
4. `src/nanoleaf/` maps perimeter samples to discovered panel IDs and sends UDP frames.
5. `src/web/` serves the LAN dashboard and configuration API on port 8088.

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
- Pairing requires physical access to the Bridge or controller.
- `config.json` contains secrets and must be mode `0600` on the TV.
- Hue v2 HTTPS uses the fingerprint captured during push-link pairing; never disable verification globally.
- Never log credentials, tokens, certificate pins, or complete configuration payloads.
