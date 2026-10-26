# Build and TV operations

Use `<tv-ip>` explicitly. Never add a private address or populated configuration to the repository.

## Local gate

Install [cargo-nextest](https://nexte.st/docs/installation/pre-built-binaries/) before running host checks. CI uses nextest 0.9.146.

```bash
make check
```

Run only the Rust tests with `make test` or `cargo nextest run --locked`.

### Optional Nix development environment

`nix develop` supplies Rust 1.99.1 with rustfmt, Clippy and the ARMv7 standard library,
plus nextest, uv/Python, Bun/Node, pkg-config and host build/package utilities. The
checked-in `flake.lock` pins nixpkgs and rust-overlay. Supported hosts are Apple
Silicon macOS and ARM64/x86-64 Linux. Nix runs on the build computer, not the TV.

```bash
nix develop
make check
uv run python scripts/test_native_toolchain.py
make build  # macOS: after the existing SDK/Buster native setup
```

The shell exports `WEBOS_RUSTC` and `WEBOS_CARGO`, so native setup/build use the Nix
compiler and its target library instead of a host rustup/Homebrew installation.
Set both variables together when supplying another explicit toolchain. Without
them, the existing rustup-stable selection is unchanged. Re-run `make native-setup`
once to refresh previously installed helper scripts before using these variables.

Nix does not install or relocate the webOS SDK, create the Buster sysroot, install
Rust Ares aliases, start Docker, or change device profiles/configuration. Keep the
existing native setup and ABI checks; Linux target builds still use the explicit
`make build-docker` fallback and require a running Docker daemon. `make setup` and
`make toolchain-update` remain rustup-management commands outside this shell; the
Nix compiler changes only when its pin is deliberately updated.

## Colour response

Hue and Nanoleaf retain fractional colour state for smoother small fades. In the dashboard,
**Advanced tuning → Spatial Gradient Blend** controls blending between nearby screen regions
(0–50%; default 0). Start at 10–20% if local transitions look too abrupt; higher values soften
spatial boundaries. Black regions stay off. Save settings to persist the value; the JSON/API
field is `spatial_blend`. Existing configurations default to zero. Named presets set their own
blend: Neutral and Fast Response 0%, High Chroma 15%, Neon Contrast 8%, Dark-Scene Detail 10%,
Low Stimulation 30%. These are conservative starting points; adjust by eye for your layout.
This is encoded-RGB interpolation, not perceptual-space blending or HDR decoding.

## Read-only capture colour probe

The dashboard's `/capture-patterns` page shows nominal SDR patches. Load it in the browser on the HDMI source before stopping the daemon; its controls continue to work without the server. Apple TV fixed Dolby Vision output can show how that output mode affects capture, but the browser patches are not authored Dolby Vision reference values. A verified HDR10 or Dolby Vision video is required to assess those encoded source paths.

`test-capture --raw-nv12-stats` requires free vtCapture hardware and prints five central Y/Cb/Cr summaries plus the TV's read-only reported dynamic-range mode. It never saves full frames. Stop the daemon before each probe, then restart it even if the probe fails. Confirm the service is active again; the existing `make test-capture` target does not manage that lifecycle. The mode label is a TV output hint, not capture colour-space metadata. Do not change the fixed BT.601 decoder based on a mode label alone.

## Compatible inputs and automatic pause

The supported capture policy is HDMI-only. A bounded foreground-app observation
checks the standard Luna `getForegroundAppInfo` interface every two seconds.
Known HDMI app IDs 1–4 permit capture; built-in apps (including Netflix) and Home
release capture/output sessions and report `pipeline_state: unsupported_source`.
The dashboard explains that it is waiting for HDMI rather than retrying the
unsupported input indefinitely. Returning to HDMI retries capture automatically
only when the input policy paused it. Manual Stop stays stopped across input
changes; Start on an unsupported input arms the next compatible HDMI transition.
Restart does not attempt capture while the foreground source is unsupported.

An unsuccessful source query preserves the last known policy. App identity is
an input-compatibility policy, not evidence of DRM; `vtCapture_init` code 17 has
not been canonically mapped to a DRM condition. PicCap documents limitations for
[built-in Netflix/Amazon protected playback](https://github.com/webosbrew/piccap#piccap).
This release does not attempt to bypass those restrictions.

Source, power, dynamic-range and frame-rate observations use `src/luna.rs`:
1.5-second deadline, 64 KiB reply limit and killed/reaped failed children. It
retains piped stdin until the reply because this firmware's `luna-send` otherwise
silently exits at EOF. Frame-rate polling is capped at once every two seconds.
No foreground-app identifiers, titles or reply bodies are exposed in error logs.

For TV validation, observe HDMI → app/Home → HDMI, verify capture release and
automatic recovery, then repeat after manual Stop to confirm it stays stopped.
Check actual capture readiness, not just process health or connection flags.

The native v0.7.0 candidate passed the first live transition on 2026-10-22:
`running`/hardware true/60 FPS → `unsupported_source`/hardware false/both rates
zero → `running`/hardware true/60 FPS, with its error cleared automatically.
Local/remote binary SHA-256 matched and the paired configuration checksum stayed
unchanged. Source-policy tests cover manual Stop surviving source changes.

## Codex project setup

Codex loads [`.codex/environments/environment.toml`](../.codex/environments/environment.toml) from this repository. It sets up new worktrees with `make setup` and adds actions to update dependencies, refresh Rust stable, validate/build, transfer to the TV, and clean Docker builds. Leave automatic worktree cleanup empty so reusable Docker caches survive.

The transfer action reads `TV_IP=<tv-ip>` from `~/.config/lg-hue-sync/device.env` on the local host, or from its terminal environment. Add `SSH_PORT=<port>` there only if it differs from 22. This file stays outside Git and is not copied into a worktree.

Ordinary builds and transfers no longer require Docker after native setup.
**Update Rust toolchain** refreshes rustup stable, runs host checks, and builds
natively. Docker must be running for explicit fallback/image/cache operations;
`make docker-toolchain-update` refreshes its image separately. Transfer preserves
the TV's paired `config.json`, verifies the uploaded binary checksum, and restarts
the daemon.

## Target build

```bash
make build
file target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
llvm-readelf -h target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
llvm-readelf --version-info target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
```

Required result: 32-bit ARM Linux ELF with no required symbol newer than `GLIBC_2.28`. On Linux, GNU `readelf` is equivalent.

`make build` aliases `make build-native`: the macOS webOS SDK compiler links
against the extracted Buster sysroot. See setup below. Missing native tools fail
with setup/fallback instructions; no automatic toolchain switch occurs.

`make build-docker` explicitly uses `docker/Dockerfile.cross`, which bakes the
archived Buster packages, stable Rust, ARM target and linker into the cached
builder. The image stays at its installed Rust version until
`make docker-toolchain-update` refreshes it. Named Cargo/target volumes preserve
incremental fallback builds. `make cross-clean` removes the image and those
volumes; it does not remove the native SDK/sysroot/cache.

Codex and local developers use the native default. Linux GitHub Actions explicitly
runs `make build-docker`; its ordinary job separately checks formatting, host
tests, Clippy, and embedded dashboard JavaScript. Both build paths target the
same soft-float ABI and glibc ceiling.

The initial hard-float GNU/musl `cross-rs` configuration and host Cargo linker
overrides have been removed; no repository workflow used them. The supported
TV target is `armv7-unknown-linux-gnueabi`, with its linker configured inside
the native helper or cached Docker image. Host Cargo builds keep their native target. Alternate
ABIs require separate ELF, libc and TV runtime validation before adoption.

## Dependencies

```bash
make deps-check   # dry-run compatible Cargo.lock updates and show duplicate versions
make deps-update  # update Cargo.lock within Cargo.toml version constraints
```

Dependency updates are deliberate direct-to-`main` changes: inspect `Cargo.lock`, run the local gate and `make build`, then commit. Automated dependency pull requests are intentionally not enabled for this single-maintainer workflow.

### webOS Brew native toolchain

[`webosbrew/native-toolchain`](https://github.com/webosbrew/native-toolchain) is the community reference SDK. On macOS, download the matching Darwin archive, extract it to a path without spaces, and run its `relocate-sdk.sh`. Its CMake toolchain file is under `share/buildroot/toolchainfile.cmake`.

The SDK's original libc is older than our Buster baseline. Earlier linking failed at `getauxval`; the SDK also includes compatibility polyfills, which this build deliberately avoids. Use its native macOS compiler and ARM archive tools with the actual Buster headers/libraries instead:

```bash
make native-setup  # once: requires installed SDK and already cached Docker builder
make build-native  # subsequent builds use Mac tools; Docker need not run
```

`native-setup` installs reusable helpers and a 25 MiB sysroot under
`~/.local/share/webosbrew-rust`, recording the cached builder's image ID. It never
pulls/rebuilds an image or downloads another SDK. Existing sysroots are reused.
The SDK defaults to
`~/.local/share/webosbrew-native-toolchain/arm-webos-linux-gnueabi_sdk-buildroot`.
For another installation, export `WEBOS_SDK` for both setup and builds. Override
`WEBOS_NATIVE_HOME` consistently to relocate the installed helpers/sysroot.

The native helper uses the supplied `WEBOS_CARGO`/`WEBOS_RUSTC` pair when set,
otherwise explicitly selects rustup's stable Cargo/rustc and ARM target;
Homebrew Rust can otherwise shadow them. It sets target-specific GCC, `ar` and
`ranlib`, disables SDK libc polyfills, and builds into `target/native`. Before
atomically replacing the normal ARM release artifact, it verifies ELF32,
little-endian ARM EABI5, soft-float ABI, `/lib/ld-linux.so.3`, and glibc <= 2.28.
Failures preserve the previous artifact. No custom Rust compiler, standard
library or fake `getauxval` shim is needed.

`make build` now selects the native build; `make build-docker` is the explicit
fallback. Both produce the same artifact path, so
`make deploy-bin TV_IP=<tv-ip>` can transfer either after an explicitly requested
deployment. `./scripts/deploy.sh` follows the native default unless `--reuse` is
explicitly supplied. To deploy a freshly verified Docker artifact, run
`make build-docker` then `make deploy-bin TV_IP=<tv-ip>`.

The native v0.6.1 artifact was tested on the TV on 2026-10-22. Local/remote SHA-256
matched; the prior binary was retained and the paired configuration checksum
stayed unchanged. The dashboard responded, systemd reported active/running with
zero restarts, hardware capture ran at 320x180/60 FPS, and both Hue and Nanoleaf
reported connected. This verifies this artifact's service/capture integration,
not physical lighting quality or every future build.

[`hyperhdr-webos-loader`](https://github.com/webosbrew/hyperhdr-webos-loader)
remains the reference for native service, frontend, autostart, and IPK layout.

[`openlgtv/buildroot-nc4`](https://github.com/openlgtv/buildroot-nc4) is the Buildroot source used by the community toolchain. Rebuilding or forking it would be a separate SDK maintenance project. Keep the cached container as a recoverable fallback until supervised TV checks pass.

For native LG capture interfaces, consult [`webosbrew/webos-userland`](https://github.com/webosbrew/webos-userland/tree/main) and its [generated API reference](https://www.webosbrew.org/webos-userland/index.html). These are community headers and stub libraries, not proof that a symbol or capture mode works on this TV firmware. Keep runtime symbol checks and frame validation at the capture boundary.

### Experimental Zig cross-build

[`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild#specify-glibc-version)
can link the daemon without Docker or the webOS SDK. The 2026-10-22 probe used
`cargo-zigbuild 0.24.0`, `ziglang 0.17.0` and rustup stable `rustc 1.99.1`.
The locked v0.6.1 release build completed in about 95 seconds with four jobs.
From the repository root, this isolated invocation leaves the normal release
artifact untouched and installs no global tools:

```bash
(
set -eu
ZIG_PROBE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/lg-zig-probe.XXXXXX")"
uv venv "$ZIG_PROBE_DIR/venv"
uv pip install --python "$ZIG_PROBE_DIR/venv/bin/python" cargo-zigbuild==0.24.0 ziglang==0.17.0
ZIG_PROBE_RUSTC="$(rustup which --toolchain stable rustc)"
ZIG_PROBE_CARGO="$(rustup which --toolchain stable cargo)"
env -u RUSTFLAGS -u CARGO_ENCODED_RUSTFLAGS -u CARGO_BUILD_TARGET \
  -u RUSTC_WRAPPER -u RUSTC_WORKSPACE_WRAPPER \
  -u CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABI_LINKER \
  -u CC_armv7_unknown_linux_gnueabi -u AR_armv7_unknown_linux_gnueabi \
  -u RANLIB_armv7_unknown_linux_gnueabi \
  -u CPATH -u C_INCLUDE_PATH -u CPLUS_INCLUDE_PATH -u LIBRARY_PATH \
  PATH="$ZIG_PROBE_DIR/venv/bin:$PATH" RUSTUP_TOOLCHAIN=stable \
  RUSTC="$ZIG_PROBE_RUSTC" CARGO="$ZIG_PROBE_CARGO" \
  CARGO_TARGET_DIR="$ZIG_PROBE_DIR/target" CARGO_BUILD_JOBS=4 \
  CARGO_ZIGBUILD_PYTHON_PATH="$ZIG_PROBE_DIR/venv/bin/python" \
  CARGO_ZIGBUILD_ZIG_PATH="$ZIG_PROBE_DIR/venv/bin/python-zig" \
  CARGO_ZIGBUILD_CACHE_DIR="$ZIG_PROBE_DIR/cargo-zig-cache" \
  ZIG_GLOBAL_CACHE_DIR="$ZIG_PROBE_DIR/zig-cache" \
  ZIG_LOCAL_CACHE_DIR="$ZIG_PROBE_DIR/zig-local-cache" \
  rustup run stable cargo zigbuild --locked --release --jobs 4 \
    --target armv7-unknown-linux-gnueabi.2.28 \
    --manifest-path "$PWD/Cargo.toml" --bin lg-hue-sync
printf 'Candidate: %s\n' "$ZIG_PROBE_DIR/target/armv7-unknown-linux-gnueabi/release/lg-hue-sync"
)
```

The candidate passed SDK `readelf` checks: ELF32 little-endian ARM EABI5,
soft-float ABI, `/lib/ld-linux.so.3`, and maximum imported glibc version 2.28.
The lockfile and normal release artifact hashes stayed unchanged. This proves
local compilation and the inspected ABI, not TV execution. Zig's glibc suffix
has upstream caveats; every candidate still needs the same ELF/loader/symbol
checks and supervised TV validation. No candidate is copied, deployed or
published by this command. The native build remains the default and the cached
Docker build remains the validated fallback.

## First install

Root SSH must already work; rooting is a separate owner action.

```bash
./scripts/deploy.sh <tv-ip>
```

The script installs the binary, Luna permissions, service unit, boot hook, and launcher package. It does not upload `config.json` unless `--with-config` is supplied explicitly.

### Ares transfer tools

The official Rust rewrite is [`webosbrew/ares-cli-rs`](https://github.com/webosbrew/ares-cli-rs). This workstation keeps Node commands unchanged and exposes the verified Rust v0.7.0 binaries as `ares-rs-*` aliases. Upstream shares the OSE registry, but LG's Node TV CLI uses a separate `~/.webos/tv` registry here, so `lgc1` is registered in both.

```bash
ares-rs-setup-device --add tv --info host=<tv-ip> --info username=root --info port=22 --info keyPath=<ssh-private-key>
ares-rs-package webos-app --outdir target
ares-rs-install --device tv target/org.webosbrew.lg-hue-sync_<version>_all.ipk
ares-rs-push --device tv <local-file> <remote-path>
ares-rs-shell --device tv --run '<command>'
```

The `ares-rs-*` names are local aliases for upstream's Rust binaries; Node commands
remain unchanged. `scripts/package_ipk.py` now delegates archive generation to
`ares-rs-package`, preserving the local Homebrew manifest/hash. Packaging failure
preserves prior outputs. Deployment consumes the exact current appinfo version,
not the last filename found in `target`.

```bash
make package-app                           # local only
make list-apps ARES_DEVICE=<registered-name>
make install-app ARES_DEVICE=<registered-name>
make launch-app ARES_DEVICE=<registered-name>
uv run python scripts/test_package_ipk.py
uv run python scripts/test_native_toolchain.py
```

Device-facing targets require an explicit device name. `install-app` installs
only the launcher: it does not provision, update or uninstall the daemon.
Existing IP-based deployment retains SSH/Luna for root service ownership,
configuration preservation and rollback. Full deployment packages locally before
any TV mutation. Do not publish the launcher-only IPK or its generated manifest
as a complete daemon installer.

## Safe binary update

```bash
make build
make deploy-bin TV_IP=<tv-ip>
```

The update retains `config.json`, backs up the previous binary, uploads through a temporary filename, and restarts the service.

## Apple Shortcuts and presets

On the same trusted LAN, create an Apple Shortcut with a **URL** action set to `http://<tv-ip>:8088/api/presets/neutral`, followed by **Get Contents of URL** with method **POST** and no request body. Replace `neutral` with `highChroma`, `neonContrast`, `darkSceneDetail`, `fastResponse`, or `lowStimulation` for the other dashboard presets. A successful request returns `{"status":"ok"}`; an unknown name returns HTTP 400. The existing `POST /api/start` and `POST /api/stop` endpoints can be used in separate Shortcuts.

Preset changes affect live settings only. Use the dashboard's **Save Settings** control, or `POST /api/save-config`, to keep the chosen settings after restart. Presets preserve Hue/Nanoleaf output trims, device enablement, alignment, and other non-preset controls. Port 8088 has no authentication; keep it on the trusted LAN and do not forward it to the Internet. No Shortcut or API call is needed to edit the TV's `config.json` directly.

## Verification

```bash
shasum -a 256 target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
ssh root@<tv-ip> 'sha256sum /var/home/root/lg-hue-sync/lg-hue-sync'
ssh root@<tv-ip> 'systemctl is-active lg-hue-sync'
curl --fail --silent http://<tv-ip>:8088/api/status
```

Match digests, require `active`, and inspect dashboard JSON. Physical capture-to-light behavior needs a visible check; a healthy process alone is insufficient.

For control integration, use `POST /api/start` and `POST /api/stop`, then read
`GET /api/status`. `requested_running` acknowledges the latest accepted HTTP intent;
`pipeline_state` reports actual progress, and `last_error` retains capture failures.
`is_syncing` is true only after a valid hardware frame, not when a command is queued.
Check Hue/Nanoleaf connection flags and light-update rate separately. Stop releases the
capture owner and zeros paused rates; Start reacquires it. A paused daemon still serves
the API. Process start/stop/restart remains systemd's responsibility.

## Rollback

```bash
ssh root@<tv-ip> 'systemctl stop lg-hue-sync && cp /var/home/root/lg-hue-sync/lg-hue-sync.previous /var/home/root/lg-hue-sync/lg-hue-sync && chmod 755 /var/home/root/lg-hue-sync/lg-hue-sync && systemctl start lg-hue-sync'
```

## Uninstall

```bash
./scripts/uninstall.sh <tv-ip>
```

Default behavior preserves `config.json` in a timestamped backup directory. Use `--purge-config` only when the owner explicitly wants credentials removed.

## Diagnostics

Capture skips the first three consecutive unavailable frames. A fourth unavailable frame or an explicit backend transition releases the driver and retries after 750 ms, 1.5 s, then 3 s. Thirty seconds of valid hardware frames resets this recovery budget. Invalid frame layouts or an exhausted budget pause sync; the dashboard remains available, and **Start** or **Restart** permits a manual retry. Native failures retain their backend, operation, and return code in the daemon log. Hardware initialization retries use exponential backoff capped at 16 seconds.

Full deployment installs `lg-hue-sync-log.timer`, which checks `daemon.log` every minute and truncates it in place at 1 MiB (1,048,576 bytes). The timer permits temporary size overshoot between checks, discards the previous log contents, and retains no archives. It preserves the inode used by the daemon's append-only stdout/stderr, so sync does not restart. The webOS Brew boot hook restores the timer after reboot; uninstall removes it. No Rust dependency or firmware-wide logging setting changes.

For an existing installation, copy `scripts/log-maintenance.sh` to `/var/home/root/lg-hue-sync/log-maintenance.sh`, make it executable, and run it on the TV with `--install`. Verify `systemctl is-active lg-hue-sync-log.timer` and `systemctl list-timers lg-hue-sync-log.timer`. Disable it without stopping sync using `systemctl stop lg-hue-sync-log.timer`; remove `/var/lib/webosbrew/init.d/55-lg-hue-sync-log` to keep it disabled after reboot. Run `sh scripts/test-log-maintenance.sh` for boundary, open-writer, and unsafe-path checks.

- Hue layout changed: dashboard **Refresh selected area layout**.
- Gradient count unexpected: inspect `entertainment_configuration.channels[].members`; physical segment count differs from stream-channel count.
- Nanoleaf order wrong: run **4D Tracer**, then adjust corner, direction, and offset under **Calibration**.
- Standby leaves lights owned: enable **Follow TV power** and inspect transition logs.

## Hue authentication and migration recovery

New pairing uses verified HTTPS and persists the Bridge ID. Existing paired configurations
keep their credentials and certificate pin. On the first successful identity refresh, an exact
legacy pin match establishes the saved Bridge ID; later CA-signed certificate renewals are
validated against that ID. A changed certificate before identity migration requires explicit
re-pairing. Do not remove the pin to bypass a validation failure.

If several bridges are discovered, supply the intended IP explicitly. If pairing succeeds but
area discovery fails or several areas exist, credentials are still saved; choose an area in the
dashboard or run `lg-hue-sync sync-hue --config <path> --area <name-or-id>`. Hue output stays
unavailable until a valid area is selected. Selecting an area never changes its physical layout.

- Authorization rejected: re-pair using the Bridge button.
- Area missing after migration: select an existing area explicitly; no arbitrary replacement is chosen.
- Known layout change with failed refresh: Hue stays disconnected. Retry **Refresh selected area layout** after resolving connectivity.
- Another application owns the area: Hue suspends independently of Nanoleaf. Use **Start** to retry after that application releases it; no automatic takeover.
- Bridge IP changed: a failed TCP connection permits identity-scoped rediscovery. TLS identity failures do not permit fallback to another bridge.

Validate on the TV before release: initial pairing, existing-pin migration, automatic address
recovery, another app's ownership, area edits/deletion, explicit Stop, HDMI/app transitions and
independent Nanoleaf output. Host fixtures and ARM compilation do not establish these results.
Measure CPU and packet-loss behavior before claiming a performance improvement from cached
50 Hz repetitions. A 2 Hz repeat remained below Hue's timeout, but was below its recommended
continuous transport cadence.

### v0.8.0 device validation (2026-10-22)

The native ARMv7 artifact's SHA-256 matched the installed TV binary. Verified HTTPS
pairing and legacy-pin-to-Bridge-ID migration succeeded. The initial migration preserved
credentials and configuration except for the new Bridge ID; explicit re-pairing then saved
new Hue credentials while retaining Nanoleaf configuration and the selected layout.

Home caused `unsupported_source`, zero capture/update rates, released hardware and
both output flags false. Returning to the original HDMI input restored hardware capture
at 60 FPS and both outputs. Manual Stop remained paused across Home/HDMI changes;
explicit Start restored both outputs. Disabling either output left the other and capture
running; both output settings were restored. The service reported zero automatic restarts.

The maintainer confirmed both Hue and Nanoleaf visibly syncing after restoration;
this was not a calibrated colour or latency measurement.

The first DTLS attempt timed out once, then retry connected; later resume handshakes
succeeded. Competing-owner runtime testing was omitted at the maintainer's direction;
this is not a claim of validated multi-client behavior. Physical colour accuracy, CPU
impact and packet-loss resilience were not measured. Docker CI publishes a separately
built artifact; native TV results do not establish runtime validation of that binary.

`make pair TV_IP=<tv-ip>` now invokes the TV's Rust pairing command over SSH and
restarts the daemon only after successful pairing. The deprecated Python wrapper is
for host-side pairing and intentionally rejects its removed `--tv-ip` option.
