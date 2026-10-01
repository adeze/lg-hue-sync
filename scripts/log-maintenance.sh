#!/bin/sh
set -eu

if [ "${1:-}" = --install ]; then
    install_dir=/var/home/root/lg-hue-sync
    cat > "$install_dir/lg-hue-sync-log.service" <<'UNIT'
[Unit]
Description=Clear LG sync log at 1 MiB
ConditionPathIsDirectory=/var/home/root/lg-hue-sync

[Service]
Type=oneshot
ExecStart=/var/home/root/lg-hue-sync/log-maintenance.sh
UNIT
    cat > "$install_dir/lg-hue-sync-log.timer" <<'UNIT'
[Unit]
Description=Check LG sync log size every minute

[Timer]
OnBootSec=1min
OnUnitActiveSec=1min
AccuracySec=5s
UNIT
    cat > /var/lib/webosbrew/init.d/55-lg-hue-sync-log <<'BOOT'
#!/bin/sh
set -eu
install_dir=/var/home/root/lg-hue-sync
mkdir -p /run/systemd/system
cp "$install_dir/lg-hue-sync-log.service" /run/systemd/system/
cp "$install_dir/lg-hue-sync-log.timer" /run/systemd/system/
systemctl daemon-reload
systemctl start lg-hue-sync-log.timer
BOOT
    chmod 755 /var/lib/webosbrew/init.d/55-lg-hue-sync-log
    /var/lib/webosbrew/init.d/55-lg-hue-sync-log
    exit 0
fi

[ "$#" -le 1 ] || { echo "Usage: $0 [log-file | --install]" >&2; exit 2; }
log_file=${1:-/var/home/root/lg-hue-sync/daemon.log}
if [ -L "$log_file" ] || { [ -e "$log_file" ] && [ ! -f "$log_file" ]; }; then
    echo "Refusing to clear a symlink or non-regular log: $log_file" >&2
    exit 1
fi
[ -f "$log_file" ] || exit 0
size=$(stat -c %s "$log_file" 2>/dev/null || stat -f %z "$log_file")
if [ "$size" -ge 1048576 ]; then
    # Preserve the inode: the daemon's O_APPEND descriptors keep writing here.
    # ponytail: minute polling permits temporary overshoot; use a writer cap if needed.
    : > "$log_file"
fi
