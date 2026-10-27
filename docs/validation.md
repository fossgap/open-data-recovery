# Implementation validation

Validated on 2026-10-26. The release binary was built against Debian Bookworm's GTK 4.8 and glibc 2.36. Integration checks used regular disposable image files and separate temporary filesystems; no physical recovery source was scanned or formatted.

| Check | Result |
| --- | --- |
| Rust tests | 13 passed; 4 integration tests run separately |
| Disposable-image integration tests | All 4 passed |
| GtkBuilder layouts | All validated |
| Demo and production GTK smoke checks | Passed with the release binary |
| Production GUI recovery | Passed with the development binary and packaged AppImage |
| Release packaging | Debian, RPM, Arch and AppImage artifacts built locally |

Integration cases cover deleted ext2 content via TSK, ddrescue cloning, reopening and rescanning after disconnecting the original source, byte-exact recovery, PhotoRec/Scalpel/Foremost/Digler/Magicrescue carving, compound NTFS metadata addresses, RecuperaBit reconstruction, E01 normalization, and ext4rescue with a retained clone and map. Recovery checks compare known bytes and recorded SHA-256 verification.

The GUI recovery check opens a saved scan, selects a candidate, validates a separate destination, confirms recovery options, runs recovery, and checks the resulting file and audit record. The AppImage also verifies interaction with host recovery tools while using its bundled GTK runtime.

These checks validate specific fixtures and workflows. They do not establish recovery success for every damaged filesystem or installed backend version, and native packages have not been installed into the host system.

Reproduce with `scripts/check.sh`, `scripts/check-engines.sh`, and `scripts/check-ui-recovery.sh`. Optional integration engines must be installed or present on `PATH`; the complete run included the locally packaged RecuperaBit, Digler and ext4rescue builds. See [packaging](packaging.md) for testing a packaged binary.

## System theme and compact dialog revision — 2026-10-27

The prior implementation was checkpointed in commit `74fbf52` before these UI changes. All 13 unit tests passed again in the GTK 4.8 release build. GtkBuilder validation and existing demo/production smoke checks passed with native GTK styling, including dark and high-contrast checks. All three Custom Scan tabs were visually checked at 600 × 480; scrollable tab contents accommodate larger fonts without the earlier sizing warning.

Debian, RPM, Arch and AppImage application packages were rebuilt and their checksums refreshed. The final AppImage passed both UI smoke checks and the complete disposable-image recovery workflow with verified output. The earlier four engine integration results remain applicable to the unchanged recovery engine code.

## Optional final consolidation — 2026-10-27

The GTK 4.8 release build passes **24 unit tests**; all **five real-engine integration tests** were run separately and passed. New checks cover exact grouping, retained provenance, alias searches, distinct-path recovery and space accounting, content tampering, bounded member paging, insufficient cache capacity, cancellation and interrupted-scan selection. The real consolidation test restores both deleted ext2 paths after removing the original source image.

The final release passed **42 off/on scans** across seven ext2, FAT16 and NTFS cases, with three repetitions per case. Independent content checks found no false merges, missed identical findings or lost paths. Source image hashes were unchanged. The complete timing/storage table, method, limitations and raw data are in [consolidation implementation validation](consolidation-implementation.md). These timings use separate RAM-backed filesystems and do not forecast HDD/SSD performance.

GtkBuilder validation, demo/production smoke checks and the real GTK consolidation workflow passed. The workflow opens a saved grouped scan, inspects its findings, selects the group, validates a separate destination, enables every-path recovery and verifies both resulting files. Both the portable release and the rebuilt AppImage passed this workflow. The new checkbox defaults off, and the Custom Scan dialog was visually checked at 600 × 480 with default GTK styling.

Debian, RPM, Arch and AppImage application packages were rebuilt in `dist/`; the optional engine packages are unchanged. Native package binaries were extracted and compared with the tested portable release, whose SHA-256 is recorded in the benchmark JSON. `dist/SHA256SUMS` covers all seven application/engine packages. Packages were not installed on the host or published.
