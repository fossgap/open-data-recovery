#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
test "$(uname -m)" = x86_64 || { echo 'This pinned toolchain is x86_64; supply an architecture-specific verified toolchain.' >&2; exit 1; }
test -d dist/OpenDataRecovery.AppDir || { echo 'Build the AppDir first: sh scripts/package.sh appdir' >&2; exit 1; }
cache="$PWD/target/appimage-tools"
mkdir -p "$cache"
tool_version=1.9.1
tool_hash=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
runtime_hash=617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46
if [ -z "${APPIMAGETOOL:-}" ]; then
    tool="$cache/appimagetool-$tool_version.AppImage"
    if [ ! -f "$tool" ]; then curl -fL --retry 2 "https://github.com/AppImage/appimagetool/releases/download/$tool_version/appimagetool-x86_64.AppImage" -o "$tool"; fi
    printf '%s  %s\n' "$tool_hash" "$tool" | sha256sum -c -
    chmod +x "$tool"
    mkdir -p "$cache/appimagetool-$tool_version"
    (cd "$cache/appimagetool-$tool_version" && "$tool" --appimage-extract >/dev/null)
    APPIMAGETOOL="$cache/appimagetool-$tool_version/squashfs-root/AppRun"
fi
if [ ! -f "$cache/runtime-x86_64" ]; then curl -fL --retry 2 https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64 -o "$cache/runtime-x86_64"; fi
printf '%s  %s\n' "$runtime_hash" "$cache/runtime-x86_64" | sha256sum -c -
ARCH=x86_64 "$APPIMAGETOOL" --no-appstream --runtime-file "$cache/runtime-x86_64" --mksquashfs-opt -processors --mksquashfs-opt 2 dist/OpenDataRecovery.AppDir dist/OpenDataRecovery-x86_64.AppImage
