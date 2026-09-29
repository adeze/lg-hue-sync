# Build and TV operations

Use `<tv-ip>` explicitly. Never add a private address or populated configuration to the repository.

## Local gate

```bash
make check
```

## Read-only capture colour probe

The dashboard's `/capture-patterns` page shows nominal SDR patches. Load it in the browser on the HDMI source before stopping the daemon; its controls continue to work without the server. Apple TV fixed Dolby Vision output can show how that output mode affects capture, but the browser patches are not authored Dolby Vision reference values. A verified HDR10 or Dolby Vision video is required to assess those encoded source paths.

`test-capture --raw-nv12-stats` requires free vtCapture hardware and prints five central Y/Cb/Cr summaries plus the TV's read-only reported dynamic-range mode. It never saves full frames. Stop the daemon before each probe, then restart it even if the probe fails. Confirm the service is active again; the existing `make test-capture` target does not manage that lifecycle. The mode label is a TV output hint, not capture colour-space metadata. Do not change the fixed BT.601 decoder based on a mode label alone.

## Codex project setup

Codex loads [`.codex/environments/environment.toml`](../.codex/environments/environment.toml) from this repository. It sets up new worktrees with `make setup` and adds actions to update dependencies, refresh Rust stable, validate/build, transfer to the TV, and clean Docker builds. Leave automatic worktree cleanup empty so reusable Docker caches survive.

The transfer action reads `TV_IP=<tv-ip>` from `~/.config/lg-hue-sync/device.env` on the local host, or from its terminal environment. Add `SSH_PORT=<port>` there only if it differs from 22. This file stays outside Git and is not copied into a worktree.

Docker Desktop must be running for toolchain update, build, transfer, and cache-clean actions. Ordinary builds reuse the most recently built stable Rust image; **Update Rust toolchain** refreshes stable Rust on the host, rebuilds the image without Docker's layer cache, then runs host checks and the ARM build. Transfer preserves the TV's paired `config.json`, verifies the uploaded binary checksum, and restarts the daemon.

## Target build

```bash
make build
file target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
llvm-readelf -h target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
llvm-readelf --version-info target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
```

Required result: 32-bit ARM Linux ELF with no required symbol newer than `GLIBC_2.28`. On Linux, GNU `readelf` is equivalent.

`make build` uses `docker/Dockerfile.cross`, which bakes the archived Debian Buster packages, the stable Rust toolchain, ARMv7 target, and linker into `lg-hue-sync-cross`. The cached image remains at its installed Rust release until `make toolchain-update` refreshes it. Cargo registry and target artifacts live in named Docker volumes, so subsequent builds are incremental. `make cross-clean` removes this project's cross-toolchain images and both caches when disk space matters.

Codex and developers use this same target rather than maintaining separate toolchains. GitHub Actions also runs it on every push to `main`; the ordinary CI job separately checks formatting, host tests, Clippy, and embedded dashboard JavaScript.

## Dependencies

```bash
make deps-check   # dry-run compatible Cargo.lock updates and show duplicate versions
make deps-update  # update Cargo.lock within Cargo.toml version constraints
```

Dependency updates are deliberate direct-to-`main` changes: inspect `Cargo.lock`, run the local gate and `make build`, then commit. Automated dependency pull requests are intentionally not enabled for this single-maintainer workflow.

### webOS Brew native toolchain

[`webosbrew/native-toolchain`](https://github.com/webosbrew/native-toolchain) is the community reference SDK. On macOS, download the matching Darwin archive, extract it to a path without spaces, and run its `relocate-sdk.sh`. Its CMake toolchain file is under `share/buildroot/toolchainfile.cmake`.

It was evaluated for this Rust repository, not ignored. Relocation repairs its original CI sysroot path, but the current Rust dependency graph still fails to link because the SDK libc lacks `getauxval`, required by Rust's supported `armv7-unknown-linux-gnueabi` standard library and `ring`.

Do not add a fake `getauxval` shim. Reconsider the native SDK when either:

- the SDK libc exports `getauxval`; or
- a custom Rust standard library built against its sysroot passes tests, ELF inspection, and a supervised TV probe.

Until then, the Debian Buster container is canonical. [`hyperhdr-webos-loader`](https://github.com/webosbrew/hyperhdr-webos-loader) remains the reference for native service, frontend, autostart, and IPK layout; it uses the same Buildroot SDK, but does not solve this Rust libc boundary.

[`openlgtv/buildroot-nc4`](https://github.com/openlgtv/buildroot-nc4) is the Buildroot source used by the community toolchain. Rebuilding or forking it would be a separate SDK maintenance project; it does not by itself resolve the current Rust `getauxval` link failure. Keep the cached container build unless a candidate SDK passes the link, ELF/glibc inspection, and supervised TV checks above.

For native LG capture interfaces, consult [`webosbrew/webos-userland`](https://github.com/webosbrew/webos-userland/tree/main) and its [generated API reference](https://www.webosbrew.org/webos-userland/index.html). These are community headers and stub libraries, not proof that a symbol or capture mode works on this TV firmware. Keep runtime symbol checks and frame validation at the capture boundary.

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

Node equivalents (`ares-package`, `ares-install`, `ares-push`, `ares-shell`) remain supported. The `ares-rs-*` names are local aliases for upstream's `ares-*` Rust binaries. Ares can package and install the launcher app and transfer files, but the repository's deploy scripts currently use `scripts/package_ipk.py` and SSH/Luna. Root-owned daemon/service provisioning stays on the SSH workflow until the IPK owns and verifies the complete install/uninstall lifecycle.

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

- Hue layout changed: dashboard **Refresh selected area layout**.
- Gradient count unexpected: inspect `entertainment_configuration.channels[].members`; physical segment count differs from stream-channel count.
- Nanoleaf order wrong: run **4D Tracer**, then adjust corner, direction, and offset under **Calibration**.
- Standby leaves lights owned: enable **Follow TV power** and inspect transition logs.
