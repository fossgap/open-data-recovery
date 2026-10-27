#!/bin/sh
# Capture only this application's window in a private virtual display.
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-}" != "--inside" ]; then
    cargo build --locked
    env GTK_IM_MODULE=simple GDK_BACKEND=x11 GSK_RENDERER=cairo xvfb-run -a -s '-screen 0 1600x1100x24' sh scripts/capture-preview.sh --inside
    exit 0
fi
mkdir -p docs/screenshots
log=$(mktemp /tmp/odr-preview.XXXXXX)
target/debug/open-data-recovery --demo >"$log" 2>&1 &
app_pid=$!
trap 'kill "$app_pid" 2>/dev/null || true; cat "$log"; rm -f "$log"' EXIT HUP INT TERM
window=$(timeout 10s xdotool search --sync --onlyvisible --all --pid "$app_pid" --name 'Open Data Recovery' | head -n 1)
test -n "$window"
sleep 1
import -window "$window" docs/screenshots/workspace.png
xdotool windowsize "$window" 1060 700
xdotool windowfocus --sync "$window"
xdotool key --clearmodifiers ctrl+l
sleep 1
import -window "$window" docs/screenshots/compact.png
xdotool windowsize "$window" 1380 920
xdotool key --clearmodifiers ctrl+l
sleep 1
xdotool windowfocus --sync "$window"
xdotool key --clearmodifiers ctrl+r
sleep 2
xdotool mousemove --window "$window" 740 196 click 1
sleep 1
import -window "$window" docs/screenshots/scan-progress.png
xdotool mousemove --window "$window" 803 196 click 1
xdotool mousemove --window "$window" 1280 95 click 1
sleep 1
xdotool key --clearmodifiers ctrl+f
xdotool type --clearmodifiers 'Alpine'
sleep 1
xdotool mousemove --window "$window" 500 425 click 1
sleep 1
import -window "$window" docs/screenshots/image-preview.png
xdotool key --clearmodifiers ctrl+f ctrl+a BackSpace
sleep 1
xdotool mousemove --window "$window" 325 355 click 1
sleep 1
xdotool mousemove --window "$window" 1280 847 click 1
dialog=$(timeout 10s xdotool search --sync --onlyvisible --all --pid "$app_pid" --name '^Review recovery$' | head -n 1)
test -n "$dialog"
sleep 1
import -window "$dialog" docs/screenshots/recovery.png
printf 'Saved workspace, compact, scan-progress, image-preview, and recovery screenshots in docs/screenshots/\n'

xdotool key --clearmodifiers Escape
xdotool windowfocus --sync "$window"
xdotool key --clearmodifiers ctrl+shift+r
dialog=$(timeout 10s xdotool search --sync --onlyvisible --all --pid "$app_pid" --name '^Custom scan$' | head -n 1)
sleep 1
import -window "$dialog" docs/screenshots/custom-scan.png
xdotool key --clearmodifiers Escape
xdotool windowfocus --sync "$window"
xdotool key --clearmodifiers ctrl+shift+s
dialog=$(timeout 10s xdotool search --sync --onlyvisible --all --pid "$app_pid" --name '^Saved scans$' | head -n 1)
sleep 1
import -window "$dialog" docs/screenshots/saved-scans.png
