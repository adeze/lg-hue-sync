#!/usr/bin/env bash
set -euo pipefail

TV_IP="${1:-192.168.1.149}"
SSH_PORT="${2:-22}"
SSH_OPTS="-p $SSH_PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"

echo "[*] Creating webOS quick-toggle app on TV at $TV_IP..."

ssh $SSH_OPTS root@"$TV_IP" 'bash -s' << 'REMOTEEOC'
APP_DIR="/media/developer/apps/usr/palm/applications/com.adeze.lghuesync"
mkdir -p "$APP_DIR"

cat << 'EOF' > "$APP_DIR/appinfo.json"
{
  "id": "com.adeze.lghuesync",
  "version": "0.2.0",
  "vendor": "adeze",
  "type": "native",
  "main": "toggle.sh",
  "title": "Hue Sync",
  "icon": "icon.png",
  "largeIcon": "icon.png",
  "miniicon": "icon.png",
  "noSplash": true,
  "supportQuickStart": false
}
EOF

cat << 'EOF' > "$APP_DIR/toggle.sh"
#!/bin/sh
if systemctl is-active --quiet lg-hue-sync; then
    systemctl stop lg-hue-sync
    luna-send -n 1 luna://com.webos.notification/createToast '{"message": "Philips Hue Sync: OFF", "iconUrl": ""}' || true
else
    systemctl start lg-hue-sync
    luna-send -n 1 luna://com.webos.notification/createToast '{"message": "Philips Hue Sync: ON", "iconUrl": ""}' || true
fi
EOF

chmod +x "$APP_DIR/toggle.sh"

if [ ! -f "$APP_DIR/icon.png" ]; then
    echo "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==" | base64 -d > "$APP_DIR/icon.png" || true
fi

luna-send -n 1 luna://com.webos.applicationManager/rescanAppList '{}' || true
echo "[TV] Registered 'Hue Sync' app! You can now hold '0' on your LG Magic Remote and bind it to a number key (e.g. 9)."
REMOTEEOC

echo "[+] Done! 'Hue Sync' is now installed on the TV."
