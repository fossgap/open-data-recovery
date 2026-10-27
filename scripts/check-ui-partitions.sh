#!/bin/sh
# TestDisk and real GTK flow on regular disposable images only.
# Set ODR_TEST_STORAGE=/tmp to exercise the same-drive image exception.
set -eu
cd "$(dirname "$0")/.."
if [ -z "${ODR_BINARY:-}" ]; then cargo build --locked; fi
binary=$(realpath "${ODR_BINARY:-target/debug/open-data-recovery}")
source_root=$(mktemp -d /tmp/odr-partition-ui-source.XXXXXX)
work_root=$(mktemp -d "${ODR_TEST_STORAGE:-/dev/shm}/odr-partition-ui-work.XXXXXX")
trap 'rm -rf "$source_root" "$work_root"' EXIT HUP INT TERM
truncate -s 16M "$source_root/filesystem.img"
mkfs.ext2 -q -F -L LostFiles "$source_root/filesystem.img"
printf 'Verified recovery through partition discovery.\n' > "$source_root/original.txt"
debugfs -w -R "write $source_root/original.txt recovered.txt" "$source_root/filesystem.img"
debugfs -w -R "write $source_root/original.txt kept.txt" "$source_root/filesystem.img"
debugfs -w -R 'rm recovered.txt' "$source_root/filesystem.img"
python3 - "$source_root" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
with (root / 'disk.img').open('wb') as disk:
    disk.truncate(32 * 1024 * 1024)
    disk.seek(510)
    disk.write(b'\x55\xaa')
    disk.seek(2 * 1024 * 1024)
    disk.write((root / 'filesystem.img').read_bytes())
PY
source_hash=$(sha256sum "$source_root/disk.img" | cut -d ' ' -f 1)
# Exercise the CLI against the same real, disposable lost-partition fixture.
"$binary" --discover-partitions "$source_root/disk.img" --workspace "$work_root/cli-scans" --table intel > "$work_root/discovery.json"
report=$(python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); assert r["complete"]; print(r["directory"] + "/report.json")' "$work_root/discovery.json")
number=$(python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); print(next(i+1 for i,p in enumerate(r["partitions"]) if p["offset_bytes"] == 2097152 and p["length_bytes"] == 16777216 and p["verified"]))' "$work_root/discovery.json")
for invalid in 0 999; do
    if "$binary" --scan-partition "$report" --partition "$invalid" --workspace "$work_root/invalid" > "$work_root/invalid.log" 2>&1; then
        echo "Invalid partition number was accepted" >&2
        exit 1
    fi
done
test ! -e "$work_root/invalid"
if "$binary" --discover-partitions "$source_root/disk.img" --workspace "$work_root/invalid" --table 'intel,write' > "$work_root/invalid.log" 2>&1; then
    echo "Invalid partition table was accepted" >&2
    exit 1
fi
test ! -e "$work_root/invalid"
"$binary" --scan-partition "$report" --partition "$number" --workspace "$work_root/cli-scans" --mode quick --include-existing --extensions txt > "$work_root/cli-scan.log"
cli_session=$(sed -n 's/^SAVED=//p' "$work_root/cli-scan.log")
test -n "$cli_session"
"$binary" --list "$cli_session" > "$work_root/candidates.jsonl"
python3 - "$work_root/candidates.jsonl" <<'PY'
import json, sys
files = {f['path']: f for f in map(json.loads, open(sys.argv[1]))}
assert files['recovered.txt']['deleted']
assert not files['kept.txt']['deleted']
PY
"$binary" --recover "$cli_session" --all --destination "$work_root/cli-recovered"
cmp "$source_root/original.txt" "$work_root/cli-recovered/recovered.txt"
cmp "$source_root/original.txt" "$work_root/cli-recovered/kept.txt"
truncate -s 8M "$source_root/empty.img"
"$binary" --discover-partitions "$source_root/empty.img" --workspace "$work_root/cli-scans" --table intel > "$work_root/empty.json"
python3 -c 'import json,sys; r=json.load(open(sys.argv[1])); assert r["complete"] and not r["partitions"]' "$work_root/empty.json"
echo "CLI partition discovery passed: search, report scan, verified recovery, empty search and invalid options."
"$binary" --scan "$source_root/filesystem.img" --workspace "$work_root/scans" --mode quick --extensions txt > "$work_root/scan.log"
session=$(sed -n 's/^SAVED=//p' "$work_root/scan.log")
test -n "$session"
env ODR_SMOKE_DISCOVERY_SOURCE="$source_root/disk.img" ODR_SMOKE_DISCOVERY_WORKSPACE="$work_root/scans" \
    ODR_SMOKE_SESSION="$session" ODR_SMOKE_DESTINATION="$work_root/recovered" \
    GTK_IM_MODULE=simple GTK_A11Y=none G_DEBUG=fatal-criticals GDK_BACKEND=x11 GSK_RENDERER=cairo \
    xvfb-run -a "$binary" --live-smoke-test
cmp "$source_root/original.txt" "$work_root/recovered/recovered.txt"
cmp "$source_root/original.txt" "$work_root/recovered/kept.txt"
test "$source_hash" = "$(sha256sum "$source_root/disk.img" | cut -d ' ' -f 1)"
