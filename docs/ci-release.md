# CI and releases

[Repository](https://github.com/fossgap/open-data-recovery) · [Development](development.md) · [Packaging](packaging.md)

## Automated checks

[Test CI](../.github/workflows/test-ci.yml) runs for pushes to `main`, pull requests
and manual dispatches. It builds the Debian Bookworm GTK 4.8 environment and the
pinned RecuperaBit and Digler toolchain. Checks cover Rust formatting,
unit tests, every GtkBuilder layout, GTK smoke checks, disposable real-engine
integration cases (including GNU dd restoration), advanced scan and image-restoration controls,
consolidation/recovery/saved-scan management, and partition discovery through
the CLI and production UI.

Fixtures are regular disposable image files on container tmpfs mounts. Tests do
not receive host devices. Actions are pinned to commit hashes; the test and build
jobs have read-only repository permissions.

## Package builds

[Release CI](../.github/workflows/release-ci.yml) runs when a `v*` tag is pushed or
when manually dispatched. It repeats the checks, builds the release binary and
produces:

- Application packages: DEB, RPM, Arch and AppImage.
- Optional-engine packages: DEB, RPM and Arch, with upstream licenses and GPL sources.
- An archive of the exact committed application source.
- `SHA256SUMS` covering all eight downloadable files.

The AppImage also passes the production recovery and partition-discovery GUI
workflows, including a startup check with host MIME search paths hidden. The
`linux-x86_64-release` Actions artifact retains the packages for 14 days.

Five installation jobs download those exact artifacts, verify `SHA256SUMS`, and
test them in fresh x86_64 containers:

| Target | Application | Optional engines |
| --- | --- | --- |
| Debian 12 Bookworm | DEB installed with APT | DEB |
| Kali rolling | DEB installed with APT | DEB |
| Fedora 44 | RPM installed with DNF | RPM |
| Arch rolling | Arch package installed with pacman | Arch package |
| Ubuntu 24.04 | AppImage, extracted without FUSE | DEB |

Native-package jobs check installed versions and dependency resolution. The
AppImage job checks that its binary matches the application DEB and runs without
host GTK libraries, a MIME database, or Fontconfig configuration. Every job runs
ODR as an ordinary user and checks demo/live startup, English/French/Arabic UI,
recovery and saved scans, CLI/GUI partition discovery, restoration navigation,
and actual PNG carving with Digler and NTFS reconstruction with RecuperaBit.
Recovered bytes must match the fixtures, and source images must remain unchanged.
Advanced-scan controls are also exercised when Foremost and extundelete are
installed. All fixtures are disposable regular files; containers receive no host
devices. Base-image digests and installation/test output are retained in the
`package-verification-<target>` Actions artifacts for 14 days, including failures.

A manual run from `main` builds and verifies these artifacts without publishing:

```sh
gh workflow run release-ci.yml --repo fossgap/open-data-recovery --ref main
```

## Publish a version

Commit the intended release and ensure `Cargo.toml` has the matching version.
Keep `packaging/PKGBUILD` in sync when changing the application version. The
optional-engine bundle has its own version and pinned upstream revisions.

```sh
git tag -a v0.1.6 -m "Open Data Recovery 0.1.6"
git push origin v0.1.6
```

The workflow rejects tags that do not equal `v` plus the Cargo version. After
building, testing, and passing all five installation jobs, a separate job verifies
the downloaded checksums and publishes the tagged GitHub Release with generated
notes. Only that job has
`contents: write`; it uses GitHub's provided token. No personal access token is
required. An existing release is not overwritten.

## Reproduce locally

```sh
docker build -t open-data-recovery-ci .
docker build -t open-data-recovery-engines -f packaging/engines/Dockerfile .
sh scripts/ci.sh check
sh scripts/ci.sh release
for target in debian kali fedora arch appimage; do
    sh scripts/check-package-installation.sh "$target" dist/release
done
```

The release command requires committed tracked changes and a new output directory,
which defaults to `dist/release/`. Set `ODR_CI_OUTPUT` to another new directory for
a subsequent local build. The installation checker accepts that artifact directory
as its second argument and installs only inside disposable Docker containers;
it does not install packages on the host. These local commands build and validate
artifacts; publication is performed only by the tagged GitHub workflow.
