# Native packages and AppImage

See [CI and releases](ci-release.md) for automated checks, package artifacts and tag-based publication.

`sh scripts/package.sh deb|rpm|arch|appdir` packages a release binary, desktop entry, icon, README and license. Artifacts go to `dist/`, which is ignored by Git. Set `ODR_BINARY` to package a binary built elsewhere. The RPM builder keeps its temporary files and private RPM database inside `target/packages`; it does not install packages or modify the system RPM database. `packaging/PKGBUILD` also supports a normal source build with makepkg. Debian control metadata is generated in `scripts/package.sh`; the optional-engine package uses `scripts/package-engines.sh`.

Build on the oldest supported runtime. The included Debian Bookworm container uses GTK 4.8 and glibc 2.36; a binary built on a newer distribution may require newer libraries. The runtime kernel must provide Landlock ABI 3 (Linux 6.2+). Packages are not published or installed automatically.

```sh
cargo build --release --locked
sh scripts/package.sh deb
sh scripts/package.sh rpm
sh scripts/package.sh arch
sh scripts/package.sh appdir
```

The native packages depend on the default engines (Sleuth Kit, TestDisk/PhotoRec and ddrescue) and recommend the other distribution recovery tools. Availability and package names differ between repositories; for example, Debian Bookworm does not provide bulk-extractor in its default repository. The UI detects executable availability rather than assuming an optional package exists. No backend is downloaded or installed during a scan.

## Additional native engines

`packaging/engines/Dockerfile` builds two pinned upstream revisions:

| Engine | Revision | Packaging adaptation |
| --- | --- | --- |
| RecuperaBit | `90930dc8e104bb2198c1596004d355066d314d4b` | Relocatable Python launcher; Python 3.9+ |
| Digler | `2f62fc7e90f5c4123e0f0fa9a31c0128bfffc32c` | Native CLI; build revision recorded in its version banner |

```sh
sh scripts/package-engines.sh deb
ODR_SKIP_ENGINE_BUILD=1 sh scripts/package-engines.sh rpm
ODR_SKIP_ENGINE_BUILD=1 sh scripts/package-engines.sh arch
```

Open Data Recovery is licensed under [GPL-3.0-only](../LICENSE). Application packages include the license in `/usr/share/licenses/open-data-recovery/LICENSE`; the Debian package also installs `/usr/share/doc/open-data-recovery/copyright`. Optional engines retain their upstream licenses. The `open-data-recovery-engines` packages include those licenses and the corresponding RecuperaBit GPL source. They install ordinary executables; they do not add privileged services.

The optional-engine bundle is version 0.1.1. Packaging copies only the two supported engines and their required files, so reusing an older build cache does not include retired engines.

## AppImage

The AppDir bundles GTK runtime libraries, matching GTK/GIO modules and image loaders, schemas, icons and themes. It uses the host libc/loader and installed recovery commands; optional engine packages remain independently installable. Its launcher locates resources relative to itself. Host recovery commands run without the bundled library/module overrides.

Use `sh scripts/appimage.sh` to wrap an existing AppDir. It downloads the official [appimagetool 1.9.1 release](https://github.com/AppImage/appimagetool/releases/tag/1.9.1) only when needed, verifies the pinned SHA-256, extracts it without FUSE, and builds the AppImage. The tool cache is versioned so older downloads cannot be reused accidentally. The pinned runtime download is also verified and requires network access. Set `APPIMAGETOOL` to a previously extracted compatible AppRun to build with your own verified tool.

```sh
sh scripts/package.sh appdir
sh scripts/appimage.sh
./dist/OpenDataRecovery-x86_64.AppImage --help
# On systems without FUSE:
./dist/OpenDataRecovery-x86_64.AppImage --appimage-extract-and-run --help
```

These build recipes produce local review artifacts. Installation, repository signing, and publication are separate distribution operations.

The packaged application can run the same disposable-image recovery check as a source build:

```sh
ODR_BINARY=target/release/open-data-recovery sh scripts/check-ui-recovery.sh
APPIMAGE_EXTRACT_AND_RUN=1 ODR_BINARY=dist/OpenDataRecovery-x86_64.AppImage \
  sh scripts/check-ui-recovery.sh
```
