#!/bin/sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM
log_file=$test_dir/daemon.log

# Missing logs stay missing; small logs retain their contents.
sh "$script_dir/log-maintenance.sh" "$log_file"
[ ! -e "$log_file" ]
printf 'keep me\n' > "$log_file"
sh "$script_dir/log-maintenance.sh" "$log_file"
[ "$(cat "$log_file")" = 'keep me' ]

# The exact boundary clears in place, including an already-open append writer.
dd if=/dev/zero of="$log_file" bs=1024 count=1024 2>/dev/null
inode=$(ls -i "$log_file" | awk '{print $1}')
exec 3>> "$log_file"
sh "$script_dir/log-maintenance.sh" "$log_file"
[ ! -s "$log_file" ]
[ "$(ls -i "$log_file" | awk '{print $1}')" = "$inode" ]
printf 'after reset\n' >&3
exec 3>&-
[ "$(cat "$log_file")" = 'after reset' ]

# Oversized logs clear; symlinks and directories never become truncation targets.
dd if=/dev/zero of="$log_file" bs=1024 count=1025 2>/dev/null
ln -s "$log_file" "$test_dir/link"
if sh "$script_dir/log-maintenance.sh" "$test_dir/link" 2>/dev/null; then exit 1; fi
[ -s "$log_file" ]
if sh "$script_dir/log-maintenance.sh" "$test_dir" 2>/dev/null; then exit 1; fi
sh "$script_dir/log-maintenance.sh" "$log_file"
[ ! -s "$log_file" ]
echo 'Log maintenance checks passed'
