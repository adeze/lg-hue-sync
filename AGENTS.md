# LG Hue Sync — Architecture & Agent Guide

Native screen capture and Philips Hue Entertainment synchronization for LG webOS Smart TVs, specifically configured for:
- **Target Device**: LG C1 OLED 2021 (MAC: `24:E8:53:D7:55:4C`, IP: `192.168.1.149`)
- **Firmware**: webOS 6.x (Software Version `03.53.45`)
- **SoC**: MediaTek/Realtek customized Alpha 9 Gen 4 (64-bit ARM Cortex-A73/A53 with 32-bit `armv7l` GNU userspace)

---

## Tooling & Environment Discipline

- **Python Scripting**: Always execute Python scripts using `uv run <script.py>`. Never invoke `python3` or `python` directly.
- **Rust Tooling**: Cross-compile for webOS using `cross build --target armv7-unknown-linux-gnueabihf --release` or local `cargo` for test/mock validation.
- **File Editing**: Always use designated Antigravity editing tools (`write_to_file`, `replace_file_content`), never ad-hoc shell redirects.

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
# Step 1: Stage the DejaVuln autoroot USB exploit
./scripts/prepare_dejavuln_usb.sh /Volumes/<USB_DRIVE>

# Step 2: Once TV is rooted and SSH is enabled in Homebrew Channel:
./scripts/provision_tv.sh 192.168.1.149

# Step 3: Discover Hue Bridge and pair pushlink button:
uv run scripts/pair_hue.py --tv-ip 192.168.1.149

# Step 4: (Alternative) Run native Rust Hue sync daemon:
./target/debug/lg-hue-sync test-capture --config config.example.json
```

---

## Upstream & Reference Repositories

- **`webosbrew/tv-native-apis`**: Header files and stub definitions for `vtcapture.h`, `dile_vt.h`, and `halgal.h`.
- **`webosbrew/hyperion-webos`**: Reference C++ grabber for webOS utilizing `libvtcapture`.
- **`throwaway96/dejavuln-autoroot`**: USB autoroot exploit for webOS 3.5–8.
- **`voyvodka/LumaSync`**: Reference Rust implementation of DTLS 1.2 PSK streaming to Hue Bridge.
- **`hyperion-project/hyperion.ng`**: Official Hyperion ambient lighting engine.
