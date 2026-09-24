# lg-hue-sync

Rust daemon and LAN dashboard for screen-derived Philips Hue Entertainment and Nanoleaf 4D output on rooted LG webOS TVs.

## Commands

- Host gate: `cargo fmt --all -- --check && cargo test && cargo clippy --bin lg-hue-sync -- -D warnings`
- Target build: `make build` (`armv7-unknown-linux-gnueabi`, cached `docker/Dockerfile.cross`, Debian Buster/glibc 2.28 baseline)
- Dependencies: `make deps-check`; update compatible versions with `make deps-update`, then run host and target gates
- Reference SDK: `webosbrew/native-toolchain`; see `docs/operations.md` before changing the canonical build
- Ares tools: `ares-rs-*` (official Rust v0.7.0 aliases) or existing Node `ares-*`
- Safe update: `make deploy-bin TV_IP=<tv-ip>`; preserve the paired TV `config.json`
- First install: `./scripts/deploy.sh <tv-ip>`; root SSH must already work
- Verify: `make status TV_IP=<tv-ip>` and `curl http://<tv-ip>:8088/api/status`

## Rules

- Maintainer and agent work goes directly to `main`; do not create branches or pull requests unless explicitly requested. External contributors use pull requests.
- Never commit device IPs, MAC addresses, Hue credentials, Nanoleaf tokens, certificate pins, or populated `config.json` files.
- A host macOS binary is not deployable. Verify a 32-bit ARM Linux ELF compatible with webOS glibc 2.28.
- Binary updates preserve `/var/home/root/lg-hue-sync/config.json` and keep a recoverable prior binary.
- Upload, install, restart, autoroot, Luna mutation, or live light output requires explicit user direction.
- Verify deployment with local/remote SHA-256, service state, dashboard status, and—when claimed—physical output.
- Hue v2 channel IDs and grouped gradient members are authoritative; never invent absent channels.
- Hue and Nanoleaf remain independently controllable. Hue physical placement belongs to the Hue app.

## Pull-request review

- Reject changes that expose credentials, device addresses, certificate pins, or populated configuration.
- Require host checks for all code changes and the ARMv7 target build for runtime, dependency, or packaging changes.
- Do not publish the launcher-only IPK or Homebrew manifest until the package owns safe install, upgrade, and uninstall of the daemon and service lifecycle.

## Detailed guidance

- [Architecture and device ownership](docs/architecture.md)
- [Build, transfer, rollback, and uninstall](docs/operations.md)
- [Release and semantic-version workflow](docs/releasing.md)
- [Repository operations skill](.agents/skills/lg-hue-sync-operations/SKILL.md)
