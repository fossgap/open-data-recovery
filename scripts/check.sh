#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-}" = "--inside" ]; then
    for file in ui/*.ui; do
        gtk4-builder-tool validate "$file"
    done
    G_DEBUG=fatal-criticals target/debug/open-data-recovery --smoke-test
    G_DEBUG=fatal-criticals target/debug/open-data-recovery --live-smoke-test
else
    cargo test --locked
    cargo build --locked
    # Keep a parent shell: Xvfb does not send its ready signal to PID 1 in Docker.
    env GTK_IM_MODULE=simple GDK_BACKEND=x11 GSK_RENDERER=cairo xvfb-run -a sh scripts/check.sh --inside
fi
