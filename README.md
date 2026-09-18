# lg-hue-sync

**High-Performance Native Philips Hue Entertainment Synchronizer for LG webOS**  
*Specifically tailored for the LG C1 OLED (2021, Alpha 9 Gen 4) and compatible webOS 5.x / 6.x Smart TVs.*

[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org)
[![webOS](https://img.shields.io/badge/webOS-5.x%20%7C%206.x-blue.svg)](https://www.webosbrew.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

---

## Overview

Official Philips Hue Sync apps are only available on 2024+ LG TVs running webOS 24+. Older premium OLEDs like the **LG C1 (2021)** are left unsupported, forcing owners to either buy an expensive external HDMI Sync Box ($250+) or install heavy multi-app stacks (Hyperion + PicCap) that consume substantial RAM, run unnecessary web servers, and clutter the TV menu.

**`lg-hue-sync`** provides a **pure headless Rust daemon** that runs silently as a Linux `systemd` service directly on the TV:
- Directly grabs downscaled frames from the TV's hardware scaler using private `libvtcapture.so` FFI.
- Applies **Hue Gamut C** chromaticity clamping and **CIE 1931 xy** mode for true-to-life LED phosphor colorimetry.
- Applies real-time **OLED HDR10 & Dolby Vision tone-mapping** to prevent bright highlights from clipping to flat white.
- Streams encrypted **DTLS 1.2 PSK** binary frames directly to your Philips Hue Bridge over UDP port 2100.
- **Pure headless operation**: No Homebrew Channel app required on your launcher, no web UIs, and zero extra apps on your TV screen.

---

## Architecture

```
┌────────────────────────────────────────────────────────────────────────┐
│                        LG C1 TV (webOS 6.x)                            │
│                                                                        │
│   [ Apple TV / PS5 / HDMI ] ──► [ SoC Video Processing Pipeline ]      │
│                                              │                         │
│                                              ▼ Hardware Scaler         │
│                                    [ libvtcapture.so ]                 │
│                                              │ Zero-Copy FFI           │
│                                              ▼                         │
│                                    [ lg-hue-sync daemon ]              │
│                                    - Spatial Zone Sampler              │
│                                    - Reinhard HDR Tone-Mapping         │
│                                    - Hue Gamut C Boundary Clamp        │
│                                    - CIE 1931 xy Quantizer             │
│                                    - Single-Slot Drop-Stale Pump       │
│                                              │                         │
│                                              ▼ DTLS 1.2 PSK            │
└──────────────────────────────────────────────┼─────────────────────────┘
                                               │ UDP Port 2100 (24-60Hz)
                                               ▼
                                    [ Philips Hue Bridge ]
                                               │ Zigbee Light Link
                                               ▼
                                 [ Play Gradient / Bulbs ]
```

---

## Key Features

* **Sub-10 MB Footprint:** Consumes $\approx 8\text{ MB}$ of RAM and $< 1.5\%$ CPU on the Alpha 9 Gen 4 processor (compared to $\approx 120\text{ MB}$ for Hyperion/Qt/PicCap).
* **Apple TV Frame Matching (24 & 30 FPS):** Fully supports 23.976, 24.000, 29.970, and 30.000 FPS cadences to match Apple TV 4K "Match Frame Rate", eliminating micro-stutter and phase beating between film cuts and ambient lights.
* **Dynamic Letterbox / Aspect Ratio Auto-Detection:** Automatically detects top/bottom black bars on $2.39:1 / 2.0:1$ widescreen films and re-anchors spatial sampling to the active film frame instead of black pixels.
* **Saturation-Weighted Dominant Color Sampling:** Weights pixels by chroma saturation ($1.0 + \gamma \cdot S^2$) so vibrant accent colors, neon signs, and explosions punch through without being diluted into muddy grey/brown by dark backgrounds.
* **Scene-Cut Instant Snap & Adaptive EMA:** Snaps instantly ($\alpha = 1.0$) with zero latency on hard camera cuts while maintaining smooth ($\alpha = 0.35$) temporal tracking during camera pans.
* **OLED Near-Black Noise Gate:** Clamps video compression noise and dither ($< 2\%$ luminance) to true zero, preventing distracting light flicker in pitch-black scenes.
* **Adaptive Deadband Throttling:** When screen colors are static or paused ($\Delta < 1\%$), throttles UDP transmissions down to a 2 Hz keep-alive heartbeat, reducing Bridge CPU load and WiFi airtime by 90%.
* **OLED HDR10 & Dolby Vision Tone-Mapping:** Built-in Reinhard curve ($C_{out} = \frac{C}{C + 0.25} \times 1.25$) prevents bright HDR highlights from clipping to flat white.
* **Color Accuracy via Hue Gamut C Clamping:** Clamps out-of-gamut sRGB values to the official Philips Hue Gamut C triangle boundaries (Red `[0.6915, 0.3083]`, Green `[0.17, 0.7]`, Blue `[0.1532, 0.0475]`), preventing color skewing and sudden hue jumps.
* **Production systemd Watchdog Integration:** Integrates `sd-notify` with `WatchdogSec=10` and live status reporting (`systemctl status lg-hue-sync`).
* **Single-Slot Drop-Stale Pump:** Real-time single-frame buffer ensures zero latency accumulation during fast-paced action or gaming.

---

## Hardware & Content Compatibility

| Source | Supported? | Notes |
| :--- | :---: | :--- |
| **HDMI Inputs** (Apple TV 4K, PS5, Xbox, Switch, PC) | **YES** | 4K 120Hz, HDR10, Dolby Vision, VRR captured with zero latency |
| **Native Non-DRM Apps** (YouTube, Plex, Media Player) | **YES** | Captured smoothly |
| **Native DRM Apps** (Netflix, Disney+, Prime Video) | **NO** | Blocked by hardware TrustZone HDCP on webOS. *(Play these via Apple TV or HDMI stick)* |

---

## Quick Start Guide

### Step 1: Pair Your Hue Bridge (Run Locally on Mac)

Ensure your Mac is connected to the same local network as your Hue Bridge:

```bash
# Clone the repository
git clone https://github.com/adeze/lg-hue-sync.git
cd lg-hue-sync

# Discover bridge, press link button, and auto-generate config.json
cargo run -- pair
```

When prompted, press the physical round button on your Hue Bridge. The CLI will:
1. Negotiate your DTLS Pre-Shared Key (`clientkey`).
2. Discover your configured Entertainment Areas and spatial light coordinates.
3. Save your credentials to `config.json`.

---

### Step 2: Verify Lights with a Test Pattern

Verify that the DTLS 1.2 PSK engine can drive your lights directly over UDP port 2100:

```bash
cargo run -- test-pattern --config config.json
```

Your entertainment lights will cycle through a smooth 10-second rainbow pattern.

---

### Step 3: Root the LG C1 (One-Time USB Exploit)

To allow the background daemon to capture hardware display buffers, root access is required once to open SSH.

1. Format a USB flash drive as **FAT32**.
2. Run the staging script:
   ```bash
   ./scripts/prepare_dejavuln_usb.sh /Volumes/<YOUR_USB_DRIVE>
   ```
3. Insert the USB into the LG C1.
4. Launch the TV's built-in **Music** app, open the USB drive, and play the exploit track.
5. The exploit gains root access and enables the SSH daemon on port 22.

*(Note: You do not need to keep or use the Homebrew Channel app. You can safely ignore or hide it from the webOS launcher.)*

---

### Step 4: Deploy the Headless Daemon to the TV

Run the automated deployment script with your TV's IP address:

```bash
./scripts/deploy.sh 192.168.1.149
```

The script will:
1. Cross-compile `lg-hue-sync` for webOS 32-bit ARM (`armv7-unknown-linux-gnueabihf`).
2. Upload the stripped binary and `config.json` to `/var/home/root/lg-hue-sync/`.
3. Install a systemd service unit at `/etc/systemd/system/lg-hue-sync.service`.
4. Enable and start the service.

Your TV is now syncing ambient lighting on boot with zero visible apps or clutter!

---

## Management via SSH

```bash
# Check service status
ssh root@192.168.1.149 'systemctl status lg-hue-sync'

# View live real-time sync logs
ssh root@192.168.1.149 'journalctl -u lg-hue-sync -f'

# Stop / Start service
ssh root@192.168.1.149 'systemctl stop lg-hue-sync'
ssh root@192.168.1.149 'systemctl start lg-hue-sync'
```

---

## Configuration (`config.json`)

```json
{
  "bridge_ip": "192.168.1.100",
  "username": "your-hue-application-username",
  "clientkey": "your-32-char-dtls-psk-key",
  "entertainment_area_id": "93f225c7-96f9-40cd-afdc-89a779615607",
  "fps": 24,
  "brightness_multiplier": 1.0,
  "use_xy_gamut": true,
  "hdr_tone_mapping": true,
  "letterbox_detection": true,
  "saturation_boost": 1.5,
  "noise_gate_threshold": 0.02,
  "adaptive_throttling": true,
  "zones": [
    { "channel_id": 0, "name": "Left",   "x_min": 0.0, "x_max": 0.25, "y_min": 0.1, "y_max": 0.9 },
    { "channel_id": 1, "name": "Top",    "x_min": 0.2, "x_max": 0.8,  "y_min": 0.0, "y_max": 0.3 },
    { "channel_id": 2, "name": "Right",  "x_min": 0.75, "x_max": 1.0, "y_min": 0.1, "y_max": 0.9 },
    { "channel_id": 3, "name": "Bottom", "x_min": 0.2, "x_max": 0.8,  "y_min": 0.7, "y_max": 1.0 }
  ]
}
```

---

## License

MIT License. See [LICENSE](LICENSE) for details.
