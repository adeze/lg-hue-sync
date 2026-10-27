#!/bin/sh
# Run over existing root SSH with `sh -s -- --trial`; default only prints the plan.
set -eu

case "${1:---plan}" in
    --plan)
        printf '%s\n' 'Preview only: temporarily mask and stop remotelogger.service.' \
            'A systemd timer must be active first; it unmasks and starts the service after two minutes.' \
            'No boot hook, vendor file, consent, networking, Alexa or sportsalarm changes.' \
            'Use --trial explicitly on the audited webOS 6.5.3 TV. Re-run the audit after restoration.'
        exit 0 ;;
    --trial) ;;
    *) echo 'Usage: sh c1-debloat-trial.sh [--plan|--trial]' >&2; exit 2 ;;
esac
[ "$#" -eq 1 ] || { echo 'Unexpected arguments' >&2; exit 2; }
fail() { echo "$*" >&2; exit 1; }
[ "$(id -u)" = 0 ] || fail 'Root SSH is required.'
grep -qx 'VERSION="6.5.3"' /etc/webos-release || fail 'Firmware differs from the audited webOS 6.5.3.'
unit=remotelogger.service
timer=c1-debloat-remotelogger-rollback
property() { systemctl show "$unit" --property="$1" --value; }
[ "$(property LoadState)" = loaded ] || fail 'Service must be loaded and unmasked.'
[ "$(property ActiveState)" = active ] || fail 'Service must already be active.'
[ "$(property UnitFileState)" = static ] || fail 'Service enablement differs from the audit.'
[ "$(property FragmentPath)" = /lib/systemd/system/remotelogger.service ] || fail 'Service override differs from the audit.'
[ -z "$(property RequiredBy)$(property BoundBy)$(property ConsistsOf)" ] || fail 'New hard reverse dependencies require another audit.'
for suffix in timer service; do
    state=$(systemctl show "$timer.$suffix" --property=LoadState --value 2>/dev/null || true)
    [ "$state" = not-found ] || fail 'An earlier rollback unit exists; inspect it before another trial.'
done
rollback='systemctl unmask --runtime remotelogger.service && systemctl start remotelogger.service'
# The independent timer survives loss of the SSH connection.
systemd-run --unit="$timer" --on-active=2m --timer-property=AccuracySec=1s \
    /bin/sh -c "$rollback" || fail 'Could not schedule restoration; service unchanged.'
systemctl is-active --quiet "$timer.timer" || fail 'Rollback timer is not active; service unchanged.'
if ! systemctl mask --runtime "$unit"; then
    fail 'Mask failed; no stop attempted. Restoration timer remains scheduled.'
fi
if ! systemctl stop "$unit"; then
    /bin/sh -c "$rollback" || true
    fail 'Stop failed; immediate restoration attempted and timer remains scheduled.'
fi
state=$(property ActiveState)
if [ "$state" != inactive ]; then
    /bin/sh -c "$rollback" || true
    fail 'Service did not become inactive; restoration attempted and timer remains scheduled.'
fi
printf '%s\n' 'remotelogger is temporarily inactive. Automatic restoration is scheduled in two minutes.' \
    'Confirm the timer completes and the service becomes active; do not infer success from this message.'
