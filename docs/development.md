# Development and validation

[README](../README.md) · [Repository Guidelines](../AGENTS.md) · [Packaging](packaging.md) · [CI and releases](ci-release.md)

## Build locally

Use Rust 1.85 or newer, a C compiler, `pkg-config` and GTK 4.8+ development headers.
The backend sandbox requires Linux 6.2+ with Landlock ABI 3. Install the recovery
tools needed by the cases you run; `cargo run --locked -- --backends` lists them.

```sh
cargo build --locked
cargo run --locked
cargo run --locked -- --demo
cargo fmt --all -- --check
cargo test --locked
sh scripts/check.sh
```

`check.sh` runs unit tests, builds the application, validates every GtkBuilder
layout, and runs demo/production smoke checks in Xvfb. It needs `xvfb`, `xauth`
and `gtk4-builder-tool`. The pinned Debian Bookworm `Dockerfile` includes these
dependencies; its default command runs the same checks. Use
`docker run --rm --tmpfs /tmp:rw,exec open-data-recovery-dev` so test images live
on tmpfs rather than Docker's overlay filesystem. The Docker build and
devcontainer configure this temporary filesystem too.

## Real engines and GTK workflows

Use only disposable regular image files. The default fixture workflows create
sources in `/tmp` and outputs in `/dev/shm`; these must be separate filesystems.
Do not run checks on physical recovery devices or user snapshots.

```sh
sh scripts/check-engines.sh
sh scripts/check-ui-recovery.sh
ODR_CONSOLIDATE=1 ODR_MANAGE_SCANS=1 sh scripts/check-ui-recovery.sh
sh scripts/check-ui-partitions.sh
ODR_TEST_STORAGE=/tmp sh scripts/check-ui-partitions.sh
ODR_TEST_STORAGE=/tmp ODR_CONSOLIDATE=1 ODR_MANAGE_SCANS=1 sh scripts/check-ui-recovery.sh
```

The engine script runs the ignored `disposable_` Rust tests. Install optional
engines to exercise their cases; unavailable optional adapters can be skipped
by the tests, so inspect the output before claiming full backend coverage.
Same-drive GUI checks set `ODR_TEST_STORAGE=/tmp` to exercise the image exception.

Test a packaged application with:

```sh
APPIMAGE_EXTRACT_AND_RUN=1 ODR_BINARY=dist/OpenDataRecovery-x86_64.AppImage \
  ODR_CONSOLIDATE=1 ODR_MANAGE_SCANS=1 sh scripts/check-ui-recovery.sh
```

`tests/fixtures/recovery.png` is the tracked, reproducible input for PNG carving
and storage tests. Its generation instructions are in [the fixture notes](../tests/fixtures/README.md).

## Consolidation measurements

The production benchmark is independent of the experimental spike:

```sh
python3 scripts/check-consolidation.py --runs 3 \
  --binary target/release/open-data-recovery \
  --output target/consolidation-check
```

The output directory must be new. Use Python 3.11 or newer. Fixture helpers live in
`scripts/consolidation_fixtures.py`; dependencies and methodology are documented
in [consolidation validation](consolidation-implementation.md). Historical
measurements remain in `docs/benchmarks/`.

The original experiment is tracked in [spikes/consolidation](../spikes/consolidation/README.md),
along with its [report and measurements](consolidation-spike.md). It documents the
tradeoffs assessed before implementation and is separate from application builds,
tests and release packages. Generated spike binaries and caches remain ignored.

## Documentation and screenshots

Keep workflow details in `docs/wiki/` and the README focused on getting started.
Record checks actually performed, including limitations, in
[validation results](validation.md). `sh scripts/capture-preview.sh` captures the
production application with a disposable ext2 image, real scans, isolated XDG
settings and a private display. It requires ImageMagick, xdotool, Xvfb, and the
system Python packages `python3-gi`, `python3-cairo` and `gir1.2-rsvg-2.0`. It
verifies the source hash after capturing all eight screenshots; no user data or
in-memory demonstration catalogue is used.
Package outputs go to ignored `dist/`; build outputs and temporary reports go
to ignored `target/`. Packaging does not install or publish a release.
