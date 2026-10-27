#!/bin/sh
# Capture production UI with a disposable image and isolated settings/display.
set -eu
cd "$(dirname "$0")/.."
if [ -z "${ODR_BINARY:-}" ]; then cargo build --locked; fi
binary=$(realpath "${ODR_BINARY:-target/debug/open-data-recovery}")
env GTK_IM_MODULE=simple GDK_BACKEND=x11 GSK_RENDERER=cairo GDK_SCALE=1 GDK_DPI_SCALE=1 \
    xvfb-run -a -s '-screen 0 1600x1100x24' /usr/bin/python3 scripts/capture-screenshots.py --binary "$binary" "$@"
