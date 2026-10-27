#!/bin/sh
# Build an installable native package from a release binary. No install or publish occurs.
set -eu
cd "$(dirname "$0")/.."
format=${1:-deb}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
binary=${ODR_BINARY:-target/release/open-data-recovery}
if [ ! -f "$binary" ]; then cargo build --release --locked; fi
binary=$(realpath "$binary")
mkdir -p dist target/packages
stage=$(mktemp -d "$PWD/target/packages/stage.XXXXXX")
trap 'rm -rf "$stage"' EXIT HUP INT TERM
chmod 755 "$stage"
install -Dm755 "$binary" "$stage/usr/bin/open-data-recovery"
install -Dm644 packaging/org.opendatarecovery.App.desktop "$stage/usr/share/applications/org.opendatarecovery.App.desktop"
install -Dm644 packaging/org.opendatarecovery.App.svg "$stage/usr/share/icons/hicolor/scalable/apps/org.opendatarecovery.App.svg"
install -Dm644 README.md "$stage/usr/share/doc/open-data-recovery/README.md"
case "$format" in
deb)
    arch=$(dpkg --print-architecture)
    mkdir -p "$stage/DEBIAN"
    cat > "$stage/DEBIAN/control" <<EOF
Package: open-data-recovery
Version: $version
Architecture: $arch
Maintainer: Open Data Recovery maintainers
Section: utils
Priority: optional
Depends: libc6 (>= 2.36), libgtk-4-1 (>= 4.8), libglib2.0-0 (>= 2.74), util-linux, udisks2, policykit-1, sleuthkit, testdisk, gddrescue
Recommends: ext4magic, extundelete, ntfs-3g, btrfs-progs, e2fsprogs, ewf-tools, scalpel, foremost, safecopy, magicrescue, recoverjpeg, bulk-extractor, open-data-recovery-engines
Description: Native Linux data recovery workspace
 Read-only source access, persistent scans, and verified recovery to a separate drive.
EOF
    dpkg-deb --root-owner-group --build "$stage" "dist/open-data-recovery_${version}_${arch}.deb"
    ;;
rpm)
    root="$PWD/target/packages/rpm"
    mkdir -p "$root/BUILD" "$root/RPMS" "$root/SOURCES" "$root/SPECS" "$root/SRPMS"
    tar -C "$stage" -czf "$root/SOURCES/payload.tar.gz" usr
    cat > "$root/SPECS/open-data-recovery.spec" <<EOF
Name: open-data-recovery
Version: $version
Release: 1
Summary: Native Linux data recovery workspace
License: LicenseRef-Unspecified
Source0: payload.tar.gz
AutoReqProv: no
Requires: glibc >= 2.36, gtk4 >= 4.8, glib2 >= 2.74, util-linux, udisks2, polkit, sleuthkit, testdisk, ddrescue
Recommends: ext4magic, extundelete, ntfs-3g, btrfs-progs, e2fsprogs, libewf-tools, scalpel, foremost, safecopy, magicrescue, recoverjpeg, bulk_extractor, open-data-recovery-engines
%description
Read-only source access, persistent scans, and verified recovery to a separate drive.
%install
mkdir -p %{buildroot}
tar -xzf %{SOURCE0} -C %{buildroot}
%files
/usr/bin/open-data-recovery
/usr/share/applications/org.opendatarecovery.App.desktop
/usr/share/icons/hicolor/scalable/apps/org.opendatarecovery.App.svg
/usr/share/doc/open-data-recovery/README.md
EOF
    mkdir -p "$root/tmp" "$root/rpmdb"
    rpmbuild -bb --define "_topdir $root" --define "_tmppath $root/tmp" --define "_dbpath $root/rpmdb" --nodeps --define '_build_id_links none' --define '__os_install_post %{nil}' "$root/SPECS/open-data-recovery.spec"
    find "$root/RPMS" -name '*.rpm' -exec cp '{}' dist/ \;
    ;;
arch)
    cat > "$stage/.PKGINFO" <<EOF
pkgname = open-data-recovery
pkgbase = open-data-recovery
pkgver = $version-1
pkgdesc = Native Linux data recovery workspace
builddate = $(date +%s)
packager = Open Data Recovery maintainers
size = $(du -sb "$stage/usr" | cut -f1)
arch = $(uname -m)
license = LicenseRef-Unspecified
depend = gtk4
depend = glib2
depend = util-linux
depend = udisks2
depend = polkit
depend = sleuthkit
depend = testdisk
depend = ddrescue
optdepend = open-data-recovery-engines: RecuperaBit, Digler and ext4rescue
optdepend = ext4magic: ext journal recovery
optdepend = extundelete: ext journal undelete
optdepend = ntfs-3g: NTFS undelete
optdepend = btrfs-progs: btrfs recovery
optdepend = e2fsprogs: ext diagnostics
optdepend = libewf: E01 support
optdepend = scalpel: signature carving
optdepend = foremost: signature carving
optdepend = safecopy: clone alternative
optdepend = magicrescue: recipe carving
optdepend = recoverjpeg: JPEG recovery
optdepend = bulk_extractor: forensic feature extraction
EOF
    tar --zstd -cf "dist/open-data-recovery-${version}-1-$(uname -m).pkg.tar.zst" --owner=0 --group=0 -C "$stage" .PKGINFO usr
    ;;
appdir)
    destination="$PWD/dist/OpenDataRecovery.AppDir"
    if [ -e "$destination" ]; then echo "Remove the previous AppDir before rebuilding: $destination" >&2; exit 1; fi
    mv "$stage" "$destination"
    install -m755 packaging/AppRun "$destination/AppRun"
    cp packaging/org.opendatarecovery.App.desktop "$destination/"
    cp packaging/org.opendatarecovery.App.svg "$destination/"
    mkdir -p "$destination/usr/lib" "$destination/usr/share/glib-2.0"
    # Bundle runtime libraries, retaining the host's libc/loader for compatibility.
    ldd "$binary" | awk '/=> \// {print $3}' | while IFS= read -r library; do
        case "$(basename "$library")" in libc.so.*|libm.so.*|libdl.so.*|libpthread.so.*|librt.so.*|ld-linux*) continue;; esac
        cp -L "$library" "$destination/usr/lib/"
    done
    cp -a /usr/share/glib-2.0/schemas "$destination/usr/share/glib-2.0/"
    mkdir -p "$destination/usr/lib/gio/modules" "$destination/usr/lib/gtk-4.0"
    # Bundle modules from the same distribution as GTK; host modules may target
    # newer GLib symbols even when the executable itself remains compatible.
    for modules in /usr/lib/*/gio/modules; do
        if [ -d "$modules" ]; then cp -a "$modules/." "$destination/usr/lib/gio/modules/"; fi
    done
    for modules in /usr/lib/*/gtk-4.0; do
        if [ -d "$modules" ]; then cp -a "$modules/." "$destination/usr/lib/gtk-4.0/"; fi
    done
    for loaders in /usr/lib/*/gdk-pixbuf-2.0/*/loaders; do
        if [ -d "$loaders" ]; then
            cp -L "$loaders/"*.so "$destination/usr/lib/"
            sed -E 's@"[^\"]*/(libpixbufloader-[^\"]+)"@"\1"@' "$(dirname "$loaders")/loaders.cache" > "$destination/usr/lib/loaders.cache"
        fi
    done
    find "$destination/usr/lib" -name '*.so' -print | while IFS= read -r module; do
        ldd "$module" | awk '/=> \// {print $3}' | while IFS= read -r library; do
            case "$(basename "$library")" in libc.so.*|libm.so.*|libdl.so.*|libpthread.so.*|librt.so.*|ld-linux*) continue;; esac
            cp -L "$library" "$destination/usr/lib/"
        done
    done
    for data in icons themes; do if [ -d "/usr/share/$data" ]; then cp -a "/usr/share/$data" "$destination/usr/share/"; fi; done
    echo "AppDir created: $destination"
    echo "Recovery engines are host packages; see README and Help → Backend availability."
    ;;
*) echo 'Usage: scripts/package.sh deb|rpm|arch|appdir' >&2; exit 2;;
esac
