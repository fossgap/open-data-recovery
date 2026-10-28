#!/bin/sh
# Runs inside the Debian Bookworm image with temporary, regular-file fixtures.
set -eu
cd "$(dirname "$0")/.."
mode=${1:-check}
case "$mode" in check|release) ;; *) exit 2;; esac
export PATH="$PWD/target/engine-package/usr/bin:$PATH"
for engine in recuperabit digler ext4rescue; do command -v "$engine" >/dev/null; done
cargo fmt --all -- --check
xvfb-run -a sh scripts/check.sh --inside
sh scripts/check-engines.sh
sh scripts/check-ui-advanced.sh
ODR_TEST_STORAGE=/tmp ODR_CONSOLIDATE=1 ODR_MANAGE_SCANS=1 sh scripts/check-ui-recovery.sh
ODR_TEST_STORAGE=/tmp sh scripts/check-ui-partitions.sh
if [ "$mode" = release ]; then
    cargo build --release --locked
    for format in deb rpm arch; do
        sh scripts/package.sh "$format"
        ODR_SKIP_ENGINE_BUILD=1 sh scripts/package-engines.sh "$format"
    done
    sh scripts/package.sh appdir
    sh scripts/appimage.sh
    APPIMAGE_EXTRACT_AND_RUN=1 ODR_BINARY=dist/OpenDataRecovery-x86_64.AppImage \
        ODR_TEST_STORAGE=/tmp ODR_CONSOLIDATE=1 ODR_MANAGE_SCANS=1 sh scripts/check-ui-recovery.sh
    APPIMAGE_EXTRACT_AND_RUN=1 ODR_BINARY=dist/OpenDataRecovery-x86_64.AppImage \
        ODR_TEST_STORAGE=/tmp sh scripts/check-ui-partitions.sh
    mkdir dist/release
    mv dist/*.deb dist/*.rpm dist/*.pkg.tar.zst dist/*.AppImage dist/release/
fi
