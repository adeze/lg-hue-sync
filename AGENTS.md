# LG Hue Sync — Architecture & Agent Guide

Native screen capture and Philips Hue Entertainment synchronization for LG webOS Smart TVs, tailored for the **LG C1 (2021, webOS 6.x, Alpha 9 Gen 4)**.

## Project Scope & Objectives

1. **Automation Suite (`scripts/`)**:
   - Automated USB payload staging for root acquisition via DejaVuln (`dejavuln-autoroot`).
   - Remote TV provisioning over SSH: automated installation of the Homebrew Channel (`org.webosbrew.hbchannel`), PicCap (`org.webosbrew.piccap`), and Hyperion.NG (`org.webosbrew.hyperion.ng`).
   - Automated Hue Bridge discovery, pushlink authentication, Entertainment Area parsing, and Hyperion JSON-RPC configuration.

2. **Native Rust Daemon (`lg-hue-sync`)**:
   - Single-binary replacement for PicCap + Hyperion.NG.
   - Low-overhead direct FFI binding to webOS private display pipelines (`libvtcapture.so` / `libdile_vt.so`).
   - Native SIMD/downsampling color extraction into spatial TV zones (top, bottom, left, right, center).
   - High-performance DTLS 1.2 PSK streaming over UDP port 2100 directly to the Philips Hue Bridge using the Hue Entertainment API.

---

## Hardware & Architecture Specifics

- **Target Device**: LG C1 OLED (2021)
- **SoC**: MediaTek/Realtek customized Alpha 9 Gen 4 (Cortex-A73/A53).
- **Userspace**: webOS 6.x runs a **32-bit ARM GNU userspace** (`armv7l`, glibc 2.28+).
- **Compilation Target**:
  - Architecture: `armv7-unknown-linux-gnueabihf` (standard glibc) or `armv7-unknown-linux-musleabihf` (fully static).
  - Cross-compilation tool: `cross` (Docker containerized) or native toolchain `arm-linux-gnueabihf-gcc`.

---

## Security & Root Invariants

- **Root Access Mandatory**: `libvtcapture` and access to `/dev/video*` hardware decoders are blocked by webOS user sandboxes. The daemon and grabbers must run as `root`.
- **Automatic Updates Must Be Disabled**: LG pushes firmware patches that mitigate root exploits. The TV must have "Allow Automatic Updates" turned OFF.
- **DRM Content Boundary**:
  - **HDMI Inputs**: PS5, Xbox, Apple TV 4K, Shield TV, Switch, and PC inputs are fully captured at low latency.
  - **Internal DRM Apps**: Native webOS streaming apps (Netflix, Disney+, Prime Video) route video through hardware TrustZone secure video planes. `libvtcapture` will yield black frames for these streams. Use an external HDMI streaming device for ambient sync with DRM content.

---

## Network Ports & Protocols

| Service | Port | Protocol | Purpose |
| :--- | :--- | :--- | :--- |
| **TV SSH** | 22 / 9922 | SSH | Remote management and root execution |
| **Hyperion Flatbuffers** | 19400 | TCP/UDP | PicCap raw frame transport to Hyperion |
| **Hyperion JSON-RPC** | 8090 | HTTP/JSON | Hyperion web interface & remote config API |
| **Hue REST API** | 80 / 443 | HTTP/HTTPS | Pairing, pushlink authentication, area config |
| **Hue Entertainment** | 2100 | UDP / DTLS 1.2 | Real-time packed RGB streaming (25–60 Hz) |

---

## Upstream & Reference Repositories

- **`webosbrew/tv-native-apis`**: Header files and stub definitions for `vtcapture.h`, `dile_vt.h`, and `halgal.h`.
- **`webosbrew/hyperion-webos`**: Reference C++ grabber for webOS utilizing `libvtcapture`.
- **`throwaway96/dejavuln-autoroot`**: USB autoroot exploit for webOS 3.5–8.
- **`voyvodka/LumaSync`**: Reference Rust implementation of DTLS 1.2 PSK streaming to Hue Bridge.
- **`hyperion-project/hyperion.ng`**: Official Hyperion ambient lighting engine.
