# Native capture, player output and encoding

2026-11-05: read-only research; no encoder, snapshot, recording or media-control
operation was invoked. TV and lighting settings were not changed.

## Findings

- `webosbrew/hyperion-webos` implements hardware capture and network frame
  transport. Its advertised high-rate capture does not constitute a C1 hardware
  H.264 encoder implementation.
- Inspected `webosbrew/samples` media examples feed compressed elementary streams
  into hardware decoders. They are useful display/lifecycle references, not
  encoding examples.
- The public `StarfishMediaAPIs.h` declares `takeSnapshot(const char*)`. The
  installed C1 `libplayerAPIs.so.1.0.0` exports that method and a snapshot-event
  handler. This establishes a snapshot entry point, not its message contract,
  HDMI support, protection behaviour, returned pixels or sustainable frame rate.
  No continuous raw-frame callback or H.264 export was established from the
  player interfaces inspected.
- Older LG GPL driver code exposes H.264/VP8 selection, input-buffer information,
  display-engine/decoder/memory input modes, encoded-output retrieval, and encoder
  create/encode/destroy ioctls. Its physical-address and stride fields are relevant
  leads. This webOS 3.0 source is not a compatible C1 header by assumption.
- The C1 currently has a loaded `venc_hxenc` module. Module presence establishes
  neither a usable user API nor safe simultaneous operation with capture.
- OSE's documented media recorder opens a camera ID and optional audio. That
  documentation does not establish HDMI recording on retail webOS 6.

No ready-made C1 HDMI-to-H.264 implementation was found in the inspected projects
or targeted searches. This is a bounded search result, not proof none exists.

## Proposed architecture

Keep LG display playback separate from readable capture. Native Luma should use
the external-input display pipeline directly when its native integration is
qualified; it should not show an HLS round trip of the TV's own HDMI input.

QSync remains the sole capture owner. Fan out captured frames to existing
lighting, a latest-frame AI consumer and an optional encoder worker. Preserve
source timestamps. Slow AI/viewer consumers drop work instead of queueing stale
frames or blocking lighting. A buffer must be copied into owned bounded staging
memory or retained through a verified lifetime contract before capture release.
Prefer native YUV for encoding when available; do not convert RGB back to YUV
merely to satisfy a transport invented by the companion.

The encoder produces H.264 access units. A separate packager/host serves HLS for
viewing; HLS is not the timing path for local inference or lighting. Start with
video only. Audio capture, clock synchronization and codec conversion are a
separate unqualified branch; the player hearing HDMI audio does not expose PCM.

Next qualification: recover the installed C1 encoder ABI, accepted buffer types,
allocation ownership, output release, errors and teardown. Then qualify a bounded
short encode before any persistent stream or higher-resolution capture change.
Do not claim zero-copy, supported resolution or frame rate before measurement.
Player snapshots may provide diagnostics, but are not the proposed video engine.

## Sources and receipts

- [Hardware capture project](https://github.com/webosbrew/hyperion-webos)
- [Native playback samples](https://github.com/webosbrew/samples)
- [Inspected player header](https://github.com/webosbrew/webos-userland/blob/00a5f8721e15d5c10030b99b2f3e73e0c527bcce/include/starfish-media-pipeline/StarfishMediaAPIs.h)
- [Older LG encoder header](https://github.com/pr0v3rbs/WebOS/blob/8840d51d227783830947d5eeca507aa33c0fb928/webos3.0_driver/drivers/kdriver/include/venc_kapi.h)
- [OSE recorder API](https://www.webosose.org/docs/reference/ls2-api/com-webos-service-mediarecorder/)

Pinned repository trees, inspected headers and the private installed-library copy
are retained locally in `~/.local/share/lg-hue-sync/research/encoder-survey-2026-11-05/`.
