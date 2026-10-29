#!/bin/sh
# Build pinned optional engines and wrap their payload in the requested native format.
set -eu
cd "$(dirname "$0")/.."
format=${1:-deb}
version=0.1.1
if [ "${ODR_SKIP_ENGINE_BUILD:-0}" != 1 ]; then
    docker build -t open-data-recovery-engines -f packaging/engines/Dockerfile .
    container=$(docker create open-data-recovery-engines)
    trap 'docker rm "$container" >/dev/null' EXIT HUP INT TERM
    mkdir -p target/engine-package
    docker cp "$container:/export/." target/engine-package/
    docker rm "$container" >/dev/null
    trap - EXIT HUP INT TERM
fi
payload="$PWD/target/engine-package"
for engine in recuperabit digler; do
    test -x "$payload/usr/bin/$engine"
done
mkdir -p dist target/packages
root=$(mktemp -d "$PWD/target/packages/engine-stage.XXXXXX")
trap 'rm -rf "$root"' EXIT HUP INT TERM
chmod 755 "$root"
# Copy only supported components, including when reusing an older build cache.
for path in \
    bin/recuperabit \
    bin/digler \
    lib/open-data-recovery-engines/recuperabit \
    share/doc/open-data-recovery-engines/RecuperaBit.LICENSE \
    share/doc/open-data-recovery-engines/Digler.LICENSE \
    share/doc/open-data-recovery-engines/sources/recuperabit.tar.gz; do
    mkdir -p "$root/usr/$(dirname "$path")"
    cp -a "$payload/usr/$path" "$root/usr/$path"
done
find "$root" -type d -name __pycache__ -prune -exec rm -rf '{}' +
case "$format" in
deb)
    mkdir -p "$root/DEBIAN"
    cat > "$root/DEBIAN/control" <<EOF
Package: open-data-recovery-engines
Version: $version
Architecture: $(dpkg --print-architecture)
Maintainer: Open Data Recovery maintainers
Section: utils
Priority: optional
Depends: python3 (>= 3.9)
Description: Additional recovery engines for Open Data Recovery
 Pinned RecuperaBit and Digler builds. Includes licenses and
 the corresponding source for the GPL component.
EOF
    dpkg-deb --root-owner-group --build "$root" "dist/open-data-recovery-engines_${version}_$(dpkg --print-architecture).deb"
    ;;
rpm)
    build="$PWD/target/packages/engine-rpm"
    mkdir -p "$build/SOURCES" "$build/SPECS" "$build/BUILD" "$build/RPMS" "$build/SRPMS"
    tar -C "$root" -czf "$build/SOURCES/engines.tar.gz" usr
    cat > "$build/SPECS/engines.spec" <<EOF
Name: open-data-recovery-engines
Version: $version
Release: 1
Summary: RecuperaBit and Digler recovery engines
License: GPL-3.0-or-later AND MIT
Source0: engines.tar.gz
AutoReqProv: no
Requires: python3 >= 3.9
%description
Pinned recovery engines, licenses and corresponding GPL sources.
%install
mkdir -p %{buildroot}
tar -xzf %{SOURCE0} -C %{buildroot}
%files
/usr/bin/recuperabit
/usr/bin/digler
/usr/lib/open-data-recovery-engines
/usr/share/doc/open-data-recovery-engines
EOF
    mkdir -p "$build/tmp" "$build/rpmdb"
    rpmbuild -bb --define "_topdir $build" --define "_tmppath $build/tmp" --define "_dbpath $build/rpmdb" --nodeps --define '_build_id_links none' --define '__os_install_post %{nil}' "$build/SPECS/engines.spec"
    find "$build/RPMS" -name "open-data-recovery-engines-$version-1.*.rpm" -exec cp '{}' dist/ \;
    ;;
arch)
    cat > "$root/.PKGINFO" <<EOF
pkgname = open-data-recovery-engines
pkgbase = open-data-recovery-engines
pkgver = $version-1
pkgdesc = RecuperaBit and Digler recovery engines
arch = $(uname -m)
builddate = $(date +%s)
packager = Open Data Recovery maintainers
size = $(du -sb "$root/usr" | cut -f1)
license = GPL-3.0-or-later
license = MIT
depend = python
EOF
    tar --zstd -cf "dist/open-data-recovery-engines-$version-1-$(uname -m).pkg.tar.zst" --owner=0 --group=0 -C "$root" .PKGINFO usr
    ;;
*) echo 'Usage: scripts/package-engines.sh deb|rpm|arch' >&2; exit 2;;
esac
