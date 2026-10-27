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

## Retained results and saved-scan deletion — 2026-10-27

The GTK 4.8 release build passes **29 unit tests**. All **five real-engine integration tests** passed again after adding session lifetime locks. New deletion tests cover active readers, folder replacement, changed scan identity, symbolic links, source overlap, mount boundaries, partial-deletion retries, and preservation of other snapshots and external files, including hard-linked content.

GtkBuilder validation and demo/production smoke checks passed. The extended production GUI workflow passed with consolidation both disabled and enabled. It adds another image source, switches sidebar sources in both directions, and checks that the open scan, selection, search and preview survive. Recovery still uses the open scan. Cancelling deletion keeps its folder; confirming deletion removes it and clears the open results, while recovered files remain and the source hash is unchanged. The workflow also checks that a pending page refresh cannot cancel opening a saved scan.

The final rebuilt AppImage passed the consolidated source-switching, recovery and deletion workflow. Saved Scans and the deletion confirmation were visually checked with native GTK styling. Debian, RPM, Arch and AppImage packages were rebuilt; native package binaries match the tested portable release, and all seven package checksums in `dist/SHA256SUMS` were refreshed and verified. Native packages were not installed on the host, and no physical recovery sources or user snapshots were used in these tests.

Reproduce the management workflow with `ODR_MANAGE_SCANS=1 scripts/check-ui-recovery.sh`; add `ODR_CONSOLIDATE=1` for grouped results. For the packaged application, also set `APPIMAGE_EXTRACT_AND_RUN=1 ODR_BINARY=dist/OpenDataRecovery-x86_64.AppImage`.

## File menu and recovery destination row — 2026-10-27

The preceding implementation was committed as `7ca36a6`. The File menu's Saved scan details entry was removed, and the recovery destination field and Browse destination button now share one row. The recovery dialog was visually checked at its existing 590 × 680 size with native GTK styling.

All 29 unit tests, GtkBuilder validation and demo/production smoke checks passed. Rebuilt Debian, RPM and Arch package binaries match the portable release; the rebuilt AppImage passed the consolidated recovery and saved-scan management workflow. All seven package checksums were refreshed and verified. The five broader engine integration cases were not rerun for these layout-only changes.

## Read-only TestDisk partition discovery — 2026-10-27

The development and GTK 4.8 portable release builds pass **37 unit tests**. All **six real-engine integration tests** passed on disposable regular image files. New cases cover MBR/CHS and GPT/LBA parsing, 512/4096-byte sector arithmetic, malformed/incomplete output, range bounds, exact-range deduplication, overlap reporting, the fixed read-only command list, invalid/incomplete reports, early cancellation, separate-storage preflight, legacy source loading, and comparison of distinct partitions on the same disk.

The real TestDisk 7.2 fixture embeds a 16 MiB ext2 filesystem at byte 2,097,152 in a 32 MiB disk image without a partition-table entry. Auto, explicit MBR Quick Search, MBR Deeper Search, and explicit GPT Quick Search all find the expected range. The independent filesystem probe verifies its ext2 header. Source hashes remain unchanged; changing a sampled partition byte invalidates the recorded target. Scanning retains exactly 16 MiB, and recovery and rescan both succeed after disconnecting the original image.

`scripts/check-ui-partitions.sh` passed with both the development binary and rebuilt AppImage. Its CLI checks cover discovery, report-based scanning with existing files included, listing, byte-exact recovery of an existing and a deleted file, an empty search, and rejection of invalid table/partition choices. Its GTK workflow covers cancellation, search, report loading through the chooser's production handler, preservation of the previous catalogue/selection/search/preview, virtual-target selection, replacement of stale target fingerprints, source refresh, fixed range fields, scanning and verified recovery. The script supplies the report path to the handler; it does not synthesize native/portal file-chooser responses.

All GtkBuilder layouts and both general GTK smoke tests passed. The portable release also passed the existing consolidation, recovery, source-switching and saved-scan deletion workflow. The discovery dialog was visually checked with native GTK styling, including overlapping verified/unverified candidates and long report paths. The real native report chooser was separately exercised with mouse/keyboard input in an isolated display/session with desktop portals disabled for that check; accepting the report populated the candidates correctly.

Debian, RPM, Arch and AppImage packages were rebuilt. Extracted native package binaries and the AppDir binary match the tested portable release; all seven application/engine package checksums were refreshed and verified. No package was installed on the host or published. Tests do not establish success for every damaged filesystem, TestDisk version or physical-device sector configuration; 4096-byte sector parsing is covered by unit fixtures rather than a physical-device test.

## Image files sharing output storage — 2026-10-27

The preceding TestDisk implementation was committed as `6238f19` before this change. The GTK 4.8 portable release passes **44 unit tests**; all **seven real-engine integration cases** passed. New unit checks cover automatic image eligibility, unchanged device/folder/discovered-device restrictions, every E01 segment's path, changed source types, nonregular/symlink paths, disconnected image records, unknown ancestry, output collisions with hard links/symlinks, and snapshot deletion preserving the image and recovered files.

The new integration case places its raw image, workspace and recovery destinations on the **same filesystem**. Quick, Deep and Full scans recover known PNG bytes; Full also consolidates the metadata and carving findings. The image contains a deleted ext2 file and a separate contiguous carving copy, so this is an output-placement test rather than a fragmented-file reconstruction claim. A segmented E01 image is then scanned on that filesystem, every input segment is compared byte-for-byte, and recovery and restart succeed after disconnecting all original segments. The raw image's SHA-256 remains unchanged.

Both GTK workflows passed with `ODR_TEST_STORAGE=/tmp`, including the rebuilt AppImage: partition discovery/report loading/range scanning/recovery, and consolidation/source switching/recovery/saved-scan deletion. Output files match their original payloads and source image hashes remain unchanged. GtkBuilder validation and both general smoke tests passed. The revised image-specific recovery wording fits the existing 590 × 680 dialog; Custom Scan remains 600 × 480 with native GTK styling.

Debian, RPM, Arch and AppImage packages were rebuilt. Native package binaries and AppDir match the tested portable release, and `dist/SHA256SUMS` was refreshed and verified for all seven packages. No package was installed on the host or published. Physical-device policy checks use disposable descriptors; no physical recovery device was scanned or written. Allowing an image to share storage is not a hardware-health assessment, and unknown network/overlay storage remains unsupported.

Reproduce the same-filesystem GUI cases with `ODR_TEST_STORAGE=/tmp sh scripts/check-ui-partitions.sh` and `ODR_TEST_STORAGE=/tmp ODR_MANAGE_SCANS=1 ODR_CONSOLIDATE=1 sh scripts/check-ui-recovery.sh`. Add `APPIMAGE_EXTRACT_AND_RUN=1 ODR_BINARY=dist/OpenDataRecovery-x86_64.AppImage` to test the package. The new engine case is `cargo test --locked disposable_same_drive_image_scan_modes_and_segmented_ewf -- --ignored --nocapture`.

## Header, consolidation tooltip and recovery wording — 2026-10-27

The image-storage exception was committed as `b33bfd6` before these UI changes. The header's extra source label was removed, the consolidation tooltip now ends with the requested review benefit, and the recovery dialog omits its redundant destination heading and the overwrite sentence. The main window, hovered tooltip and recovery dialog were visually checked with native GTK styling.

All **44 unit tests**, GtkBuilder validation and demo/production smoke checks passed. The rebuilt AppImage passed the same-filesystem consolidated recovery and saved-scan management workflow, including verified recovery output. The seven broader engine integration cases were not rerun for these text and layout changes. Debian, RPM, Arch and AppImage packages were rebuilt; extracted native package binaries match the tested portable release, and all seven package checksums were refreshed and verified.

## First release preparation — 2026-10-27

The local spike directory, prototype report and prototype measurements were removed from Git tracking and ignored, while retaining their working copies. Production consolidation checks now use independent helpers in `scripts/consolidation_fixtures.py`; their function bodies match the previously validated helpers. A reproducible PNG in `tests/fixtures/` replaces the missing root-level image used by three integration tests.

A fresh Debian Bookworm Docker build passes **44 unit tests**, with all **seven real-engine integration tests** separately passing on disposable images. Docker's build and documented runtime/devcontainer configuration use tmpfs for test fixtures because source validation rejects overlay storage. The relocated consolidation check passed **14 scans**: seven off/on cases, one repetition each, with exact grouping, preserved findings and paths, successful reopening, and unchanged source hashes. This functional check does not replace the historical three-run timing table; its local output is under `target/release-prep/consolidation-check/`.

All GtkBuilder layouts and both GTK smoke checks passed against GTK 4.8. The new SVG was rendered at desktop icon sizes, checked on light and dark backgrounds, and verified in the application header. The README screenshot uses fictional demo data. Local documentation links and README anchors were checked.

Debian, RPM, Arch and AppImage packages were rebuilt with the new logo, README and GPL-3.0-only license. Extracted native package contents and AppDir match the tested portable binary and current assets; RPM/Arch license metadata and the Debian copyright file were verified. All seven package checksums were refreshed and verified. The final AppImage passed the same-filesystem consolidated recovery and saved-scan management workflow, with verified output and unchanged source contents. Packages were not installed or published.
