#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
: "${ODR_BINARY:=target/debug/open-data-recovery}"
fixture=$(mktemp -d "${TMPDIR:-/tmp}/odr-ui-advanced-XXXXXX")
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
mkdir "$fixture/input"
printf 'Advanced scan review fixture\n' > "$fixture/input/notes.txt"
truncate -s 32M "$fixture/source.img"
mkfs.ext2 -q -F -d "$fixture/input" "$fixture/source.img"
before=$(sha256sum "$fixture/source.img")
env GTK_IM_MODULE=simple GDK_BACKEND=x11 GSK_RENDERER=cairo G_DEBUG=fatal-criticals \
    ODR_SMOKE_ADVANCED_SOURCE="$fixture/source.img" \
    ODR_SMOKE_ADVANCED_WORKSPACE="$fixture/workspace" \
    xvfb-run -a "$ODR_BINARY" --live-smoke-test
test "$before" = "$(sha256sum "$fixture/source.img")"
