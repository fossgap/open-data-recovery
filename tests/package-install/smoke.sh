#!/bin/sh
set -eu
test "$(id -u)" != 0
binary=${1:?Choose installed binary}
export ODR_BINARY="$binary" ODR_LANGUAGE=en
export GTK_IM_MODULE=simple GTK_A11Y=none G_DEBUG=fatal-criticals
export GDK_BACKEND=x11 GSK_RENDERER=cairo
export ODR_TEST_STORAGE=/tmp ODR_CONSOLIDATE=1 ODR_MANAGE_SCANS=1
if [ "$binary" = /opt/OpenDataRecovery.AppImage ]; then export APPIMAGE_EXTRACT_AND_RUN=1; fi
fixture=$(mktemp -d /tmp/odr-installed-test-XXXXXX)
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
export XDG_CONFIG_HOME="$fixture/config" XDG_DATA_HOME="$fixture/data"
xvfb-run -a "$binary" --smoke-test
xvfb-run -a "$binary" --live-smoke-test
for language in en fr ar; do
    ODR_LANGUAGE="$language" xvfb-run -a "$binary" --i18n-smoke-test
done
if command -v foremost >/dev/null && command -v extundelete >/dev/null; then
    sh /workspace/scripts/check-ui-advanced.sh
else
    echo 'Advanced-scan fixture requires optional Foremost and extundelete; testing the installed default workflows below.'
fi
sh /workspace/scripts/check-ui-recovery.sh
sh /workspace/scripts/check-ui-partitions.sh
truncate -s 4M "$fixture/restore.img"
before=$(sha256sum "$fixture/restore.img")
ODR_SMOKE_RESTORE_SOURCE="$fixture/restore.img" xvfb-run -a "$binary" --live-smoke-test
test "$before" = "$(sha256sum "$fixture/restore.img")"
python3 /odr-test/engines.py "$binary" "$fixture"
echo 'INSTALLED_BINARY_GUI_RECOVERY_PARTITION_RESTORATION_AND_ENGINE_CHECKS_PASSED'
