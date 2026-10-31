#!/bin/sh
# Runs inside the Debian Bookworm image with temporary, regular-file fixtures.
set -eu
cd "$(dirname "$0")/.."
mode=${1:-check}
case "$mode" in check|release) ;; *) exit 2;; esac
export PATH="$PWD/target/engine-package/usr/bin:$PATH"
for engine in recuperabit digler; do command -v "$engine" >/dev/null; done
cargo fmt --all -- --check
xvfb-run -a sh scripts/check.sh --inside
sh scripts/check-engines.sh
for language in en fr ar; do
    ODR_LANGUAGE="$language" sh scripts/check-ui-advanced.sh
    ODR_LANGUAGE="$language" sh scripts/check-ui-restoration.sh
done
ODR_LANGUAGE=en ODR_TEST_STORAGE=/tmp ODR_CONSOLIDATE=1 ODR_MANAGE_SCANS=1 sh scripts/check-ui-recovery.sh
ODR_LANGUAGE=en ODR_TEST_STORAGE=/tmp sh scripts/check-ui-partitions.sh
if [ "$mode" = release ]; then
    cargo build --release --locked
    for format in deb rpm arch; do
        sh scripts/package.sh "$format"
        ODR_SKIP_ENGINE_BUILD=1 sh scripts/package-engines.sh "$format"
    done
    apt-get update
    sh scripts/check-deb-dependencies.sh dist/open-data-recovery_*.deb dist/open-data-recovery-engines_*.deb
    sh scripts/package.sh appdir
    sh scripts/appimage.sh
    # Detect missing bundled runtime data instead of borrowing the host MIME DB.
    runtime_data=$(mktemp -d /tmp/odr-appimage-data.XXXXXX)
    APPIMAGE_EXTRACT_AND_RUN=1 XDG_DATA_HOME="$runtime_data" XDG_DATA_DIRS="$runtime_data" \
        ODR_LANGUAGE=en G_DEBUG=fatal-criticals xvfb-run -a dist/OpenDataRecovery-x86_64.AppImage --smoke-test
    rmdir "$runtime_data"
    APPIMAGE_EXTRACT_AND_RUN=1 ODR_BINARY=dist/OpenDataRecovery-x86_64.AppImage \
        ODR_LANGUAGE=en ODR_TEST_STORAGE=/tmp ODR_CONSOLIDATE=1 ODR_MANAGE_SCANS=1 sh scripts/check-ui-recovery.sh
    APPIMAGE_EXTRACT_AND_RUN=1 ODR_BINARY=dist/OpenDataRecovery-x86_64.AppImage \
        ODR_LANGUAGE=en ODR_TEST_STORAGE=/tmp sh scripts/check-ui-partitions.sh
    mkdir dist/release
    mv dist/*.deb dist/*.rpm dist/*.pkg.tar.zst dist/*.AppImage dist/release/
fi
