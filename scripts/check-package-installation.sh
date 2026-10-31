#!/bin/sh
# Install release artifacts only in disposable containers, never on the host.
set -eu
cd "$(dirname "$0")/.."
if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
    echo 'Usage: check-package-installation.sh debian|kali|fedora|arch|appimage [ARTIFACT_DIRECTORY]' >&2
    exit 2
fi
case "$1" in
    debian) image=debian:bookworm-slim; format=deb ;;
    kali) image=kalilinux/kali-rolling:latest; format=deb ;;
    fedora) image=registry.fedoraproject.org/fedora:44; format=rpm ;;
    arch) image=archlinux:base; format=arch ;;
    appimage) image=ubuntu:24.04; format=appimage ;;
    *) echo "Unknown installation target: $1" >&2; exit 2 ;;
esac
test "$(uname -m)" = x86_64 || { echo 'Release installation checks require an x86_64 host.' >&2; exit 1; }
artifacts=$(realpath "${2:-dist/release}")
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
test -n "$version"
(cd "$artifacts" && sha256sum -c SHA256SUMS)
docker pull "$image"
docker image inspect "$image" --format '{{json .RepoTags}} {{json .RepoDigests}} {{.Id}}'
container=$(docker create --tmpfs /tmp:rw,exec --shm-size=1g \
    --env ODR_INSTALL_TEST_CONTAINER=1 --env DEBIAN_FRONTEND=noninteractive \
    --volume "$artifacts:/artifacts:ro" \
    --volume "$PWD/scripts:/workspace/scripts:ro" \
    --volume "$PWD/tests:/workspace/tests:ro" \
    --volume "$PWD/tests/package-install:/odr-test:ro" \
    "$image" sh /odr-test/install.sh "$format" "$version")
trap 'docker rm -f "$container" >/dev/null 2>&1 || true' EXIT HUP INT TERM
docker start -a "$container"
