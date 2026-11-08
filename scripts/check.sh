#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
: "${ODR_BINARY:=target/debug/open-data-recovery}"
export ODR_BINARY
if [ "${1:-}" = "--inside" ]; then
    for file in ui/*.ui; do
        gtk4-builder-tool validate "$file"
    done
    ODR_LANGUAGE=en G_DEBUG=fatal-criticals "$ODR_BINARY" --smoke-test
    ODR_LANGUAGE=en G_DEBUG=fatal-criticals "$ODR_BINARY" --live-smoke-test
    GTK_A11Y=none sh scripts/check-ui-themes.sh --inside
    sh scripts/check-i18n.sh --inside
else
    cargo test --locked
    cargo build --locked
    # Keep a parent shell: Xvfb does not send its ready signal to PID 1 in Docker.
    env GTK_IM_MODULE=simple GDK_BACKEND=x11 GSK_RENDERER=cairo xvfb-run -a sh scripts/check.sh --inside
fi
