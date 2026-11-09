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

### C1 ABI comparison, 2026-11-09

PlxNative includes a standalone hardware-encoder diagnostic, separate from its
installed player and software UI-streaming encoder. Its
[`tv-capture-bench.c`](https://github.com/GLinnik21/plx-native/blob/8e486d1a218ae46f41a0bd4f3fa102d929de1ff0/tools/tv-capture-bench.c)
feeds captured NV12 to `HAL_VENC_CODEC_Encode` and serves H.264 over TCP. The
upstream ABI evidence is from a 49SM9000PLA running webOS 4.5; working C1 playback
does not exercise this separate encoder diagnostic.

Read-only copies of the installed C1 libraries establish that all four functions
(`Create`, `Encode`, `Destroy`, `ReleaseFrameData`, prefixed `HAL_VENC_CODEC_`)
exist. Thumb disassembly independently matches the diagnostic's layouts:

| Structure | ARM32 size | Observed layout |
|---|---:|---|
| Configuration | 32 bytes | Eight consecutive 32-bit words |
| Input | 36 bytes | Three reserved words; plane pointers at 12; lengths at 24 |
| Output | 12 bytes | Data pointer at 0; length at 4; keyframe flag at 8 |

The installed wrapper copies input planes into internal memory before invoking
`KADP_VENC_ENC_Encode`, then allocates and copies encoded output.
`ReleaseFrameData` frees that output pointer and clears it. This is not a zero-copy
interface. Function presence and matching layouts do not qualify enum values,
error cleanup, supported codecs/resolutions, sustainable frame rate, or safe
coexistence with QSync. No before/after firmware comparison establishes an encoder
enhancement, and no HEVC encoding capability was established.

Private library copies, SHA-256 receipts, disassembly, a canonical research C
header and bindgen 0.73.2 generated Rust bindings are under
`~/.local/share/lg-hue-sync/research/plx-native-2026-11-09/c1/`.
Bindings are isolated from the daemon. Original libraries remain untouched; no
encoder function was invoked and no TV or lighting setting was changed.
ARM-target C static assertions and Rust compilation passed for these sizes and
offsets. Re-running bindgen produced identical output. This validates generated
layout consistency, not encoder execution.

### Authorized synthetic encoder test, 2026-11-09

Later the same day, the user authorized hardware encoding tests while preserving
live lighting settings. A temporary standalone ARM Rust benchmark used the
generated structures, synthetic NV12 frames and bounded process deadlines. It did
not acquire capture, restart QSync, or install a persistent streaming service.

The first native create call crashed. A process-local diagnostic handler and
disassembly narrowed the failure to the KADP ioctl helper dereferencing a null
handle. The HAL wrapper stores the result of `KADP_VENC_Open` at context offset
`0x48`, but passes its zero-initialized first word to
`KADP_VENC_ENC_Create`. A temporary preload shim confirmed the null argument and
substituted the saved device handle, without changing firmware files.

With that workaround, creation returns `-1` with `errno=5` (`Input/output error`).
Kernel logs identify `ENC_H264Create: HXENC_H264SetRateCtrl - failed.(-3)`.
This persisted for 640x360 and 1280x720 synthetic input, raw bitrate values 4000
and 4000000, and a bounded alternate configuration of the final two words. The
field names and units recovered from the upstream probe remain unqualified.
The driver also logs `Not supported chip 0x24B0` in its clock-gating helper;
this message alone does not establish that encoding is impossible.

No frame encoded successfully. Consequently there is no valid encoder frame-rate,
latency, output-validation or incremental resource-load result. The next step is
offline recovery of the C1 driver's rate-control configuration contract, rather
than more parameter guesses or enabling persistent streaming. The fixed 2 MiB
HAL input allocation also excludes a naive 1920x1080 NV12 trial.

Private benchmark source, shim, logs and post-test resource/status receipts are
beside the ABI evidence in the `c1/` directory above. These are diagnostic
experiments, not a production encoder implementation.
The final approximately five-second baseline measured 35.37% whole-TV CPU,
89.04% of one CPU core for QSync, and 8.89 MiB QSync RSS (four CPU cores).
These are post-probe baseline figures with no active encoding, not encoder load.
QSync reported `running`, 50 capture fps and no error. Live settings compared
equal to the pre-test snapshot, and the persisted configuration SHA-256 was
unchanged. All temporary benchmark files and the crash core were removed from
the TV after preserving private local receipts.

### Shared capture and optional encoding

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
