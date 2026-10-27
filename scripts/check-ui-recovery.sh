#!/bin/sh
# Real production UI flow using a disposable image and separate temporary storage.
set -eu
cd "$(dirname "$0")/.."
if [ -z "${ODR_BINARY:-}" ]; then cargo build --locked; fi
binary=$(realpath "${ODR_BINARY:-target/debug/open-data-recovery}")
source_root=$(mktemp -d /tmp/odr-ui-source.XXXXXX)
work_root=$(mktemp -d "${ODR_TEST_STORAGE:-/dev/shm}/odr-ui-work.XXXXXX")
trap 'rm -rf "$source_root" "$work_root"' EXIT HUP INT TERM
truncate -s 16M "$source_root/source.img"
mkfs.ext2 -q -F "$source_root/source.img"
printf 'Verified recovery through the GTK dialog.\n' > "$source_root/original.txt"
debugfs -w -R "write $source_root/original.txt recovered.txt" "$source_root/source.img"
if [ "${ODR_CONSOLIDATE:-0}" = 1 ]; then
    debugfs -w -R "write $source_root/original.txt alias.txt" "$source_root/source.img"
fi
debugfs -w -R 'rm recovered.txt' "$source_root/source.img"
set -- --scan "$source_root/source.img" --workspace "$work_root/scans" --mode quick --clone --extensions txt
if [ "${ODR_CONSOLIDATE:-0}" = 1 ]; then
    debugfs -w -R 'rm alias.txt' "$source_root/source.img"
    set -- "$@" --consolidate
    export ODR_SMOKE_CONSOLIDATION=1 ODR_SMOKE_ALL_PATHS=1
fi
if [ "${ODR_MANAGE_SCANS:-0}" = 1 ]; then
    cp "$source_root/source.img" "$source_root/other.img"
    export ODR_SMOKE_OTHER_SOURCE="$source_root/other.img"
fi
source_hash=$(sha256sum "$source_root/source.img" | cut -d ' ' -f 1)
"$binary" "$@" > "$work_root/scan.log"
session=$(sed -n 's/^SAVED=//p' "$work_root/scan.log")
test -n "$session"
env ODR_SMOKE_SESSION="$session" ODR_SMOKE_DESTINATION="$work_root/recovered" GTK_IM_MODULE=simple GTK_A11Y=none G_DEBUG=fatal-criticals GDK_BACKEND=x11 GSK_RENDERER=cairo xvfb-run -a "$binary" --live-smoke-test
cmp "$source_root/original.txt" "$work_root/recovered/recovered.txt"
if [ "${ODR_CONSOLIDATE:-0}" = 1 ]; then
    cmp "$source_root/original.txt" "$work_root/recovered/alias.txt"
fi
if [ "${ODR_MANAGE_SCANS:-0}" = 1 ]; then
    test ! -e "$session"
    test "$source_hash" = "$(sha256sum "$source_root/source.img" | cut -d ' ' -f 1)"
fi
