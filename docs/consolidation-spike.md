# Final consolidation spike — 27 October 2026

**Measured result:** final consolidation can be cheap when contents already exist as engine artifacts, but is expensive for many lazy TSK findings with matching sizes. Reusing byte-confirmed existing artifacts avoids most additional retained file storage. It does not avoid reading/extracting those TSK findings.

**Historical scope:** these measurements were collected before implementation, using the application at commit `2877180`. That spike changed no production source, GUI option, saved-scan format, or package. Its standalone executable remains separate from the subsequently approved optional-consolidation feature. Current implementation checks use `scripts/check-consolidation.py`.

## Measurements

These are **synthetic, warm-data, RAM-backed microbenchmarks**, not predictions for a real HDD, SSD, damaged drive, or a terabyte-scale scan. Both source images (`/tmp`) and output workspaces (`/dev/shm`) use different tmpfs filesystems. The guest has 6 vCPUs reporting an Intel i9-12900KF and about 11.7 GiB RAM, running Linux 6.16.8. The existing release application and installed real recovery engines perform every scan.

Each result below is the median of **three fresh scans**. Current time includes the actual CLI scan, engine probing/diagnostics and saved catalogue. Consolidated time is that same scan plus the measured final stage. Fixture generation and independent correctness checks are excluded. The added stage includes launching the standalone spike executable. Medians are computed separately for each column; per-run totals are paired sums.

The main table uses the **reuse** cache policy: compare complete contents with SHA-256 and byte-for-byte verification; reuse an existing identical artifact; retain a newly extracted file only if no retained identical content exists. All original artifacts and candidate records remain unchanged.

| Scenario | Current scan | Added final stage | Scan + final stage | Retained workspace: current → consolidated | Findings → content groups |
|---|---:|---:|---:|---:|---:|
| Different-size files · Quick (ext2) | 0.908 s | +0.005 s | 0.914 s | 0.086 → 0.102 MiB | 80 → 80 |
| Many small files · Quick (ext2) | 0.895 s | +34.127 s | 35.042 s | 0.215 → 5.008 MiB | 657 → 597 |
| Many small files · Full (ext2) | 1.098 s | +34.281 s | 35.378 s | 64.688 → 64.859 MiB | 1,317 → 600 |
| Large files · Full (FAT16) | 1.063 s | +2.329 s | 3.408 s | 192.211 → 192.234 MiB | 80 → 32 |
| Large files · Deep (FAT16) | 0.943 s | +0.091 s | 1.034 s | 192.195 → 192.219 MiB | 40 → 32 |
| Multiple metadata engines · Quick (NTFS) | 1.311 s | +1.549 s | 2.866 s | 19.867 → 19.891 MiB | 119 → 24 |
| Metadata + carving · Full (NTFS) | 1.369 s | +1.589 s | 2.958 s | 49.543 → 49.566 MiB | 149 → 24 |

Workspace means allocated filesystem bytes (`st_blocks × 512`) after completion, including the catalogue, engine logs/reports/artifacts, new retained cache and group index. It excludes the source image and any later user recovery destination. Logical file lengths and all individual runs are in the JSON data. MiB = 1,048,576 bytes; KiB = 1,024 bytes.

Content groups preserve all known paths and engine findings. Two distinct original paths with identical contents share a group; the group count is therefore not a count of original filenames. Consolidation alone does not recover additional bytes.

## Extra retained and temporary storage

| Scenario | Extra retained workspace | Peak new extracted-file cache during finalization |
|---|---:|---:|
| Different-size files · Quick (ext2) | 16 KiB | 0 KiB |
| Many small files · Quick (ext2) | 4,908 KiB | 4,784 KiB |
| Many small files · Full (ext2) | 176 KiB | 8 KiB |
| Large files · Full (FAT16) | 24 KiB | 3,076 KiB |
| Large files · Deep (FAT16) | 24 KiB | 0 KiB |
| Multiple metadata engines · Quick (NTFS) | 24 KiB | 196 KiB |
| Metadata + carving · Full (NTFS) | 24 KiB | 196 KiB |

The cache peak is measured after each completed extraction and includes retained new content plus the current temporary file. It is separate from the retained-workspace figure and excludes the small SQLite journal/transaction and process RAM. In the large Full case, the extra file cache reaches about 3.004 MiB and returns to zero; the retained addition is the 24 KiB group index. In small Quick, unique newly extracted files remain, so the retained addition is about 4.793 MiB.

A first conservative pass retained every newly extracted TSK file, including already-confirmed copies. The following differences were also measured across three repeats:

| Scenario | Extra workspace: retain every new extraction | Extra workspace: reuse identical retained content |
|---|---:|---:|
| Different-size files · Quick (ext2) | 0.016 MiB | 0.016 MiB |
| Many small files · Quick (ext2) | 5.211 MiB | 4.793 MiB |
| Many small files · Full (ext2) | 5.227 MiB | 0.172 MiB |
| Large files · Full (FAT16) | 120.172 MiB | 0.023 MiB |
| Large files · Deep (FAT16) | 0.016 MiB | 0.023 MiB |
| Multiple metadata engines · Quick (NTFS) | 5.566 MiB | 0.023 MiB |
| Metadata + carving · Full (NTFS) | 5.566 MiB | 0.023 MiB |

The reuse pass additionally stores canonical-content paths in its index; that index overhead is included. The main timing comparison uses paired scans from the reuse pass. Neither version removes duplicate data already retained by the original engines.

## Why the time differs

TSK scan findings reference file metadata; their contents are normally extracted during recovery. Exact final consolidation moves some of that extraction into scan completion. The spike uses the unchanged production sandboxed process runner and the same `icat` arguments as recovery. Its child-completion loop polls every 50 ms.

| Scenario | TSK files extracted | Extraction time | Hashing time | Byte-comparison time |
|---|---:|---:|---:|---:|
| Different-size files · Quick (ext2) | 0 | 0.000 s | 0.000 s | 0.000 s |
| Many small files · Quick (ext2) | 657 | 33.832 s | 0.238 s | 0.029 s |
| Many small files · Full (ext2) | 657 | 33.800 s | 0.069 s | 0.355 s |
| Large files · Full (FAT16) | 40 | 2.065 s | 0.172 s | 0.079 s |
| Large files · Deep (FAT16) | 0 | 0.000 s | 0.076 s | 0.011 s |
| Multiple metadata engines · Quick (NTFS) | 29 | 1.485 s | 0.018 s | 0.038 s |
| Metadata + carving · Full (NTFS) | 29 | 1.501 s | 0.025 s | 0.049 s |

In the small-file cases, extraction accounts for about 33.8 seconds of the approximately 34-second final stage. This is chiefly a cost of reusing the current per-file extraction path, not a measured lower bound for every possible consolidation implementation. A future implementation should address that path before shipping. This spike does not quantify a faster replacement.

For the large Full case, the stage extracts/writes 120.05 MiB, hashes 240.10 MiB, and reads another 288.12 MiB for byte comparisons. Releasing confirmed duplicate temporary files saves retained space but does not remove that I/O. RAM-backed storage makes these byte-processing costs much smaller than they may be on a physical drive.

## Fixtures and actual engines

All data was generated deterministically. Files were written and then deleted before scanning, without allocating new payload files after deletion. No real user source or block device was used. Cloning and existing-file inclusion were off. Metadata and carver selection remained automatic; scans used PNG or BIN extension filters.

| Fixture | Image size | Deleted files | Payload bytes | Distinct contents |
|---|---:|---:|---:|---:|
| unique (ext2) | 16 MiB | 80 | 0.424 MiB | 80 |
| small (ext2) | 64 MiB | 660 | 4.424 MiB | 600 |
| large (fat16) | 192 MiB | 40 | 120.051 MiB | 32 |
| ntfs (ntfs) | 32 MiB | 30 | 5.635 MiB | 24 |

- `unique`: 80 binary files with distinct lengths, 4,096–7,019 bytes. The size filter avoids all extraction/hashing.
- `small`: 600 distinct valid 48×48 RGB PNGs plus 60 exact copies at other paths, each 7,028 bytes. Deliberately exercises the expensive same-size bucket.
- `large`: 32 distinct valid 1024×1024 RGB PNGs plus 8 copies, each 3,147,060 bytes, on contiguous FAT16 storage.
- `ntfs`: 24 distinct valid 256×256 RGB PNGs plus 6 copies, each 196,947 bytes. Deleted through libntfs-3g on a regular image before making it read-only.

| Scenario | Actual candidate counts by engine |
|---|---|
| Different-size files · Quick (ext2) | tsk: 80 |
| Many small files · Quick (ext2) | tsk: 657 |
| Many small files · Full (ext2) | tsk: 657; photorec: 660 |
| Large files · Full (FAT16) | tsk: 40; photorec: 40 |
| Large files · Deep (FAT16) | photorec: 40 |
| Multiple metadata engines · Quick (NTFS) | tsk: 29; ntfsundelete: 30; recuperabit: 30; testdisk: 30 |
| Metadata + carving · Full (NTFS) | tsk: 29; ntfsundelete: 30; recuperabit: 30; testdisk: 30; photorec: 30 |

The NTFS automatic plan ran TSK, ntfsundelete, RecuperaBit and TestDisk sequentially; Full additionally ran PhotoRec. ext2/FAT16 Quick used TSK, Full used TSK then PhotoRec, and Deep used PhotoRec alone.

TSK found 657 of 660 named small-fixture files and 29 of 30 NTFS files. Small Quick therefore contained 597 of the 600 distinct original contents; Full found all 600 through PhotoRec. The other NTFS engines supplied the missing finding. These are scan-discovery differences, recorded before grouping; consolidation preserved every actual finding.

PhotoRec scans all enabled file families before the application applies its extension filter. On these synthetic images it also retained `.dovecot` artifacts outside the displayed PNG catalogue. For example, the large Full case retained 918 such artifacts totaling 71.72 MiB. They are included in the workspace measurements, along with reports/logs; all per-extension artifact totals are in the raw data.

Pilot runs were used to correct fixture preparation and are excluded from the tables: an NTFS fixture with no deleted files caused ntfsundelete to exit 1; large PNGs on ext2 were not reconstructed by PhotoRec in that pilot. The published cases all completed without engine errors.

## Verification and repeatability

Across the two passes: **42 completed scan/finalization runs, 14,652 independently verified candidate memberships, zero false merges, zero missed exact duplicates among actual findings, and zero changed source-image hashes.** All existing catalogue and manifest hashes and workspace footprints remained unchanged. Every reused canonical content file was independently checked in the reuse pass. Same-size/different-content files remained separate; exact copies were grouped.

Finalization time ranges across the three reuse runs:

| Scenario | Minimum | Median | Maximum |
|---|---:|---:|---:|
| Different-size files · Quick (ext2) | 0.004 s | 0.005 s | 0.007 s |
| Many small files · Quick (ext2) | 34.054 s | 34.127 s | 34.187 s |
| Many small files · Full (ext2) | 34.218 s | 34.281 s | 34.321 s |
| Large files · Full (FAT16) | 2.311 s | 2.329 s | 2.345 s |
| Large files · Deep (FAT16) | 0.087 s | 0.091 s | 0.103 s |
| Multiple metadata engines · Quick (NTFS) | 1.548 s | 1.549 s | 1.568 s |
| Metadata + carving · Full (NTFS) | 1.559 s | 1.589 s | 1.599 s |

Original source, application-binary and prototype-binary hashes, engine versions, fixture properties, per-run timings, allocated/logical sizes, byte counters and verification outcomes are saved in [the measurements JSON](benchmarks/consolidation-2026-10-27.json). The full per-file fixture manifests and CLI scan logs remain under `target/consolidation-benchmark-2026-10-27/` and `target/consolidation-reuse-benchmark-2026-10-27/`; temporary images and workspaces were removed.

Reproduction commands and prototype limitations are in [spikes/consolidation/README.md](../spikes/consolidation/README.md). The spike remains separate from the application. Cancellation, disk-full behavior, damaged or changing sources, million-file scalability, schema migration, and GUI/recovery integration are outside this experiment.

## Decision supported by this spike

Keep the proposed Custom Scan option **unchecked by default**. If approved for implementation, use byte-confirmed artifact reuse and preserve all provenance/paths. Optimize the extraction path before shipping: with the current runner, hundreds of small same-size TSK findings can add tens of seconds even when their total content is only a few MiB. Existing-artifact cases need little extra retained storage, while metadata-only cases can require caching the unique contents being verified.
