#!/bin/sh
# Uses a disposable regular image. Never confirms or writes to a physical disk.
set -eu
cd "$(dirname "$0")/.."
: "${ODR_BINARY:=target/debug/open-data-recovery}"
while [ "$#" -gt 0 ]; do
    case "$1" in
        --screenshots) ODR_RESTORE_SCREENSHOTS="${2:?Choose a screenshot output directory}"; export ODR_RESTORE_SCREENSHOTS; shift 2 ;;
        --language) ODR_LANGUAGE="${2:?Choose a language}"; export ODR_LANGUAGE; shift 2 ;;
        *) echo 'Usage: check-ui-restoration.sh [--language CODE] [--screenshots DIRECTORY]' >&2; exit 2 ;;
    esac
done
env GTK_IM_MODULE=simple GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals \
    xvfb-run -a cargo test --locked gtk_restore_wizard -- --ignored --test-threads=1
fixture=$(mktemp -d "${TMPDIR:-/tmp}/odr-ui-restoration-XXXXXX")
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
truncate -s 4M "$fixture/source.img"
before=$(sha256sum "$fixture/source.img")
env GTK_IM_MODULE=simple GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals \
    XDG_CONFIG_HOME="$fixture/settings" XDG_DATA_HOME="$fixture/data" \
    ODR_SMOKE_RESTORE_SOURCE="$fixture/source.img" \
    xvfb-run -a "$ODR_BINARY" --live-smoke-test
test "$before" = "$(sha256sum "$fixture/source.img")"
