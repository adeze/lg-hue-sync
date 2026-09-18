#!/usr/bin/env bash
set -euo pipefail

# prepare_dejavuln_usb.sh
# Downloads the latest DejaVuln autoroot exploit and stages it onto a FAT32 USB drive
# for rooting LG webOS TVs (including LG C1 on firmware 03.53.45).

REPO="throwaway96/dejavuln-autoroot"
TARGET_DIR="${1:-}"

echo "==================================================================="
echo "    LG webOS DejaVuln USB Preparation Tool (LG C1 03.53.45)        "
echo "==================================================================="

if [ -z "$TARGET_DIR" ]; then
    echo "Usage: $0 <path-to-usb-drive-or-staging-dir>"
    echo "Example (macOS): $0 /Volumes/MY_USB"
    echo ""
    echo "Listing currently mounted external volumes on macOS:"
    ls -d /Volumes/* | grep -v "Macintosh HD" || true
    echo ""
    exit 1
fi

if [ ! -d "$TARGET_DIR" ]; then
    echo "[-] Error: Target directory '$TARGET_DIR' does not exist."
    exit 1
fi

TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

echo "[*] Querying latest DejaVuln release from GitHub ($REPO)..."
LATEST_RELEASE_JSON=$(curl -s "https://api.github.com/repos/$REPO/releases/latest")
DOWNLOAD_URL=$(echo "$LATEST_RELEASE_JSON" | grep "browser_download_url" | cut -d '"' -f 4 | grep -E '\.zip$' | head -n 1)
TAG_NAME=$(echo "$LATEST_RELEASE_JSON" | grep "tag_name" | cut -d '"' -f 4 | head -n 1)

if [ -z "$DOWNLOAD_URL" ]; then
    echo "[-] Failed to fetch download URL. Falling back to known release v0.0.10..."
    DOWNLOAD_URL="https://github.com/throwaway96/dejavuln-autoroot/releases/download/v0.0.10/dejavuln-autoroot-0.0.10.zip"
    TAG_NAME="v0.0.10"
fi

echo "[+] Found release $TAG_NAME"
echo "[*] Downloading: $DOWNLOAD_URL"
curl -L -o "$TMP_DIR/dejavuln.zip" "$DOWNLOAD_URL"

echo "[*] Extracting files into $TARGET_DIR..."
unzip -q -o "$TMP_DIR/dejavuln.zip" -d "$TARGET_DIR"

echo ""
echo "==================================================================="
echo "[+] SUCCESS: DejaVuln autoroot payload staged to $TARGET_DIR"
echo "==================================================================="
echo ""
echo "NEXT STEPS ON YOUR LG C1 TV:"
echo "1. Safely eject the USB drive from your Mac and insert it into a USB port on your LG C1."
echo "2. A notification popup will appear on the TV. Select 'Music' (or launch the TV's built-in Music app)."
echo "3. Navigate into the USB drive in the Music app."
echo "4. Play the exploit audio track from the USB drive."
echo "5. The exploit will trigger automatically in the background, root the TV, and install the webOS Homebrew Channel."
echo "6. Press the Home button on your magic remote; verify that 'Homebrew Channel' appears in your app list."
echo "7. Launch Homebrew Channel, open Settings:"
echo "   - Enable 'SSH Server' (default port: 22)"
echo "   - Verify 'Root status' shows OK"
echo "   - Ensure 'Telnet' is disabled for network security"
echo "8. Take note of the TV's IP address (Settings -> Network -> Wi-Fi Connection)."
echo "9. Proceed to run: ./scripts/provision_tv.sh <TV_IP>"
echo "==================================================================="
