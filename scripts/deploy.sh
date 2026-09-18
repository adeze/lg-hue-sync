#!/usr/bin/env bash
set -euo pipefail

# deploy.sh
# Cross-compiles the native Rust daemon for LG webOS 32-bit ARM (Alpha 9 Gen 4)
# and deploys it over SSH to the LG C1 TV as a headless systemd service.

TV_IP="${1:-192.168.1.149}"
SSH_PORT="${2:-22}"
TARGET="armv7-unknown-linux-gnueabihf"
REMOTE_DIR="/var/home/root/lg-hue-sync"

echo "==================================================================="
echo "       LG C1 Native Rust Hue Sync Deployment ($TARGET)            "
echo "==================================================================="

# 1. Check cross or cargo
if command -v cross &> /dev/null; then
    echo "[*] Compiling release binary with 'cross' for $TARGET..."
    cross build --target "$TARGET" --release
elif rustup target list | grep -q "$TARGET (installed)"; then
    echo "[*] Compiling release binary with native 'cargo' for $TARGET..."
    cargo build --target "$TARGET" --release
else
    echo "[-] Error: Cross-compilation target not ready."
    echo "    Option A: Install cross (uses Docker): cargo install cross"
    echo "    Option B: Install toolchain: rustup target add $TARGET"
    exit 1
fi

BINARY="target/$TARGET/release/lg-hue-sync"
if [ ! -f "$BINARY" ]; then
    echo "[-] Error: Compiled binary not found at $BINARY"
    exit 1
fi

echo "[+] Built binary size: $(du -h "$BINARY" | cut -f1)"

# 2. Deploy over SSH
SSH_OPTS="-p $SSH_PORT -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
echo "[*] Connecting to root@$TV_IP:$SSH_PORT..."

ssh $SSH_OPTS root@"$TV_IP" "mkdir -p $REMOTE_DIR"

echo "[*] Uploading binary to TV..."
scp $SSH_OPTS "$BINARY" root@"$TV_IP":"$REMOTE_DIR/lg-hue-sync"
ssh $SSH_OPTS root@"$TV_IP" "chmod +x $REMOTE_DIR/lg-hue-sync"

if [ -f "config.json" ]; then
    echo "[*] Uploading local config.json..."
    scp $SSH_OPTS config.json root@"$TV_IP":"$REMOTE_DIR/config.json"
fi

# 3. Create systemd autostart unit on webOS
echo "[*] Installing autostart service on TV..."
ssh $SSH_OPTS root@"$TV_IP" bash << 'REMOTEEOC'
cat << 'EOF' > /etc/systemd/system/lg-hue-sync.service
[Unit]
Description=LG C1 Native Philips Hue Sync Daemon
After=network.target
Conflicts=sleep.target suspend.target
StopWhenUnneeded=yes

[Service]
Type=notify
NotifyAccess=all
WatchdogSec=10
WorkingDirectory=/var/home/root/lg-hue-sync
ExecStart=/var/home/root/lg-hue-sync/lg-hue-sync run --config /var/home/root/lg-hue-sync/config.json
Restart=always
RestartSec=5
TimeoutStopSec=3

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload || true
systemctl enable lg-hue-sync || true
systemctl restart lg-hue-sync || true
echo "[TV] Service installed and started at /etc/systemd/system/lg-hue-sync.service"
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
