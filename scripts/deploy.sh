#!/usr/bin/env bash
set -euo pipefail

# deploy.sh
# Cross-compiles the native Rust daemon for LG webOS 32-bit ARM (Alpha 9 Gen 4)
# and deploys it over SSH to the LG C1 TV as a headless systemd service.

TV_IP="${1:?Usage: ./scripts/deploy.sh <tv-ip> [ssh-port] [--reuse] [--with-config]}"
SSH_PORT="${2:-22}"
TARGET="armv7-unknown-linux-gnueabi"
REMOTE_DIR="/var/home/root/lg-hue-sync"
REUSE=false
WITH_CONFIG=false
for option in "${@:3}"; do
    case "$option" in
        --reuse) REUSE=true ;;
        --with-config) WITH_CONFIG=true ;;
        *) echo "Unknown option: $option" >&2; exit 2 ;;
    esac
done

if $WITH_CONFIG && [ ! -f "config.json" ]; then
    echo "[-] --with-config requested but config.json is missing." >&2
    exit 1
fi

echo "==================================================================="
echo "       LG C1 Native Rust Hue Sync Deployment ($TARGET)            "
echo "==================================================================="

# 1. Build by default; reuse only when explicitly requested.
BINARY="target/$TARGET/release/lg-hue-sync"

if [ -f "$BINARY" ] && $REUSE; then
    echo "[+] Using existing compiled binary at $BINARY ($(du -h "$BINARY" | cut -f1))"
    echo "    (Pass --reuse as 3rd argument only after verifying it matches this source tree)"
else
    echo "[*] Compiling release binary for $TARGET matching webOS 6.x glibc 2.28 in container..."
    make build
fi

if [ ! -f "$BINARY" ]; then
    echo "[-] Error: Compiled binary not found at $BINARY"
    exit 1
fi

echo "[+] Binary ready: $(du -h "$BINARY" | cut -f1)"

# 2. Check TV reachability and root status
COMMON_OPTS="-o StrictHostKeyChecking=accept-new -o ConnectTimeout=5"
SSH_OPTS="-p $SSH_PORT $COMMON_OPTS"
SCP_OPTS="-P $SSH_PORT $COMMON_OPTS"

if ! nc -z -w 3 "$TV_IP" "$SSH_PORT" &>/dev/null; then
    echo "[-] Root SSH is unavailable at $TV_IP:$SSH_PORT; rooting is a separate owner action." >&2
    exit 1
fi

echo "[*] Connecting to root@$TV_IP:$SSH_PORT..."
ssh $SSH_OPTS root@"$TV_IP" "mkdir -p $REMOTE_DIR"

echo "[*] Uploading binary to TV..."
scp $SCP_OPTS "$BINARY" root@"$TV_IP":"$REMOTE_DIR/lg-hue-sync.new"
LOCAL_SHA="$(shasum -a 256 "$BINARY" | awk '{print $1}')"
REMOTE_SHA="$(ssh $SSH_OPTS root@"$TV_IP" "sha256sum $REMOTE_DIR/lg-hue-sync.new" | awk '{print $1}')"
if [ "$LOCAL_SHA" != "$REMOTE_SHA" ]; then
    echo "[-] Binary checksum mismatch; existing binary was not replaced." >&2
    exit 1
fi
ssh $SSH_OPTS root@"$TV_IP" "systemctl stop lg-hue-sync 2>/dev/null || true; if [ -x $REMOTE_DIR/lg-hue-sync ]; then cp -f $REMOTE_DIR/lg-hue-sync $REMOTE_DIR/lg-hue-sync.previous; fi; mv -f $REMOTE_DIR/lg-hue-sync.new $REMOTE_DIR/lg-hue-sync; chmod +x $REMOTE_DIR/lg-hue-sync"

if $WITH_CONFIG; then
    echo "[*] Uploading local config.json..."
    scp $SCP_OPTS config.json root@"$TV_IP":"$REMOTE_DIR/config.json"
    ssh $SSH_OPTS root@"$TV_IP" "chmod 600 $REMOTE_DIR/config.json"
fi

# 3. Upload and execute Luna Service 2 provisioning script
echo "[*] Uploading provision_luna.sh to TV..."
scp $SCP_OPTS "$(dirname "$0")/provision_luna.sh" root@"$TV_IP":"$REMOTE_DIR/provision_luna.sh"
ssh $SSH_OPTS root@"$TV_IP" "chmod +x $REMOTE_DIR/provision_luna.sh"

echo "[*] Provisioning Luna permissions for org.webosbrew.lg-hue-sync on TV..."
ssh $SSH_OPTS root@"$TV_IP" "$REMOTE_DIR/provision_luna.sh local"

# 4. Package and deploy webOS Application to Home Dashboard ribbon
if [ -d "webos-app" ]; then
    echo "[*] Packaging and installing webOS application for Home Dashboard..."
    uv run scripts/package_ipk.py
    IPK_PATH="$(find target -maxdepth 1 -name 'org.webosbrew.lg-hue-sync_*_all.ipk' -type f | sort | tail -1)"
    if [ -n "$IPK_PATH" ]; then
        scp $SCP_OPTS "$IPK_PATH" root@"$TV_IP":"/tmp/org.webosbrew.lg-hue-sync.ipk"
        ssh $SSH_OPTS root@"$TV_IP" "luna-send -n 1 -f luna://com.webos.appInstallService/dev/install '{\"id\":\"org.webosbrew.lg-hue-sync\", \"ipkUrl\":\"/tmp/org.webosbrew.lg-hue-sync.ipk\", \"subscribe\":false}' || true"
    fi
fi

# 5. Create persistent systemd unit & webosbrew init.d autostart hook
echo "[*] Installing persistent webosbrew autostart hook on TV..."
ssh $SSH_OPTS root@"$TV_IP" bash << 'REMOTEEOC'
set -e
cat << 'EOF' > /var/home/root/lg-hue-sync/lg-hue-sync.service
[Unit]
Description=LG C1 Native Philips Hue & Nanoleaf Sync Daemon
After=network.target
Conflicts=sleep.target suspend.target

[Service]
Type=simple
WorkingDirectory=/var/home/root/lg-hue-sync
ExecStart=/bin/sh -c 'exec /var/home/root/lg-hue-sync/lg-hue-sync run --config /var/home/root/lg-hue-sync/config.json >> /var/home/root/lg-hue-sync/daemon.log 2>&1'
Restart=always
RestartSec=5
TimeoutStopSec=3

[Install]
WantedBy=multi-user.target
EOF

mkdir -p /var/lib/webosbrew/init.d
cat << 'EOF' > /var/lib/webosbrew/init.d/50-lg-hue-sync
#!/bin/sh
if [ -x /var/home/root/lg-hue-sync/provision_luna.sh ]; then
    /var/home/root/lg-hue-sync/provision_luna.sh local || true
fi
mkdir -p /run/systemd/system
cp -f /var/home/root/lg-hue-sync/lg-hue-sync.service /run/systemd/system/lg-hue-sync.service
systemctl daemon-reload
systemctl stop lg-hue-sync 2>/dev/null || true
sleep 2
systemctl start lg-hue-sync
EOF
chmod +x /var/lib/webosbrew/init.d/50-lg-hue-sync

# Launch now
/var/lib/webosbrew/init.d/50-lg-hue-sync
echo "[TV] Service installed and started via systemd (/var/lib/webosbrew/init.d/50-lg-hue-sync)"
REMOTEEOC

echo ""
echo "==================================================================="
echo "[+] SUCCESS: lg-hue-sync deployed and active on TV!"
echo "==================================================================="
echo "Management commands on TV (via SSH):"
echo "  Status:  ssh -p $SSH_PORT root@$TV_IP 'systemctl status lg-hue-sync'"
echo "  Logs:    ssh -p $SSH_PORT root@$TV_IP 'journalctl -u lg-hue-sync -f'"
echo "  Restart: ssh -p $SSH_PORT root@$TV_IP 'systemctl restart lg-hue-sync'"
echo "  Stop:    ssh -p $SSH_PORT root@$TV_IP 'systemctl stop lg-hue-sync'"
echo "==================================================================="
