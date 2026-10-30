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

`GET /api/status` distinguishes `requested_running` (the most recently accepted HTTP
start/stop intent) from `pipeline_state` (`starting`, `running`, `stopping`, `paused`,
`recovering`, or `faulted`). The compatible `is_syncing` field is true only after a
real hardware frame is received and false during pause, recovery, and faults. Output
connection flags and `light_update_fps` remain separate evidence of light streaming.
`last_error` reports the latest capture failure and clears after a valid hardware frame
or a new start/restart. A queued control response acknowledges intent, not completion.
Read status after commands to observe the transition. A full or closed command channel
does not change accepted intent.

Pause and shutdown drop the capture owner through its existing RAII teardown before
reporting `paused`. Hardware release still requires on-TV verification. Resume creates a fresh capture instance; a restart
while intentionally paused retains that pause and does not acquire hardware. An explicit
Start or Restart retries a capture fault. Standby startup does not initialize capture.

`src/hue/reconnect.rs` serializes Hue activation, DTLS connection and cleanup on a dedicated
worker. Generation checks discard results superseded by stop, disable or area changes; retries
do not block Nanoleaf or command processing. DTLS handshakes share a five-second deadline;
Bridge HTTP operations retain their five-second deadline. Shutdown joins the worker after
invalidating pending connections. One bounded SSE worker subscribes before taking its initial
snapshot and merges partial updates. Reconnection reconciles ownership and layout; generation
checks discard old subscriptions. Authoritative deletion or rejected authorization blocks Hue;
transient event transport errors retry with bounded backoff. Layout changes stop cached output
before refresh; a failed refresh remains blocked until a successful refresh or area selection.
Ownership loss suspends Hue without stopping Nanoleaf. Manual Start can retry ownership loss.
Explicit area-refresh requests still run synchronously in the daemon loop.

Hue's application key authorizes HTTPS; the separate application ID from `/auth/v1` identifies
DTLS and `active_streamer`. START refuses another owner, and STOP checks that the stream is
ours. Hue offers no conditional ownership mutation, so the check and STOP cannot be atomic
against another app. Do not use automatic stop/start to force a takeover.

A dedicated output worker repeats the latest packet at 50 Hz, independently of capture FPS.
Adaptive deadband suppresses new colour-update submissions; it does not currently reduce
capture or sampling work. Repeats do not resample colours. Pause, layout invalidation and
shutdown drop this worker before releasing the session. `light_update_fps` measures submitted
colour updates, not the cached packet repetition rate or physical lamp updates.
Nanoleaf pairing limits each HTTP request to three seconds or the remaining pairing budget,
whichever is shorter, including response bodies and the subsequent panel-layout request.

Hue and Nanoleaf sampling share `ColorProcessor` for peak weighting, noise gating, saturation,
gamma, the legacy midtone lift, temporal response, strict black, and scene-change limits.
Spatial averages, colour transforms, and temporal state retain fractional RGB until output rounding.
These values remain in the existing encoded RGB domain; this does not establish HDR decoding.
[Palette](https://docs.rs/palette/0.7.7/palette/) owns RGB mixing, RGB/HSV conversions, and
the sRGB decoding used before the existing Hue-specific XYZ matrix. Saturation remains
multiplicative in HSV; smoothing, black gating, step limits, and Hue gamut clipping remain local.
[yuv](https://docs.rs/yuv/0.8.19/yuv/) owns validated NV12-to-RGBA conversion with explicit
BT.601, limited range, and Professional precision. Dimension, stride, buffer, and overflow checks
remain before conversion. Sampled values differ from the legacy coefficients
by at most one RGB count. Luma below 16 is normalized to video black before conversion,
preserving the legacy behaviour and avoiding scalar/SIMD footroom differences. Normal frames
are borrowed; active below-black luma or minimal final rows trigger a packed copy.
Packing also prevents the dependency from skipping an incomplete stride-sized final row. Final channels
are clipped to 0–255. Neither crate identifies capture colourimetry.
Optional named-colour, parallel, and platform-specific YUV feature sets are disabled.
Optional spatial blending mixes nearby zone centres within 0.35 normalized screen units, using
distance-weighted RGB interpolation capped at 50%. It preserves black regions, uses a snapshot
to avoid ordering bias, and never feeds blended colours back into temporal state. Channel IDs
and panel membership remain unchanged. Device modules
retain only geometry and wire-protocol responsibilities. Failed output reconnections use the same
bounded exponential-backoff policy so a disconnected device cannot trigger frame-rate retries.

## Ownership boundaries

- Hue app: Entertainment Area membership and physical 3D placement.
- Hue Bridge API v2: configuration, channel IDs, grouped gradient members, and stream ownership.
- This daemon: sampling rectangles, processing, output enablement, and reconnection.
- Nanoleaf controller: token and discovered panel IDs.
- This daemon: Nanoleaf edge mapping and alignment offset/direction.

Hue v2 may group several physical gradient segments into one Entertainment channel. Preserve that grouping. Dashboard overlays describe daemon sampling; they are not additional Bridge channels.

Hue sampling uses each channel's authoritative centre, with X mapped horizontally and Z vertically.
The base footprint is 25% of each screen axis before clipping. `depth_influence` (0–1, default 0.5)
controls bounded expansion: for normalized rearward depth `d = clamp((1 - Y) / 2)`, the span is
`0.25 * (1 + 0.5 * depth_influence * d*d*(3 - 2*d))`. Off stays at 25%; Mild reaches 31.25%;
Strong reaches 37.5%. This is QSync policy, not a Hue-prescribed projection or measured beam size.
The setting applies to Hue only and is saved independently of colour/response/blend presets.
Changes update sampling and reported rectangles without reconnecting Hue or resetting temporal state.
Legacy/manual rectangles without `hue_channel_position` retain their bounds until a Hue layout refresh
supplies authoritative channel coordinates. Device preview boxes enclose member rectangles; empty gaps
inside that enclosure are not additional sampled pixels. Edge clipping can reduce the visible footprint.

## Security

- Dashboard is intended for a trusted LAN and has no remote authentication layer.
- Preset automation uses the same unauthenticated LAN API as the dashboard; do not expose port 8088 to the Internet.
- Pairing requires physical access to the Bridge or controller.
- `config.json` contains secrets and must be mode `0600` on the TV.
- Hue HTTPS trusts only the two official Hue root CAs and checks the certificate CN against the saved Bridge ID before sending credentials. An existing leaf pin must match exactly to bootstrap a missing Bridge ID. Signed renewal is accepted only for that saved identity; changed identities require explicit pairing. No plaintext fallback is permitted.
- Never log credentials, tokens, certificate pins, or complete configuration payloads.

## Native API references

Foreground source policy and user intent are separate. `CaptureSourcePolicy`
auto-pauses known non-HDMI apps/Home, releases capture/output sessions, and
resumes on HDMI only when the policy caused the pause. Manual Stop cancels that
resume; failed observations retain the last known compatibility. The dashboard's
`unsupported_source` state is a policy decision, not DRM detection.
All Luna observations share `src/luna.rs`, with a 1.5-second deadline, 64 KiB
reply bound and stdin held until reply to avoid this firmware's EOF behavior.

[`webosbrew/webos-userland`](https://github.com/webosbrew/webos-userland/tree/main) provides community headers and stub libraries for LG userland APIs; its [generated reference](https://www.webosbrew.org/webos-userland/index.html) is useful when reviewing capture bindings. Treat availability and ABI behavior as firmware-specific and keep defensive dynamic loading and frame checks in `src/capture/`.

Use the [DILE VT reference](https://www.webosbrew.org/webos-userland/df/d73/dile__vt_8h.html)
and [Hyperion's native backends](https://github.com/webosbrew/hyperion-webos/tree/main/unicapture/backends)
as lifecycle references. The existing vtCapture owner performs stop, postprocess, finalize,
and release; the DILE owner stops, unmaps buffers, and destroys its handle. Current DILE
headers and older backend examples disagree on some function signatures, including Stop.
Do not replace firmware-tested bindings solely to match generated documentation. Native
stubs describe an ABI; they neither implement capture nor prove runtime availability.

[`HyperHDR`](https://github.com/awawa-dev/HyperHDR) is an ambient-light design reference. Its [colour engine](https://github.com/awawa-dev/HyperHDR/wiki/Infinite-color-engine) retains float precision through averaging, correction, and smoothing; its [LUT calibration](https://github.com/awawa-dev/HyperHDR/wiki/lut-calibration) first identifies the source conversion. This daemon currently assumes BT.601 limited-range NV12 in both capture paths, then repeatedly converts processing results to 8-bit RGB. Hue xy output linearizes sRGB, but Hue Direct RGB expands each 8-bit channel to 16 bits without restoring precision. The legacy `hdr_tone_mapping` setting lifts midtones in 8-bit RGB and has no capture transfer or HDR metadata; new configurations leave it off, while existing configurations preserve their setting. It is not a validated HDR10/Dolby Vision conversion.

Colour investigation order:

1. Compare captured known SDR and HDR colour patches with source values to establish NV12 range, matrix, and transfer behavior on this TV. Keep the current conversion until that evidence supports a change; do not import HyperHDR's large LUT pipeline speculatively.
2. Compare the same colour step and ramp at 20, 30, and 60 processed frames per second. Require similar wall-clock transition times and preserve strict black and scene-cut limits.
3. Record capture, processing, and successful-send timings and drops separately before attributing delay. Software timings are not capture-to-light latency; that claim needs a synchronized optical measurement.
4. Trial float colour storage through sampling and smoothing only if captured data or output shows quantization, banding, or slow-fade instability. Compare output bytes, Hue/Nanoleaf appearance, and ARMv7 CPU use before adopting it.

Keep device-specific geometry and protocols. HyperHDR's USB grabber and direct RGBW dithering do not solve a demonstrated problem in this rooted-TV and controller setup.

## Hue reference review (2026-10-22)

- [Streaming guide](https://developers.meethue.com/develop/hue-entertainment/hue-entertainment-api/): 50–60 Hz is a packet-loss resilience recommendation; ten seconds of inactivity closes the stream. It is not a 60 Hz physical-light update guarantee.
- [Core concepts](https://developers.meethue.com/develop/hue-api-v2/core-concepts/#representations): resource representations can lag hardware; REST state and successful UDP writes do not prove physical light output. SSE events are partial changes and can coalesce within one second.
- [HTTPS](https://developers.meethue.com/develop/application-design-guidance/using-https/): both Hue root CAs, certificate chain verification and Bridge ID verification are required; no HTTP fallback.
- [Colour conversion](https://developers.meethue.com/develop/application-design-guidance/color-conversion-formulas-rgb-to-xy-and-back/): decode sRGB, convert to XYZ/xy, derive brightness from Y, and clip against the light's reported gamut. The page's prose matrix and legacy SDK extract differ. This change retains the existing Hue-specific matrix and Gamut C output option; it does not claim per-light gamut discovery or calibrated display matching.
- [Discovery](https://developers.meethue.com/develop/application-design-guidance/hue-bridge-discovery/): local mDNS first, cloud fallback limited to one attempt per fifteen minutes per process, manual IP on ambiguity. Paired address recovery selects only the saved Bridge ID after TCP connection failure; TLS/authorization errors never trigger a different bridge selection. Separate short-lived CLI processes do not share the cloud cache.
- [Bridge Pro migration](https://developers.meethue.com/multi-bridge-migration-support-is-live/): replacement credentials may require re-pairing, and migrated resource IDs may require explicit area selection. No name-based automatic remapping.
