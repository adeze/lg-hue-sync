#!/usr/bin/env python3
"""Read-only webOS debloat inventory over existing root SSH; report to stdout."""

import argparse
import re
import subprocess
import sys


AUDIT = r'''
export LC_ALL=C SYSTEMD_PAGER=cat SYSTEMD_COLORS=0
section() { printf '\n## %s\n' "$1"; }
section 'Firmware and kernel (no device identifiers)'
uname -srm
for f in /etc/webos-release /etc/os-release; do
    [ ! -f "$f" ] || grep -E '^(ID|VERSION|VERSION_ID|PRETTY_NAME|WEBOS_RELEASE|WEBOS_BUILD_ID|WEBOS_BUILD_NAME)=' "$f"
done
section 'Resource baseline'
uptime
grep -E '^(MemTotal|MemFree|MemAvailable|Buffers|Cached|SwapTotal|SwapFree):' /proc/meminfo
df -k / /var /tmp 2>/dev/null
section 'Candidate service state; absent units are not compatibility evidence'
if command -v systemctl >/dev/null 2>&1; then
    for s in remotelogger service-logger contentminer nudge rdxd dmost acr2 admanager sportsalarm adoverlay-service uploadd iconnectivity amazon-alexa-adapter lg-hue-sync; do
        printf '\n[%s]\n' "$s"
        systemctl show "$s.service" --no-pager -p LoadState -p ActiveState -p SubState -p UnitFileState -p FragmentPath -p MainPID -p Restart -p RestartUSec -p Requires -p Requisite -p Wants -p BindsTo -p PartOf -p RequiredBy -p WantedBy -p BoundBy -p ConsistsOf -p Before -p After -p Triggers -p TriggeredBy -p RefuseManualStop 2>&1
    done
else
    printf 'systemctl unavailable\n'
fi
section 'Candidate executable presence and bind mounts'
for b in acr2 admanager sportsalarm adoverlay-service uploadd iconnectivity amazon-alexa-adapter; do
    if [ -e "/usr/sbin/$b" ]; then
        ls -ld "/usr/sbin/$b"
    fi
done
awk '$2 ~ /^\/usr\/sbin\/(acr2|admanager|sportsalarm|adoverlay-service|uploadd|iconnectivity|amazon-alexa-adapter)$/ || $2 == "/etc/hosts" || $2 == "/etc/xinetd.d/ssg_https" {print $2, $3, $4}' /proc/mounts
section 'Process names and memory only (no arguments)'
printf 'PID PPID COMMAND RSS_KiB CPU_percent_since_start\n'
ps -eo pid,ppid,comm,rss,pcpu 2>/dev/null | awk 'NR > 1 && $4 > 0' | sort -k4,4nr | head -n 60
section 'Startup hook names and hashes (no contents)'
for d in /var/lib/webosbrew/init.d /var/lib/webosbrew /home/root/debloat; do
    [ -d "$d" ] || continue
    printf '\n[%s]\n' "$d"
    for f in "$d"/*; do
        [ -f "$f" ] || continue
        case "$f" in
            *.orig|*.json|*.pem|*.key) continue ;;
        esac
        if command -v sha256sum >/dev/null 2>&1; then
            sha256sum "$f"
        else
            printf '%s\n' "$f"
        fi
    done
done
section 'LG-related hosts overrides (addresses omitted)'
awk '!/^[[:space:]]*#/ && NF > 1 {for (i=2;i<=NF;i++) {if ($i ~ /^#/) break; if ($i ~ /(^|\.)(lge\.com|lgtvcommon\.com|lgsmartad\.com|lgappstv\.com)$/) print $i}}' /etc/hosts
section 'Phone remote configuration (selected keys only)'
if [ -f /etc/xinetd.d/ssg_https ]; then
    grep -E '^[[:space:]]*(port|disable|only_from)[[:space:]]*=' /etc/xinetd.d/ssg_https | sed 's/\(only_from[[:space:]]*=\).*/\1 [configured; address omitted]/'
fi
section 'Luna activation records for candidate binaries (selected fields only)'
for d in /usr/share/luna-service2/services.d /usr/share/luna-service2/services /usr/share/luna-service2/system-services /var/luna-service2/services.d /var/luna-service2/services /mnt/otycabi/usr/share/luna-service2/services.d /mnt/otncabi/usr/share/luna-service2/services.d; do
    [ -d "$d" ] || continue
    for f in "$d"/*; do
        [ -f "$f" ] || continue
        if grep -Eq '/(remotelogger|contentminer|nudge|rdxd|dmost|acr2|admanager|sportsalarm|adoverlay-service|uploadd|iconnectivity|amazon-alexa-adapter)([[:space:]]|$)' "$f"; then
            printf '\n[%s]\n' "$f"
            awk -F= '/^[[:space:]]*(Name|Type)[[:space:]]*=/ {print} /^[[:space:]]*Exec[[:space:]]*=/ {split($2,a," "); print "Executable=" a[1]}' "$f"
        fi
    done
done
section 'Candidate executable owners and memory (no command arguments)'
for p in /proc/[0-9]*; do
    [ -r "$p/comm" ] || continue
    name=$(cat "$p/comm" 2>/dev/null)
    case "$name" in
        remotelogger|service-logger|contentminer|nudge|rdxd|dmost|acr2|admanager|sportsalarm|adoverlay-servic*|uploadd|iconnectivity|amazon-alexa-ad*)
            printf '\nPID=%s Name=%s Executable=' "${p##*/}" "$name"
            readlink "$p/exe" 2>/dev/null
            grep -E '^(PPid|Uid|VmRSS|Threads):' "$p/status" 2>/dev/null
            ;;
    esac
done
section 'Recovery and root filesystem state'
command -v systemctl
command -v sshd || command -v dropbear
awk '$2 == "/" || $2 == "/var" {print $2, $3, $4}' /proc/mounts
for f in /var/lib/webosbrew/startup.sh /var/lib/webosbrew/init.d/*; do
    [ -f "$f" ] || continue
    printf '\n[%s]\n' "$f"
    for marker in nsu.lge.com snu.lge.com init.d telnet dropbear sshd; do
        if grep -Fq "$marker" "$f"; then printf 'Contains marker: %s\n' "$marker"; fi
    done
done
section 'CPU counters: two samples five seconds apart'
grep '^cpu ' /proc/stat
sleep 5
grep '^cpu ' /proc/stat
section 'End: inventory only; no compatibility conclusions or changes applied'
'''


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host", help="TV address or existing SSH alias")
    parser.add_argument("--port", type=int, default=22)
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:-]*", args.host):
        parser.error("host must be an address or SSH alias, without options or user names")
    if not 1 <= args.port <= 65535:
        parser.error("port must be between 1 and 65535")
    command = [
        "ssh", "-T", "-p", str(args.port),
        "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
        "-o", "ConnectTimeout=5", "-o", "ServerAliveInterval=5",
        "-o", "ServerAliveCountMax=2", f"root@{args.host}", "sh -s",
    ]
    try:
        result = subprocess.run(command, input=AUDIT, text=True, timeout=60, check=False)
        return result.returncode
    except subprocess.TimeoutExpired:
        print("Audit timed out after 60 seconds; report may be incomplete.", file=sys.stderr)
        return 124
    except OSError as error:
        print(f"Cannot run SSH: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
