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
sh scripts/check-ui-advanced.sh
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

`tests/fixtures/recovery.png` is the tracked input for PNG carving and storage
tests. It is a deterministic 128 × 128 RGB image containing no user data.
Consolidation regression cases live in `src/engine/consolidation_tests.rs`.
See [consolidation](wiki/consolidation.md) for behavior and storage costs.

## Documentation and screenshots

Keep workflow details in `docs/wiki/` and the README focused on getting started.
Record checks actually performed, including limitations, in the local ignored
`docs/validation.md` file. Published check results are available in
[GitHub Actions](https://github.com/fossgap/open-data-recovery/actions).
Capture screenshots from the production application using disposable images,
isolated XDG settings and a private display. Verify the source hash afterward;
do not include user data or the in-memory demonstration catalogue.
Package outputs go to ignored `dist/`; build outputs and temporary reports go
to ignored `target/`. Packaging does not install or publish a release.
