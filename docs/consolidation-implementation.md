# Optional final consolidation: implementation validation

Validated on 2026-10-27 using the production `--consolidate` option. The option is unchecked by default in Custom Scan. It publishes one stable list after the scan engines finish, with exact content grouping, retained findings and paths, and one recovered copy per group unless **Recover every recorded path** is selected.

## Method

The release was built against Debian Bookworm's GTK 4.8 and glibc 2.36. `scripts/check-consolidation.py` ran seven cases, three times each, with consolidation off and on: **42 actual scans**. Both sides use the same release binary. The deterministic fixture helpers are maintained in `scripts/consolidation_fixtures.py`; no prototype code is required.

Sources were freshly created regular ext2, FAT16 and NTFS images on `/tmp`; workspaces used the separate `/dev/shm` filesystem. Both are RAM-backed, with warm data. These are measured fixture costs, **not HDD/SSD timing forecasts**. Source images were hashed before and after all runs and were unchanged. No physical device was scanned or formatted.

Every comparison independently reread all findings: lazy TSK findings through `icat`, other findings from their engine artifacts. Checks compared raw discovery records between off/on runs, complete content digests, group membership, all recorded paths, canonical content, SQLite integrity, and CLI reopening/listing. All cases had **zero false merges, missed identical findings, lost paths or unverified findings**.

## Results

The table below uses medians of three runs. Off/on times include the entire scan; the final-stage column measures consolidation alone. Workspace figures are allocated filesystem bytes in MiB after completion, including original engine artifacts, databases and any new cache. They exclude the source image and later recovery destinations. Temporary comparison files can require additional space while the scan runs.

| Case | Off (s) | On (s) | Final stage (s) | Findings → groups | Workspace off → on (MiB) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Unique-size ext2 · Quick | 0.678 | 0.712 | 0.008 | 80 → 80 | 0.086 → 0.113 |
| Small PNG ext2 · Quick | 0.685 | 6.109 | 5.427 | 657 → 597 | 0.215 → 5.137 |
| Small PNG ext2 · Full | 0.821 | 6.450 | 5.647 | 1317 → 600 | 64.688 → 65.031 |
| Large PNG FAT16 · Full | 0.904 | 1.481 | 0.580 | 80 → 32 | 192.211 → 192.246 |
| Large PNG FAT16 · Deep | 0.861 | 0.935 | 0.081 | 40 → 32 | 192.195 → 192.230 |
| PNG NTFS · Quick | 1.022 | 1.301 | 0.272 | 119 → 24 | 19.867 → 19.910 |
| PNG NTFS · Full | 1.076 | 1.341 | 0.262 | 149 → 24 | 49.543 → 49.586 |

[Raw measurements and exact binary/engine versions](benchmarks/consolidation-implementation-2026-10-27.json).

## Interpretation

Small Quick scans must extract matching-size TSK findings to compare their actual bytes. Unique new contents stay cached for recovery, increasing workspace use. A finding whose size appears only once needs no extraction. Quick mode is therefore no longer necessarily metadata-only when consolidation is enabled.

Full and Deep scans reuse verified content already retained by their engines. The NTFS Quick fixture also reuses content saved by its metadata engines. Original engine artifacts are preserved; consolidation does not reclaim that space. Only newly extracted duplicate temporary files are discarded. The additional retained space in these reuse cases is principally the grouping/path index and manifest/log records.

Short extraction processes wake the supervisor on exit instead of always paying a 50 ms polling delay. Consolidation preserves the full recovery view, provenance, failure reporting and selection semantics. Neither this optimization nor consolidation changes source write protection or the engine plan.

The small ext2 Quick fixture returns 657 findings representing 597 unique contents; TSK misses three of the original unique files. Full scanning finds the additional contents and produces 600 groups. Consolidation preserves the actual findings; it does not manufacture results absent from every engine. PhotoRec can retain artifacts outside the PNG filter, so workspace totals can exceed the size of the listed PNG contents.

## Failure handling and recovery checks

Unit checks cover content differences, empty files, unique sizes, missing/truncated artifacts, byte changes during finalization, cancellation rollback, insufficient cache capacity, bounded paging of a 257-member group, alias search, default one-copy recovery, every-path recovery, and saved-content tampering even with output rereading disabled. Active scans block selection; interrupted scans release their lock and retain usable raw findings. A stale raw view cannot select IDs after grouping has finished.

Real-engine integration tests recover identical deleted files from ext2, reopen the grouped scan after removing its source image, and restore every recorded path from retained content. Existing clone, E01, NTFS reconstruction and carver integrations also pass. GTK checks exercise the default-off checkbox, grouped findings dialog, selection, destination validation and every-path recovery. The Custom Scan dialog still fits at 600 × 480 with default GTK styling.

Reproduce the measurements with:

```sh
python3 scripts/check-consolidation.py --runs 3 \
  --binary target/portable-release/open-data-recovery \
  --output target/consolidation-check
```

The output directory must not already exist. The tool needs installed recovery engines, `mkfs.ext2`, `mkfs.fat`, `mkntfs`, `debugfs`, `mcopy`, `mdel`, `ntfscp`, and the NTFS userspace library used by the disposable-image fixture builder. Optional packaged engines are picked up from `target/engine-package/usr/bin`. The retained JSON report records the exact application binary hash, engine versions, fixture hashes, individual timings and storage figures.
