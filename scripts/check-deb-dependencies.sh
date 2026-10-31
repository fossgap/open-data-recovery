#!/bin/sh
# Simulate installation with the retired transitional package unavailable.
# Requires current APT lists; never installs or removes host packages.
set -eu
case "$#" in
    1) set -- "$(realpath "$1")" ;;
    2) set -- "$(realpath "$1")" "$(realpath "$2")" ;;
    *) echo 'Usage: check-deb-dependencies.sh APPLICATION.deb [ENGINES.deb]' >&2; exit 2 ;;
esac
if [ "$(dpkg-query -W -f='${db:Status-Status}' policykit-1 2>/dev/null || true)" = installed ]; then
    echo 'Run this check in an environment without policykit-1 installed, such as the CI container.' >&2
    exit 1
fi
fixture=$(mktemp -d "${TMPDIR:-/tmp}/odr-deb-dependencies-XXXXXX")
trap 'rm -rf "$fixture"' EXIT HUP INT TERM
mkdir "$fixture/preferences.d"
cat > "$fixture/preferences" <<'EOF'
Package: policykit-1
Pin: version *
Pin-Priority: -1
EOF
LC_ALL=C apt-get --simulate --no-remove \
    -o "Dir::Etc::preferences=$fixture/preferences" \
    -o "Dir::Etc::preferencesparts=$fixture/preferences.d" \
    install "$@"
echo 'DEB dependencies resolve without policykit-1 or package removals.'
