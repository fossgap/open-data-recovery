#!/bin/sh
# Invoked by check-package-installation.sh inside its disposable container only.
set -eu
if [ "${ODR_INSTALL_TEST_CONTAINER:-}" != 1 ] || [ ! -f /.dockerenv ] || [ ! -d /artifacts ]; then
    echo 'Use scripts/check-package-installation.sh; this script installs container packages.' >&2
    exit 2
fi
format=${1:?Choose deb, rpm, arch or appimage}
version=${2:?Provide the expected application version}
one_file() {
    if [ "$#" -ne 1 ] || [ ! -f "$1" ]; then
        echo 'Expected exactly one release artifact for this package.' >&2
        exit 1
    fi
    printf '%s\n' "$1"
}
cat /etc/os-release
case "$format" in
    deb)
        app=$(one_file /artifacts/open-data-recovery_*_amd64.deb)
        engines=$(one_file /artifacts/open-data-recovery-engines_*_amd64.deb)
        test "$(dpkg-deb -f "$app" Version)" = "$version"
        apt-get update
        apt-get -y --no-remove install "$app" "$engines"
        dpkg-query -W -f='${binary:Package} ${Version} ${db:Status-Abbrev}\n' open-data-recovery open-data-recovery-engines polkitd
        test "$(dpkg-query -W -f='${Version}' open-data-recovery)" = "$version"
        test "$(dpkg-query -W -f='${Version}' open-data-recovery-engines)" = "$(dpkg-deb -f "$engines" Version)"
        apt-get check
        ;;
    rpm)
        app=$(one_file /artifacts/open-data-recovery-[0-9]*.x86_64.rpm)
        engines=$(one_file /artifacts/open-data-recovery-engines-*.x86_64.rpm)
        dnf -y install "$app" "$engines"
        rpm -q open-data-recovery open-data-recovery-engines
        test "$(rpm -q --qf '%{VERSION}' open-data-recovery)" = "$version"
        for package in "$app" "$engines"; do
            name=$(rpm -qp --qf '%{NAME}' "$package")
            test "$(rpm -q --qf '%{VERSION}-%{RELEASE}' "$name")" = "$(rpm -qp --qf '%{VERSION}-%{RELEASE}' "$package")"
        done
        dnf check
        ;;
    arch)
        app=$(one_file /artifacts/open-data-recovery-[0-9]*-x86_64.pkg.tar.zst)
        engines=$(one_file /artifacts/open-data-recovery-engines-*-x86_64.pkg.tar.zst)
        pacman --noconfirm -Syu
        pacman --noconfirm -U "$app" "$engines"
        pacman -Q open-data-recovery open-data-recovery-engines
        case "$(pacman -Q open-data-recovery)" in
            "open-data-recovery $version-"*) ;;
            *) echo 'Installed application version does not match the release.' >&2; exit 1 ;;
        esac
        test "$(pacman -Q open-data-recovery)" = "$(pacman -Qp "$app")"
        test "$(pacman -Q open-data-recovery-engines)" = "$(pacman -Qp "$engines")"
        pacman -Qk open-data-recovery open-data-recovery-engines
        ;;
    appimage)
        app=$(one_file /artifacts/open-data-recovery_*_amd64.deb)
        engines=$(one_file /artifacts/open-data-recovery-engines_*_amd64.deb)
        appimage=$(one_file /artifacts/OpenDataRecovery-x86_64.AppImage)
        test "$(dpkg-deb -f "$app" Version)" = "$version"
        apt-get update
        # Install host recovery tools and test tools, without GTK or its data.
        apt-get -y --no-install-recommends install xvfb xauth dbus-x11 python3 e2fsprogs fonts-dejavu-core util-linux sleuthkit testdisk gddrescue ntfs-3g diffutils "$engines"
        install -m755 "$appimage" /opt/OpenDataRecovery.AppImage
        export APPIMAGE_EXTRACT_AND_RUN=1
        test "$(dpkg-query -W -f='${db:Status-Status}' libgtk-4-1 2>/dev/null || true)" != installed
        test ! -f /usr/share/mime/mime.cache
        test ! -f /etc/fonts/fonts.conf
        echo 'APPIMAGE_HOST_HAS_NO_GTK_MIME_DATABASE_OR_FONTCONFIG_CONFIG'
        ;;
    *) echo "Unknown package format: $format" >&2; exit 2 ;;
esac
binary=/usr/bin/open-data-recovery
useradd -m odrtest
if [ "$format" = appimage ]; then
    binary=/opt/OpenDataRecovery.AppImage
    # Check that the portable binary is the same version/build as the native DEB.
    inspection=$(runuser -u odrtest -- mktemp -d /tmp/odr-appimage-inspect.XXXXXX)
    dpkg-deb -x "$app" "$inspection/deb"
    runuser -u odrtest -- sh -c 'cd "$1"; "$2" --appimage-extract >/dev/null' sh "$inspection" "$binary"
    cmp "$inspection/deb/usr/bin/open-data-recovery" "$inspection/squashfs-root/usr/bin/open-data-recovery"
    rm -rf "$inspection"
fi
runuser -u odrtest -- "$binary" --help >/tmp/odr-installed-help.txt
runuser -u odrtest -- recuperabit --help >/tmp/odr-recuperabit-help.txt
runuser -u odrtest -- digler --help >/tmp/odr-digler-help.txt
runuser -u odrtest -- "$binary" --backends >/tmp/odr-installed-backends.txt
cat /tmp/odr-installed-backends.txt
for engine in fls icat mmls testdisk photorec ddrescue dd recuperabit digler; do
    grep -q "^$engine: " /tmp/odr-installed-backends.txt
done
echo 'PACKAGES_INSTALLED_AND_REQUIRED_BACKENDS_DETECTED'
# Provision extra fixture tools after checking the installed application's deps.
case "$format" in
    deb)
        apt-get -y --no-install-recommends install xvfb xauth dbus-x11 python3 e2fsprogs fonts-dejavu-core ntfs-3g diffutils
        apt-get clean
        ;;
    rpm)
        dnf -y install xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon python3 e2fsprogs dejavu-sans-fonts ntfs-3g diffutils
        dnf clean packages
        ;;
    arch)
        pacman --noconfirm -S --needed xorg-server-xvfb xorg-xauth dbus python e2fsprogs ttf-dejavu ntfs-3g diffutils
        ;;
    appimage) apt-get clean ;;
esac
for tool in xvfb-run cmp truncate sha256sum mktemp mkfs.ext2 debugfs mkfs.ntfs ntfscp python3; do
    command -v "$tool" >/dev/null
done
runuser -u odrtest -- /bin/sh /odr-test/smoke.sh "$binary"
echo "PACKAGE_INSTALLATION_SUITE_PASSED: $format"
