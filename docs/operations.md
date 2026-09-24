# Build and TV operations

Use `<tv-ip>` explicitly. Never add a private address or populated configuration to the repository.

## Local gate

```bash
make check
```

## Codex project setup

Use the Codex project's **Set up this project** dialog with these values:

- Setup script:

  ```bash
  cd "$CODEX_WORKTREE_PATH"
  make setup
  ```

- Cleanup script: leave blank. Automatic cleanup must not discard Docker's reusable cross-build image and named Cargo/target caches.
- Variables: `TV_IP=<tv-ip>` and, only when non-default, `SSH_PORT=<port>`. Keep device addresses in project variables, never Git.
- Actions:
  - **Update dependencies** — `make deps-update check`
  - **Build and verify webOS binary** — `make check build`
  - **Build and transfer to TV** — `make build deploy-bin`
  - **Clean Docker build caches** — `make cross-clean`

Docker Desktop must be running for build, transfer, and cache-clean actions. The build action recreates the cross-toolchain image and named caches when absent. Transfer preserves the TV's paired `config.json`, verifies the uploaded binary checksum, and restarts the daemon.

## Target build

```bash
make build
file target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
llvm-readelf -h target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
llvm-readelf --version-info target/armv7-unknown-linux-gnueabi/release/lg-hue-sync
```

Required result: 32-bit ARM Linux ELF with no required symbol newer than `GLIBC_2.28`. On Linux, GNU `readelf` is equivalent.

`make build` uses `docker/Dockerfile.cross`, which bakes the archived Debian Buster packages, pinned Rust toolchain, ARMv7 target, and linker into `lg-hue-sync-cross`. Cargo registry and target artifacts live in named Docker volumes, so subsequent builds are incremental. `make cross-clean` removes the image and both caches when disk space matters.

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
ares-rs-shell --device tv '<command>'
```

Node equivalents (`ares-package`, `ares-install`, `ares-push`, `ares-shell`) remain supported. Ares handles the launcher app and ordinary transfer. Root-owned daemon/service provisioning still uses the repository's SSH workflow until the IPK owns and verifies the complete install/uninstall lifecycle.

## Safe binary update

```bash
make build
make deploy-bin TV_IP=<tv-ip>
```

The update retains `config.json`, backs up the previous binary, uploads through a temporary filename, and restarts the service.

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
