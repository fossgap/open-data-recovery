#!/bin/sh
# Themes use disposable settings and widgets; no source discovery or recovery jobs.
set -eu
cd "$(dirname "$0")/.."
if [ "${1:-}" != --inside ]; then
    if [ -z "${ODR_BINARY:-}" ]; then cargo build --locked; fi
    exec env GTK_IM_MODULE=simple GTK_A11Y=none GDK_BACKEND=x11 GSK_RENDERER=cairo \
        xvfb-run -a sh scripts/check-ui-themes.sh --inside
fi
: "${ODR_BINARY:=target/debug/open-data-recovery}"
for theme in Adwaita Adwaita:dark HighContrast; do
    for preference in system custom unknown; do
        GTK_THEME="$theme" ODR_SMOKE_THEME="$preference" ODR_LANGUAGE=en \
            G_DEBUG=fatal-warnings "$ODR_BINARY" --appearance-smoke-test
    done
done
GTK_THEME=Adwaita:dark ODR_SMOKE_THEME=custom ODR_LANGUAGE=ar \
    G_DEBUG=fatal-warnings "$ODR_BINARY" --appearance-smoke-test
