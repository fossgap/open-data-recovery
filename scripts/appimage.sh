#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -m)" = x86_64 || { echo 'This pinned toolchain is x86_64; supply an architecture-specific verified toolchain.' >&2; exit 1; }
test -d dist/OpenDataRecovery.AppDir || { echo 'Build the AppDir first: sh scripts/package.sh appdir' >&2; exit 1; }
cache="$PWD/target/appimage-tools"
mkdir -p "$cache"
tool_hash=95cbe7cce9717fce90c484e34052ee7c7f1d7635b33c12525b4776826a7d29b6
runtime_hash=617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46
if [ -z "${APPIMAGETOOL:-}" ]; then
    if [ ! -f "$cache/appimagetool" ]; then curl -fL --retry 2 https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage -o "$cache/appimagetool"; fi
    printf '%s  %s\n' "$tool_hash" "$cache/appimagetool" | sha256sum -c -
    chmod +x "$cache/appimagetool"
    (cd "$cache" && ./appimagetool --appimage-extract >/dev/null)
    APPIMAGETOOL="$cache/squashfs-root/AppRun"
fi
if [ ! -f "$cache/runtime-x86_64" ]; then curl -fL --retry 2 https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64 -o "$cache/runtime-x86_64"; fi
printf '%s  %s\n' "$runtime_hash" "$cache/runtime-x86_64" | sha256sum -c -
ARCH=x86_64 "$APPIMAGETOOL" --no-appstream --runtime-file "$cache/runtime-x86_64" --mksquashfs-opt -processors --mksquashfs-opt 2 dist/OpenDataRecovery.AppDir dist/OpenDataRecovery-x86_64.AppImage
