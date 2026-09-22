# LG Hue Sync — Architecture & Agent Guide

Native screen capture and Philips Hue Entertainment synchronization for LG webOS Smart TVs, specifically configured for:
- **Target Device**: LG C1 OLED 2021 (MAC: `24:E8:53:D7:55:4C`, IP: `192.168.1.149`)
- **Firmware**: webOS 6.x (Software Version `03.53.45`)
- **SoC**: MediaTek/Realtek customized Alpha 9 Gen 4 (64-bit ARM Cortex-A73/A53 with 32-bit `armv7l` GNU userspace)

---

## Tooling & Environment Discipline

- **Python Scripting**: Always execute Python scripts using `uv run <script.py>`. Never invoke `python3` or `python` directly.
- **Rust Tooling**: Run host checks with `cargo test` and `cargo clippy -- -D warnings`. The TV target is `armv7-unknown-linux-gnueabi`, not `gnueabihf`; a macOS build is never deployable to webOS.
- **File Editing**: Use scoped patch-based edits; preserve unrelated working-tree changes.

---

## Project Structure

```
lg-hue-sync/
├── AGENTS.md                  # Project context, hardware targets, protocols
├── Cargo.toml                 # Rust daemon dependencies and profiles
├── Cross.toml                 # Containerized cross-compilation config
├── config.example.json        # Template configuration for zones & bridge
├── scripts/
│   ├── prepare_dejavuln_usb.sh # DejaVuln USB autoroot payload stager
│   ├── deploy.sh              # Cross-compiler & SSH deployer for headless systemd daemon
│   └── pair_hue.py            # Optional Hue Bridge pairing helper via uv
└── src/
    ├── main.rs                # Daemon CLI entrypoint (run, pair, test-pattern, test-capture)
    ├── config.rs              # Configuration loader & zone definitions
    ├── capture/
    │   ├── mod.rs             # Screen capture factory
    │   └── vtcapture.rs       # libvtcapture / dile_vt FFI loader & mock pattern generator
    ├── color/
    │   ├── mod.rs             # Color processing exports
    │   └── zones.rs           # Spatial zone sampler & EMA smoothing filter
    └── hue/
        ├── mod.rs             # Entertainment stream activation via REST
        ├── dtls.rs            # DTLS 1.2 PSK client on UDP port 2100
        └── stream.rs          # Binary HueStream protocol packet builder
```

---

## Hardware & Root Invariants

- **Root Access Mandatory**: `libvtcapture` and access to `/dev/video*` hardware decoders are blocked by webOS user sandboxes. The daemon and grabbers must run as `root`.
- **Automatic Updates Must Be Disabled**: LG pushes firmware patches that mitigate root exploits. The TV must have "Allow Automatic Updates" turned OFF.
- **DRM Content Boundary**:
  - **HDMI Inputs**: PS5, Xbox, Apple TV 4K, Shield TV, Switch, and PC inputs are fully captured at low latency.
  - **Internal DRM Apps**: Native webOS streaming apps (Netflix, Disney+, Prime Video) route video through hardware TrustZone secure video planes. `libvtcapture` will yield black frames for these streams. Use an external HDMI streaming device for ambient sync with DRM content.

---

## Network Ports & Protocols

| Service | Host / Port | Protocol | Purpose |
| :--- | :--- | :--- | :--- |
| **TV SSH** | `192.168.1.149:22` | SSH | Remote management and root execution |
| **Hyperion Flatbuffers** | `192.168.1.149:19400` | TCP/UDP | PicCap raw frame transport to Hyperion |
| **Hyperion JSON-RPC** | `192.168.1.149:8090` | HTTP/JSON | Hyperion web interface & remote config API |
| **Hue REST API** | `<bridge-ip>:80` | HTTP | Pairing, pushlink authentication, area config |
| **Hue Entertainment** | `<bridge-ip>:2100` | UDP / DTLS 1.2 | Real-time packed RGB streaming (25–60 Hz) |

---

## Deterministic Automation Workflow

```bash
# Step 1: Root the TV over the LAN using SlopBro (No USB drive required)
uv run scripts/root_tv.py --webos-version 6 192.168.1.149

# Step 2: Cross-compile & deploy lg-hue-sync daemon to the TV over SSH:
./scripts/deploy.sh 192.168.1.149

# Step 3: Discover Hue Bridge and pair pushlink button (if needed):
uv run scripts/pair_hue.py --tv-ip 192.168.1.149
```

---

## Native Build and Deployment Policy

- **Current target baseline**: 32-bit ARMv7 GNU EABI userspace with a glibc 2.28 ceiling. Build a Linux ELF using a matching ARM linker and sysroot; host macOS binaries and newer Linux glibc outputs are invalid deployment artifacts.
- **Why Debian Buster Docker exists**: it supplies the ARM GNU toolchain and an old-enough libc/sysroot, preventing newer host symbols from leaking into a C1 binary. It is compatibility isolation, not an arbitrary container preference.
- **Current implementation**: `make build` and `scripts/deploy.sh` use an archived Debian Buster container. The script rebuilds by default; `--reuse` is permitted only after confirming the binary corresponds to the checked-out source. Do not claim a target build works until `file`/`readelf` checks and a supervised TV probe pass.
- **Native-toolchain assessment (2026-09-22)**: webOS Brew's macOS arm64 `native-toolchain` SDK was downloaded, relocated, and attempted against this daemon. Its relocated Buildroot compiler retains a stale CI sysroot path; explicit target-only `--sysroot` repairs that. The resulting link still fails because the SDK libc lacks `getauxval`, required by Rust's supported `armv7-unknown-linux-gnueabi` standard library and `ring`. Do not add a fake `getauxval` shim or promote this SDK as a Rust build path. Docker remains canonical unless a custom Rust standard library is built and verified against the SDK, which is not a simplification.
- **Ares CLI status**: official Node Ares commands are already on `PATH`. `ares-cli-rs` v0.7.0 is installed separately under `~/.local/share/ares-cli-rs/v0.7.0`, checksum-verified, and intentionally does not shadow them. This repository currently uses root `ssh`/`scp` and `luna-send`; no committed script invokes Ares, and historic deployment claims are not live evidence. `ares-cli-rs` can package/install IPKs and push/shell files, including root SSH devices, but it does not by itself replace the daemon's root-owned service, Luna permissions, or webOS Brew boot hook. Adopt it first for IPK install/developer workflows; retain SSH for privileged daemon provisioning until equivalence is tested.
- **Hue Entertainment identity**: `entertainment_configuration_id` is the CLIP V2 UUID embedded in HueStream 2.0 packets and used for V2 `start`/`stop` actions. `entertainment_area_id` remains only as a backward-compatible config field and is migrated to the UUID on the next Bridge sync. A successful DTLS handshake alone does not prove lights accept packets. Keep all V2 requests certificate-pinned.
- **Hue V2 certificate boundary**: this Bridge presents a device-specific self-signed HTTPS certificate. Pairing and dashboard area-sync use a scoped SHA-256 DER certificate pin: physical Hue pushlink authorizes trust-on-first-use, then every later V2 request must match the stored fingerprint. Never disable TLS verification globally or log the fingerprint, username, client key, or Nanoleaf token.
- **Dashboard setup**: Axum serves the local dashboard. Pairing endpoints are LAN-local and require physical Hue/Nanoleaf pairing actions; they persist credentials with owner-only config permissions and request a supervised daemon restart. Do not return credentials from status or setup endpoints. Luna remains the webOS service bus for permissions/integration, not a dashboard UI framework.
- **Deployment safety**: build and static artifact inspection are local. Upload, install, service restart, autoroot, Luna mutation, or light output require explicit user direction and post-action readback; never infer success from a CLI exit code.

---

## Upstream & Reference Repositories

- **`webosbrew/tv-native-apis`**: Header files and stub definitions for `vtcapture.h`, `dile_vt.h`, and `halgal.h`.
- **`webosbrew/hyperion-webos`**: Reference C++ grabber for webOS utilizing `libvtcapture`.
- **`throwaway96/dejavuln-autoroot`**: USB autoroot exploit for webOS 3.5–8.
- **`voyvodka/LumaSync`**: Reference Rust implementation of DTLS 1.2 PSK streaming to Hue Bridge.
- **`hyperion-project/hyperion.ng`**: Official Hyperion ambient lighting engine.
