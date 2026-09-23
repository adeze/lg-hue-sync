#!/usr/bin/env bash
set -euo pipefail

TV_IP="${1:?Usage: ./scripts/uninstall.sh <tv-ip> [ssh-port] [--purge-config]}"
SSH_PORT="${2:-22}"
PURGE_CONFIG=false
for option in "${@:3}"; do
    case "$option" in
        --purge-config) PURGE_CONFIG=true ;;
        *) echo "Unknown option: $option" >&2; exit 2 ;;
    esac
done

SSH_OPTS=(-p "$SSH_PORT" -o StrictHostKeyChecking=accept-new -o ConnectTimeout=5)
purge_value=false
$PURGE_CONFIG && purge_value=true

ssh "${SSH_OPTS[@]}" root@"$TV_IP" "PURGE_CONFIG=$purge_value sh -s" <<'REMOTE'
set -eu
install_dir=/var/home/root/lg-hue-sync
backup_dir="/var/home/root/lg-hue-sync-backup-$(date +%Y%m%d-%H%M%S)"

systemctl stop lg-hue-sync 2>/dev/null || true
systemctl disable lg-hue-sync 2>/dev/null || true
rm -f /run/systemd/system/lg-hue-sync.service /etc/systemd/system/lg-hue-sync.service
rm -f /var/lib/webosbrew/init.d/50-lg-hue-sync

for root in /var/luna-service2 /var/luna-service2-dev; do
    rm -f "$root/manifests.d/org.webosbrew.lg-hue-sync.json"
    rm -f "$root/roles.d/org.webosbrew.lg-hue-sync.json"
    rm -f "$root/client-permissions.d/org.webosbrew.lg-hue-sync.json"
done
ls-control scan-services 2>/dev/null || true

luna-send -n 1 -f luna://com.webos.appInstallService/dev/remove \
    '{"id":"org.webosbrew.lg-hue-sync","subscribe":false}' >/dev/null 2>&1 || true

if [ "$PURGE_CONFIG" = false ] && [ -f "$install_dir/config.json" ]; then
    mkdir -p "$backup_dir"
    cp -p "$install_dir/config.json" "$backup_dir/config.json"
    echo "Configuration preserved at $backup_dir/config.json"
fi
rm -rf "$install_dir"
systemctl daemon-reload 2>/dev/null || true
echo "lg-hue-sync removed"
REMOTE
