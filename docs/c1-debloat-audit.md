# C1 debloat audit — 2026-10-26

Read-only SSH inspection of webOS TV 6.5.3. No services, settings, consent stores,
mounts, firewall rules or boot hooks were changed. No firmware or application
files were removed. Local reports remain under `/tmp/lg-tv-debloat-detailed.*`;
they are diagnostic snapshots, not durable backups.

## Findings and proposed scope

| Component | Observed activation/dependency | Decision |
| --- | --- | --- |
| `remotelogger` | Active systemd unit from `/lib`; boot target wants it; no reported reverse `Requires`/`BindsTo` | First temporary trial |
| `contentminer` | Active systemd unit from `/lib`; static Luna registration; no reported reverse `Requires`/`BindsTo` | Trial separately after remotelogger |
| `nudge` | Active systemd unit from `/lib`; static Luna registration also names `com.webos.service.nudgeSync`; no reported reverse `Requires`/`BindsTo` | Trial separately after contentminer |
| `rdxd` | Active; `crashreportd.service` requires it | Exclude from initial trial |
| `dmost`, `uploadd` | Active units from `/etc`; uploadd also has dynamic Luna activation | Defer; runtime masking has different precedence and activation concerns |
| `sportsalarm` | Running under ls-hubd; dynamic Luna registration executes `/usr/sbin/sportsalarm` | User does not need it; separate reversible activation-block trial required |
| Alexa adapter | Dynamic registration uses `jailer`; running executable is inside `/var/palm/jail/amazon.alexa.adapter`; separate VSK registration also exists | User does not use Alexa; audit covers both activation names, but blocking only the host adapter path is insufficient evidence of disabling Alexa |
| ACR, admanager | Running under ls-hubd; executable files exist; direct registrations not found in inspected service directories | Defer until actual activation route is identified |
| Phone connectivity, AirPlay, HomeKit | Connectivity and integration processes observed | Preserve |

The three initial systemd candidates all use `Restart=on-failure` (100 ms), allow
manual stop, and have no observed reload/namespace coupling. Their combined RSS
was approximately 8.5 MiB. RSS includes shared pages; this is not a prediction of
recoverable memory. No performance improvement has been measured.

The TV had approximately 716–722 MiB available RAM. The root filesystem is
read-only squashfs; its reported 100% usage does not establish storage pressure.
`/var` had approximately 303 MiB available. CPU counters over one five-second
sample indicate approximately 35% non-idle time across all cores, including the
audit itself. Process CPU percentages in the report are lifetime averages.

Pre-existing failed units: `accountmanager`, `bugreportd`, `crashreportd` and the
system `dropbear.service`. An existing Homebrew Channel dropbear connection
nevertheless provided working root SSH. Do not restart SSH or ls-hubd. Only Hue
Sync hooks were found in `/var/lib/webosbrew/init.d`; other startup mechanisms
are not ruled out. The startup script contains an update-block hostname marker;
that alone does not establish effective update protection. No LG-domain hosts
overrides or selected executable bind blocks appeared in the audit.

## Proposed temporary trial — not executed

`scripts/c1-debloat-trial.sh` implements only the first remotelogger trial.
Its default/`--plan` mode performs no device operations. Explicit `--trial`
requires the audited firmware and service state, rejects changed hard reverse
dependencies and existing rollback units, and schedules an independent systemd
restoration timer before masking or stopping. The timer attempts unmask/start
after two minutes even if SSH disconnects. Scheduling and checking the timer are
not proof that restoration succeeds: verify service state afterwards. Host
fixtures cover ordering, refusal, timer failure and immediate restoration after
a failed stop; actual TV mutation/restoration has not been run.

The tool deliberately has no persistent mode or Alexa/sportsalarm blocking yet.
Use this timed tool for the first trial rather than the untimed manual sequence
below, which documents the underlying operations and immediate recovery.

Run one unit at a time through the existing root SSH connection. Start with
`remotelogger.service`. Confirm it remains loaded/active, uses the recorded
`/lib/systemd/system/` fragment, and has no existing runtime or persistent mask
before creating a new runtime mask. Stop if the current state differs from the
audit. Do not overwrite another change's mask.

```sh
systemctl show remotelogger.service --no-pager -p LoadState -p ActiveState -p FragmentPath -p UnitFileState
systemctl mask --runtime remotelogger.service
systemctl stop remotelogger.service
systemctl show remotelogger.service --no-pager -p LoadState -p ActiveState -p SubState
```

If masking fails, do not proceed to stop the unit. If stopping fails after the
new mask was created, remove that mask and investigate. A runtime mask is a
temporary file under `/run`; it does not delete the vendor unit. It also does
not prove that all clients tolerate the missing service.

Immediate rollback for this previously active unit:

```sh
systemctl unmask --runtime remotelogger.service
systemctl start remotelogger.service
systemctl show remotelogger.service --no-pager -p LoadState -p ActiveState -p SubState
```

Require successful unmask before start. A full reboot normally clears `/run`
masks; standby/wake is not a substitute for reboot. Do not install a persistent
hook during the first trial. Repeat this pattern with `contentminer.service`,
then `nudge.service`, only after each preceding trial passes.

For each trial: rerun the audit, compare failed units against the baseline,
check repeated activation/error behaviour, test LG Home, app store, streaming,
HDMI/audio, remote, phone integrations and Hue Sync, then test standby/wake.
Use comparable foreground workloads and a settled TV for performance comparison.
Revert on regressions; retain no persistent change until the behavioural checks
and actual rollback have passed.

Alexa and sportsalarm require a separate trial that accounts for Luna dynamic
activation, the existing Alexa jail and both Alexa adapter/VSK entry points.
Do not kill ls-hubd, replace jailer, change global permissions, or assume a
no-op executable avoids activation retries. Their disable/restore procedure is
not yet hardware-qualified. No consent-file edits, broad DNS lists, update-policy
changes or system-file deletions are part of this first trial.

## Repeat the inventory

```sh
uv run python scripts/audit_tv_debloat.py <existing-tv-address-or-ssh-alias> --port 22
uv run python scripts/test_audit_tv_debloat.py
```

The script requires an already trusted SSH host key and existing noninteractive
root authentication. It streams POSIX shell over SSH without installing a TV
script, limits the connection/run duration, omits process arguments and credentials,
and tolerates absent vendor tools/units. Missing output is not evidence that a
feature is absent. Root SSH access is verified; recovery after an intentional
debloat change remains untested.
