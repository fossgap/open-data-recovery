#!/bin/sh
# Run the same isolated checks locally and on GitHub Actions. Does not publish.
set -eu
cd "$(dirname "$0")/.."
mode=${1:-check}
case "$mode" in check|release) ;; *) echo 'Usage: scripts/ci.sh check|release' >&2; exit 2;; esac
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
test -n "$version"
if [ "$mode" = release ] && [ "${GITHUB_REF_TYPE:-}" = tag ]; then
    test "${GITHUB_REF_NAME:-}" = "v$version" || { echo 'Release tag must match Cargo.toml version.' >&2; exit 1; }
fi
output=${ODR_CI_OUTPUT:-dist/release}
if [ "$mode" = release ]; then
    test ! -e "$output" || { echo "Choose a new output directory: $output" >&2; exit 1; }
    git diff --quiet HEAD -- || { echo 'Commit tracked changes before making release source archives.' >&2; exit 1; }
fi
engine_container=
build_container=
cleanup() {
    if [ -n "$build_container" ]; then docker rm -f "$build_container" >/dev/null 2>&1 || true; fi
    if [ -n "$engine_container" ]; then docker rm -f "$engine_container" >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT HUP INT TERM
engine_container=$(docker create "${ODR_ENGINE_IMAGE:-open-data-recovery-engines}")
mkdir -p target/engine-package
docker cp "$engine_container:/export/." target/engine-package/
build_container=$(docker create --tmpfs /tmp:rw,exec --shm-size=1g \
    --env GTK_A11Y=none \
    --volume "$PWD/target/engine-package:/workspace/target/engine-package:ro" \
    "${ODR_CI_IMAGE:-open-data-recovery-ci}" sh scripts/ci-inside.sh "$mode")
docker start -a "$build_container"
if [ "$mode" = release ]; then
    mkdir -p "$output"
    docker cp "$build_container:/workspace/dist/release/." "$output/"
    git archive --format=tar.gz --prefix="open-data-recovery-$version/" HEAD > "$output/open-data-recovery-$version-source.tar.gz"
    (cd "$output" && sha256sum ./*.deb ./*.rpm ./*.pkg.tar.zst ./*.AppImage ./*-source.tar.gz > SHA256SUMS && sha256sum -c SHA256SUMS)
fi
