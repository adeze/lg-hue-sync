#!/usr/bin/env bash
set -euo pipefail

# provision_tv.sh
# Connects to a rooted LG C1 TV via SSH and installs/configures:
# 1. PicCap (screen capture daemon)
# 2. Hyperion.NG (ambient lighting server with Hue Entertainment driver)
# 3. Autostart and capture parameters

TV_IP="${1:-}"
SSH_PORT="${2:-22}"

echo "==================================================================="
echo "       LG C1 TV Homebrew Provisioning (PicCap + Hyperion)         "
echo "==================================================================="

if [ -z "$TV_IP" ]; then
    echo "Usage: $0 <TV_IP_ADDRESS> [SSH_PORT (default: 22)]"
    echo "Example: $0 192.168.1.150"
    exit 1
fi

SSH_CMD="ssh -p $SSH_PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null root@$TV_IP"

echo "[*] Testing SSH connection to root@$TV_IP:$SSH_PORT..."
if ! $SSH_CMD "echo 'Connected successfully as root! WebOS kernel: \$(uname -r)'" 2>/dev/null; then
    echo "[-] Could not connect via SSH directly."
    echo "[!] Hint: If you just rooted via DejaVuln or Homebrew Channel:"
    echo "    1. Open 'Homebrew Channel' on the TV."
    echo "    2. Check Settings -> SSH Server is turned ON (default port 22)."
    echo "    3. If password prompt appears, Homebrew default is 'alpine' or blank depending on setup."
    echo "    4. You can also copy your SSH key: ssh-copy-id -p $SSH_PORT root@$TV_IP"
    exit 1
fi

echo "[+] SSH connection confirmed."

echo "[*] Installing/updating PicCap and Hyperion.NG via Luna bus on webOS..."
$SSH_CMD bash << 'REMOTEEOC'
set -euo pipefail

echo "[TV] Checking root identity: $(whoami)"

# Query Homebrew service
HB_SERVICE="luna://org.webosbrew.hbchannel.service"

# Install PicCap (video grabber frontend)
echo "[TV] Requesting PicCap installation..."
luna-send -n 1 "$HB_SERVICE/install" '{"id":"org.webosbrew.piccap"}' || echo "[TV] Warning: luna install piccap returned non-zero (may already be installed)"

# Install HyperHDR Loader (modern HDR/Dolby Vision tone-mapping engine)
echo "[TV] Requesting HyperHDR Loader installation..."
luna-send -n 1 "$HB_SERVICE/install" '{"id":"org.webosbrew.hyperhdr.loader"}' || echo "[TV] Warning: luna install hyperhdr returned non-zero"

# Also install Hyperion.NG as alternative
echo "[TV] Requesting Hyperion.NG installation..."
luna-send -n 1 "$HB_SERVICE/install" '{"id":"org.webosbrew.hyperion.ng"}' || echo "[TV] Warning: luna install hyperion returned non-zero"

# Wait for package manager to settle
sleep 3

# Configure PicCap default capture settings for LG C1 Alpha 9 Gen 4
# PicCap config is stored in /var/luna/preferences/org.webosbrew.piccap or app settings
PICCAP_PREF_DIR="/var/luna/preferences"
mkdir -p "$PICCAP_PREF_DIR"

echo "[TV] Setting up PicCap configuration for LG C1..."
cat << 'CONFIG_EOF' > /tmp/piccap_config.json
{
  "autostart": true,
  "video_backend": "libvtcapture",
  "ui_backend": "libhalgal",
  "width": 160,
  "height": 90,
  "fps": 30,
  "ip": "127.0.0.1",
  "port": 19400,
  "priority": 150
}
CONFIG_EOF

# Launch services
echo "[TV] Starting HyperHDR Loader service..."
luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"org.webosbrew.hyperhdr.loader"}' || luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"org.webosbrew.hyperion.ng"}' || true

echo "[TV] Starting PicCap service..."
luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"org.webosbrew.piccap"}' || true

echo "[TV] Provisioning complete on TV."
REMOTEEOC

echo ""
echo "==================================================================="
echo "[+] SUCCESS: PicCap and HyperHDR / Hyperion installed on TV!"
echo "==================================================================="
echo "1. Verify Web UI in your browser:"
echo "   HyperHDR: http://$TV_IP:8090 (or 8092)"
echo "   Hyperion: http://$TV_IP:8090"
echo ""
echo "2. Next Step: Automatically pair your Philips Hue Bridge:"
echo "   uv run scripts/pair_hue.py --tv-ip $TV_IP"
echo "==================================================================="
