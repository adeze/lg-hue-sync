# lg-hue-sync

Rust daemon and LAN dashboard for screen-derived Philips Hue Entertainment and Nanoleaf 4D output on rooted LG webOS TVs.

## Commands

- Host gate: `make check` (release versions, rustfmt, nextest, Clippy, embedded dashboard JavaScript and diff checks). Rust tests alone: `make test` or `cargo nextest run --locked`.
- Optional pinned host environment: `nix develop`; compiler/tool pins live in `flake.nix` and `flake.lock`. The shell exports `WEBOS_RUSTC`/`WEBOS_CARGO` together; native setup/build honor that pair, otherwise use rustup stable. Re-run `make native-setup` to refresh older installed helpers before using explicit toolchains.
- Nix supplies host tools, not the webOS SDK/Buster sysroot, Rust Ares aliases or a running Docker daemon. `make setup`/`make toolchain-update` manage rustup outside Nix; updating rustup does not update the Nix pin. Linux target builds still use `make build-docker`; see `docs/operations.md`.
- Target build: `make build` (`armv7-unknown-linux-gnueabi`, macOS webOS SDK compiler/Buster sysroot, glibc 2.28); prepare once with `make native-setup`. Explicit Docker fallback/Linux CI: `make build-docker` (`docker/Dockerfile.cross`).
- Dependencies: `make deps-check`; update compatible versions with `make deps-update`, then run host and target gates
- Reference SDK: `webosbrew/native-toolchain`; see `docs/operations.md` before changing the canonical build
- Native API reference: `webosbrew/webos-userland` headers and generated docs; see `docs/architecture.md` and verify runtime symbols on the TV
- Ares tools: `ares-rs-*` (official Rust v0.7.0 aliases) or existing Node `ares-*`
- Launcher packaging: `make package-app`; app management requires `ARES_DEVICE=<registered-name>`. Daemon/service lifecycle stays on SSH. Packaging/build-script tests: `uv run python scripts/test_package_ipk.py` and `uv run python scripts/test_native_toolchain.py`.
- Zig is experimental: use the pinned isolated command in `docs/operations.md`; ABI inspection alone does not replace TV validation or the Docker fallback.
- Safe update: `make deploy-bin TV_IP=<tv-ip>`; preserve the paired TV `config.json`
- First install: `./scripts/deploy.sh <tv-ip>`; root SSH must already work
- Verify: `make status TV_IP=<tv-ip>` and `curl http://<tv-ip>:8088/api/status`

## Rules

- Maintainer and agent work goes directly to `main`; do not create branches or pull requests unless explicitly requested. External contributors use pull requests.
- Never commit device IPs, MAC addresses, Hue credentials, Nanoleaf tokens, certificate pins, or populated `config.json` files.
- A host macOS binary is not deployable. Verify a 32-bit ARM Linux ELF compatible with webOS glibc 2.28.
- Nix evaluation, host/container checks and ARM builds are separate evidence from a realized Nix shell or TV execution. Retain per-artifact ELF32/little-endian ARM EABI5 soft-float, `/lib/ld-linux.so.3`, and glibc <= 2.28 checks regardless of compiler source.
- Binary updates preserve `/var/home/root/lg-hue-sync/config.json` and keep a recoverable prior binary.
- Upload, install, restart, autoroot, Luna mutation, or live light output requires explicit user direction.
- Verify deployment with local/remote SHA-256, service state, dashboard status, and—when claimed—physical output.
- Hue v2 channel IDs and grouped gradient members are authoritative; never invent absent channels.
- Hue HTTPS must verify both official Hue root CAs and the saved Bridge ID. Legacy pins bootstrap identity only on an exact match; never fall back to HTTP or another bridge after identity/authentication failure.
- DTLS uses the application ID from `/auth/v1`, not the application key. Never take over or stop another app's stream. Known invalid sessions/layouts must stop cached Hue output without stopping Nanoleaf.
- Hue and Nanoleaf remain independently controllable. Hue physical placement belongs to the Hue app.
- Known non-HDMI foreground apps/Home must release capture and report `unsupported_source`; resume automatically on HDMI only when source-paused, never after manual Stop. Unknown observations preserve the last known policy; do not classify numeric native errors as DRM.
- Use the bounded shared `src/luna.rs` query transport; hold stdin open until the reply because this TV's `luna-send` exits on EOF. Bound output/deadlines, kill and reap failed children; never use unbounded `.output()` for Luna calls.

## Pull-request review

- Reject changes that expose credentials, device addresses, certificate pins, or populated configuration.
- Require host checks for all code changes and the ARMv7 target build for runtime, dependency, or packaging changes.
- Do not publish the launcher-only IPK or Homebrew manifest until the package owns safe install, upgrade, and uninstall of the daemon and service lifecycle.

## Detailed guidance

- [Architecture and device ownership](docs/architecture.md)
- [Build, transfer, rollback, and uninstall](docs/operations.md)
- [Release and semantic-version workflow](docs/releasing.md)
- [Repository operations skill](.agents/skills/lg-hue-sync-operations/SKILL.md)
