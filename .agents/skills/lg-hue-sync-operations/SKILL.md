---
name: lg-hue-sync-operations
description: Build, transfer, verify, roll back, uninstall, or release lg-hue-sync for rooted LG webOS TVs while preserving credentials and separating local validation from live proof.
---

# lg-hue-sync operations

Read `AGENTS.md`, then the relevant guide:

- Build/transfer/rollback/uninstall/diagnostics: `docs/operations.md`
- Version/tag/release: `docs/releasing.md`
- Hue/Nanoleaf mapping and ownership: `docs/architecture.md`

## Invariants

- Deploy only `armv7-unknown-linux-gnueabi`; never transfer a host binary.
- Default `make build`/`make build-native` uses the macOS webOS SDK compiler with the extracted Buster sysroot after `make native-setup`, not its older libc/polyfills. Verify each artifact as ELF32 little-endian ARM EABI5 soft-float, `/lib/ld-linux.so.3`, and glibc <= 2.28; previous releases' TV checks do not qualify a new artifact. `make build-docker` remains the explicit cached fallback and Linux CI path.
- [`openlgtv/buildroot-nc4`](https://github.com/openlgtv/buildroot-nc4) underpins that SDK; consult [`webosbrew/webos-userland`](https://github.com/webosbrew/webos-userland/tree/main) and its [generated API reference](https://www.webosbrew.org/webos-userland/index.html) for capture headers, then verify symbols and behavior on the target firmware. See `docs/operations.md` for gates.
- Prefer installed `ares-rs-*` aliases for launcher packaging/install and ordinary transfer; retain SSH for root service lifecycle.
- Preserve the TV's paired `config.json` during updates.
- Test source transitions: HDMI → built-in app/Home releases capture and reports `unsupported_source`; returning to HDMI resumes only source-paused sync. Manual Stop must survive input changes. Unknown input observations must not invent compatibility or DRM status.
- Use the shared bounded Luna transport with stdin retained until reply. Closing stdin early makes this firmware's `luna-send` silently exit; validate a real read-only query on the TV rather than relying only on host fixtures.
- Treat credentials, tokens, pins, IPs, and MACs as private.
- Require explicit direction before TV writes, service restarts, Luna changes, autoroot, or light output.
- Verify local/remote SHA-256, service state, and dashboard response after transfer.
- Verify physical output before claiming capture-to-light success.
- Preserve Hue v2 channel IDs and grouped gradient member identity.
- Use CA-verified HTTPS, saved Bridge ID and `/auth/v1` application identity; see `docs/architecture.md` for legacy-pin migration and ownership rules. Do not resume cached Hue output after known deletion, revoked access or a failed layout refresh. Verify recovery on the TV before claiming it works live.
- Work directly on `main`; no branch or PR unless explicitly requested.

Use repository scripts and Make targets. If a script would overwrite configuration or broaden device mutation, fix the script instead of issuing an ad-hoc destructive command.

## Host environment and validation

- Use `make check` for the complete host gate, including release consistency and embedded dashboard JavaScript. `make test`/`cargo nextest run --locked` runs only Rust tests. Build-script and packaging fixtures use `uv run python scripts/test_native_toolchain.py` and `uv run python scripts/test_package_ipk.py`.
- `nix develop` supplies the pinned host compiler, ARMv7 standard library and check/build utilities. Treat `flake.nix`/`flake.lock` as the pin sources; Nix is optional and is never installed on the TV.
- Native scripts accept `WEBOS_RUSTC` and `WEBOS_CARGO` only as a complete executable pair. Nix sets both. Without the pair, retain explicit rustup-stable selection to avoid Homebrew compiler/target mismatches. Refresh older installed helpers with `make native-setup` before using the pair.
- The SDK, extracted Buster sysroot, Rust Ares aliases and Docker daemon remain separate prerequisites. Nix does not change the native linker/sysroot, bypass ABI checks, or turn the macOS native path into a Linux build. Use the existing Docker fallback on Linux.
- `make setup` and `make toolchain-update` manage rustup outside the Nix shell; they do not refresh Nix pins. Update those pins deliberately rather than silently replacing one toolchain with another.
- Report flake evaluation, full shell realization, host checks, ARM/ABI inspection and TV validation separately. Successful evaluation alone does not prove the shell dependencies can be installed or the binary runs on the TV.
