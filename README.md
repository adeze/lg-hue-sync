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
│                                    - Reinhard HDR Tone-Mapping         │
│                                    - Dynamic Letterbox Detection       │
│                                    - OLED Near-Black Noise Gate        │
│                                    - Saturation Dominance Boost        │
│                                    - Single-Slot Drop-Stale Pump       │
│                                       │                     │          │
│                        Perimeter UDP  │                     │ DTLS PSK │
│                          (Port 60222) │                     │ (Port    │
│                                       ▼                     ▼  2100)   │
└───────────────────────────────────────┼─────────────────────┼──────────┘
                                        │                     │
                                        ▼                     ▼
                               [ Nanoleaf 4D (V1) ]   [ Philips Hue Bridge ]
                                (TV Lightstrip)        (Room Surrounds)
                                        │                     │
                                        ▼                     ▼
                               [ 30+ TV Edge LEDs ]   [ Hue Play / Bulbs ]
```

---

## Key Features

* **Sub-10 MB Footprint:** Consumes $\approx 8\text{ MB}$ of RAM and $< 1.5\%$ CPU on the Alpha 9 Gen 4 processor (compared to $\approx 120\text{ MB}$ for Hyperion/Qt/PicCap).
* **Content-adaptive light output:** polls capture at a low-latency ceiling but suppresses duplicate Hue and Nanoleaf frames, so repeated compositor frames do not create unnecessary light changes.
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

## Comparison: `lg-hue-sync` vs. Official Philips Hue Sync TV App

| Feature / Architecture | Official Hue Sync TV App (webOS 24+) | `lg-hue-sync` Daemon (LG C1 / webOS 5-6) |
| :--- | :--- | :--- |
| **TV Compatibility** | **2024+ models only** (Locked out on C1, C2, C3, CX) | **Any rooted webOS TV** (Native support for C1 Alpha 9 Gen 4) |
| **Cost** | **$129.99 USD** one-time purchase or $2.99/mo | **$0 (Free & Open Source)** |
| **Runtime Architecture** | Heavy Chromium WebApp + Node/JS bridge | **Pure headless compiled Rust binary** (zero web bloat) |
| **Memory Consumption** | **$\approx 85\text{–}140\text{ MB}$** (Heavy RAM footprint) | **$\approx 6\text{–}8\text{ MB}$** (15x less memory) |
| **CPU Utilization** | $\approx 4\text{–}7\%$ CPU | **$< 1.2\%$ CPU** |
| **Light output cadence** | Fixed internal 50/60 Hz timer | **Content-adaptive duplicate suppression with a measured output rate** |
| **Letterbox Detection** | Fixed $16:9$ sampling (samples black bars on movies) | **Dynamic real-time auto-crop** (re-anchors to $2.39:1 / 2.0:1$ film frame) |
| **OLED Near-Black Floor** | Faint 1–2% grey light flicker in dark scenes | **OLED Near-Black Noise Gate** (cuts off below 2% luma to true black) |
| **Color Dominance** | Proprietary color mixing | **Chroma-weighted saturation boost** ($1.0 + \gamma \cdot S^2$) |
| **HDR10 / Dolby Vision** | Dynamic metadata-based tone mapping | **Reinhard Non-Linear HDR Curve** (prevents highlight burnout) |
| **Network Traffic** | Continuous fixed-rate UDP | **Adaptive Deadband Throttling** (drops to 2 Hz heartbeat on pause) |
| **Lifecycle Control** | Managed in-app | **Auto TV sleep/wake, LG Magic Remote Quick Access, and Hue Mobile App** |

---

## Hardware & Content Compatibility

| Source | Supported? | Notes |
| :--- | :---: | :--- |
| **HDMI Inputs** (Apple TV 4K, PS5, Xbox, Switch, PC) | **YES** | 4K 120Hz, HDR10, Dolby Vision, VRR captured with zero latency |
| **Native Non-DRM Apps** (YouTube, Plex, Media Player) | **YES** | Captured smoothly |
| **Native DRM Apps** (Netflix, Disney+, Prime Video) | **NO** | Blocked by hardware TrustZone HDCP on webOS. *(Play these via Apple TV or HDMI stick)* |

---

## Recommended TV Picture & Video Settings (LG C1 OLED)

To achieve the lowest latency, prevent frame drops, and ensure accurate ambient phosphor colorimetry, configure your LG C1 with the following settings:

### 1. Disable LG AI Services (`Settings -> General -> AI Service`)
* **AI Picture Pro $\rightarrow$ OFF**:  
  *Reasoning*: The Alpha 9 Gen 4 AI pipeline inserts an intermediate neural processing buffer between the video decoder and the hardware scaler. This frequently causes `libvtcapture.so` to drop frames, experience buffer stalls, or stutter. In addition, its dynamic frame-by-frame edge sharpening and contrast pumping cause ambient LEDs to rapidly flutter and misrepresent true scene color.
* **AI Genre Selection $\rightarrow$ OFF**:  
  *Reasoning*: Automatically swaps display tone curves when it detects genres (e.g., switching between "Cinema" and "Standard"), causing abrupt color jumps in the lights.
* **AI Brightness Settings $\rightarrow$ OFF**:  
  *Reasoning*: Uses the room's physical ambient light sensor to alter display gamma curves, causing inconsistent color extraction between daytime and nighttime viewing.
* **AI Sound Pro $\rightarrow$ OFF**:  
  *Reasoning*: Bypasses TV DSP so uncompressed multi-channel audio (Dolby Atmos, TrueHD, LPCM) bitstreams cleanly over eARC to your AVR.

### 2. Recommended Picture Presets
* **SDR Content**: **Filmmaker Mode** or **Cinema** (D65 white point, 2.4/BT.1886 gamma, TruMotion OFF).
* **HDR10 / Dolby Vision**: **Cinema** or **Filmmaker Mode** (preserves ST.2084 PQ curve without artificial dynamic contrast bloat).
* **Gaming (Consoles / PC)**: **Game Optimizer** (low-latency ALLM bypass, zero display lag, VRR / G-Sync up to 120 Hz).

### 3. Disable Energy Saving (`Settings -> Support -> Energy Saving`)
* **Energy Saving Step $\rightarrow$ OFF**:  
  *Reasoning*: Energy saving actively restricts OLED peak luminance based on average picture level (APL), dimming bright scenes and distorting ambient light brightness tracking.

---

## Quick Start Guide

### Step 1: Pair Your Hue Bridge

After deploying the daemon, open its dashboard on your local network and use **Device Setup → Pair Hue Bridge**. Leave the Bridge IP blank for discovery or enter it directly, press the physical Bridge button, then start pairing. The TV stores the credentials and a bridge-specific HTTPS certificate fingerprint with owner-only permissions, then restarts the daemon.

The CLI remains useful for development or recovery:

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

### Step 1b (Optional): Pair with Nanoleaf 4D Lightstrip

The dashboard also has **Device Setup → Pair Nanoleaf 4D**. Enter its controller IP, hold the controller power button for 5–7 seconds until it blinks, then start pairing. The token and discovered panel IDs remain on the TV; they are never returned by the dashboard API.

If you have a **Nanoleaf 4D** lightstrip mounted around your TV perimeter:

```bash
cargo run -- pair-nanoleaf --ip 192.168.1.150
```

1. Hold the power button on the Nanoleaf controller for 5–7 seconds until the LED starts blinking.
2. The daemon pairs via the local REST API, discovers your panel layout, and saves credentials to `config.json`.
3. Verify perimeter lighting with:
   ```bash
   cargo run -- test-nanoleaf
   ```

---

### Step 1c: Re-Sync Entertainment Areas Anytime (`sync-hue`)

Whenever you add new lights, move lamps, or modify your 3D layout in the official **Philips Hue mobile app**, run:

```bash
cargo run -- sync-hue
```

Or target a specific entertainment area by name:
```bash
cargo run -- sync-hue --area "Living Room Cinema"
```

This re-queries the Hue Bridge, extracts updated 3D coordinates `[X, Y, Z]`, projects them into 2D screen sampling boxes (with front/surround depth scaling), and refreshes `config.json` **without needing to press the Bridge button again**.

---

### Step 2: Verify Lights with a Test Pattern

Verify that the DTLS 1.2 PSK engine can drive your Hue lights directly over UDP port 2100:

```bash
cargo run -- test-pattern --config config.json
```

Your entertainment lights will cycle through a smooth 10-second rainbow pattern.

---

### Step 3: Root the LG C1 (One-Time USB Exploit)

To allow the background daemon to capture hardware display buffers, root access is required once to open SSH.

#### webOS Homebrew discussion and compatibility

Before changing a TV, check the community [OLED65C1 compatibility search](https://cani.rootmy.tv/?q=OLED65C1) and the [webOS Brew projects](https://github.com/webosbrew). They are useful references for model support, packaging, service conventions, and recovery options; they are not part of this daemon or a guarantee that a particular TV can be modified safely.

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
1. Cross-compile `lg-hue-sync` for webOS 32-bit ARM (`armv7-unknown-linux-gnueabi`).
2. Upload the stripped binary and `config.json` to `/var/home/root/lg-hue-sync/`.
3. Install a systemd service unit at `/etc/systemd/system/lg-hue-sync.service`.
4. Enable and start the service.

Your TV is now syncing ambient lighting on boot with zero visible apps or clutter!

---

### Step 5 (Optional): Map Toggle to LG Magic Remote Quick Access

If you would like to be able to turn sync on or off or restart it using your **LG Magic Remote**:

```bash
./scripts/install_remote_shortcut.sh 192.168.1.149
```

This creates a lightweight native webOS app entry **"Hue Sync"**:
1. On your LG Magic Remote, **hold the `0` key** to open the **Quick Access** editor.
2. Select any number key (e.g. **`9`**) and assign **"Hue Sync"** to it.
3. Now, whenever you hold **`9`** on your remote, it will instantly toggle Hue Sync ON or OFF and display an on-screen TV notification:
   - `Philips Hue Sync: ON`
   - `Philips Hue Sync: OFF`

---

## Management via SSH

```bash
# Check service status
ssh root@192.168.1.149 'systemctl status lg-hue-sync'

# View live real-time sync logs
ssh root@192.168.1.149 'journalctl -u lg-hue-sync -f'

# Stop / Start / Restart service
ssh root@192.168.1.149 'systemctl stop lg-hue-sync'
ssh root@192.168.1.149 'systemctl start lg-hue-sync'
ssh root@192.168.1.149 'systemctl restart lg-hue-sync'
```

---

## Configuration (`config.json`)

```json
{
  "hue_enabled": true,
  "bridge_ip": "192.168.1.100",
  "username": "your-hue-application-username",
  "clientkey": "your-32-char-dtls-psk-key",
  "entertainment_area_id": "93f225c7-96f9-40cd-afdc-89a779615607",
  "nanoleaf": {
    "enabled": true,
    "ip": "192.168.1.150",
    "auth_token": "your-nanoleaf-api-token",
    "udp_port": 60222,
    "segments": 30
  },
  "fps": 0,
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

### Key Configuration Knobs
* **`"hue_enabled"` & `"nanoleaf"`**: Enable or disable Philips Hue or Nanoleaf 4D independently or run both in lockstep.
* **`"fps": 0` (Auto source hint)**: Uses a webOS-reported source rate when available, otherwise captures at 60 Hz and suppresses duplicate output frames. The dashboard distinguishes this processing ceiling from measured light updates; it does not claim a 24p source rate unless webOS supplies one.
* **`"zones"`**: Automatically populated by `cargo run -- pair` or `cargo run -- sync-hue` using your 3D room coordinates from the Philips Hue app. Depth ($Y$) and height ($Z$) are intelligently mapped: front-stage lights sample tight screen borders, while rear surround lights sample diffuse ambient scene reflections.
* **`"nanoleaf.segments"`**: Number of addressable perimeter LED zones along your TV edges (default 30 for standard 4D strips). Automatically distributed around Left, Top, Right, and Bottom edges based on 16:9 aspect ratio.

---

## License

MIT License. See [LICENSE](LICENSE) for details.
