# CI and releases

[Repository](https://github.com/fossgap/open-data-recovery) · [Development](development.md) · [Packaging](packaging.md)

## Automated checks

[Test CI](../.github/workflows/test-ci.yml) runs for pushes to `main`, pull requests
and manual dispatches. It builds the Debian Bookworm GTK 4.8 environment and the
pinned RecuperaBit, Digler and ext4rescue toolchain. Checks cover Rust formatting,
unit tests, every GtkBuilder layout, GTK smoke checks, all eleven real-engine
integration cases, advanced scan controls, consolidation/recovery/saved-scan management, and partition
discovery through the CLI and production UI.

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
workflows. The `linux-x86_64-release` Actions artifact retains the results for
14 days. A manual run from `main` builds these artifacts without publishing:

```sh
gh workflow run release-ci.yml --repo fossgap/open-data-recovery --ref main
```

## Publish a version

Commit the intended release and ensure `Cargo.toml` has the matching version.
Keep `packaging/PKGBUILD` in sync when changing the application version. The
optional-engine bundle has its own version and pinned upstream revisions.

```sh
git tag -a v0.1.0 -m "Open Data Recovery 0.1.0"
git push origin v0.1.0
```

The workflow rejects tags that do not equal `v` plus the Cargo version. After
building and testing, a separate job verifies the downloaded checksums and
publishes the tagged GitHub Release with generated notes. Only that job has
`contents: write`; it uses GitHub's provided token. No personal access token is
required. An existing release is not overwritten.

## Reproduce locally

```sh
docker build -t open-data-recovery-ci .
docker build -t open-data-recovery-engines -f packaging/engines/Dockerfile .
sh scripts/ci.sh check
sh scripts/ci.sh release
```

The release command requires committed tracked changes and a new output directory,
which defaults to `dist/release/`. Set `ODR_CI_OUTPUT` to another new directory for
a subsequent local build. These local commands build and validate artifacts;
publication is performed only by the tagged GitHub workflow.
