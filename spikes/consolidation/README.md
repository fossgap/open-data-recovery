# Final consolidation benchmark

This is a standalone experimental executable and fixture runner. It is not linked
into Open Data Recovery and does not enable or implement a GUI feature. Production
source files, existing saved scans, and packages are not modified by running it.

The recorded 2026-10-27 spike predates the production feature. This executable
includes the current `process.rs`; the subsequent exit-notification improvement
changes its timings. Use the original runner from commit `2877180` to reproduce
the historical supervisor overhead, or `scripts/check-consolidation.py` to test
the implemented feature with consolidation off and on.

## Run

From the repository root, build with the local Cargo dependency cache:

```sh
CARGO_TARGET_DIR=target/consolidation-spike-build cargo build --release --locked --offline --manifest-path spikes/consolidation/Cargo.toml
python3 spikes/consolidation/run.py --runs 3 --cache-policy all --output target/consolidation-all
python3 spikes/consolidation/run.py --runs 3 --cache-policy reuse --output target/consolidation-reuse
```

Each output directory must not already exist. Use `--cases` to select a
comma-separated subset of `unique-quick,small-quick,small-full,large-full,large-deep,ntfs-quick,ntfs-full`.
`--binary` selects the application executable. `--prototype` selects the spike
executable. The defaults use `target/portable-release/open-data-recovery` and the
release executable built above.

The runner needs write access to `/tmp` and `/dev/shm`, which must have different
filesystem device IDs. It creates regular image files only; it never opens real
block devices or creates a kernel/FUSE mount. Fixture generation uses `mkfs.ext2`,
`debugfs`, `mkfs.fat`, `mcopy`, `mdel`, `mkfs.ntfs`, `ntfscp`, and the installed
`libntfs-3g`. Its NTFS helper calls the library on its newly created image to unlink
fixture files before scanning. The helper's ABI follows the upstream
[directory API](https://github.com/tuxera/ntfs-3g/blob/2022.10.3/include/ntfs-3g/dir.h)
and [volume API](https://github.com/tuxera/ntfs-3g/blob/2022.10.3/include/ntfs-3g/volume.h).

Scan dependencies are the installed application engines; the runner prepends
`target/engine-package/usr/bin` to PATH to include the packaged optional engines.
The NTFS cases need RecuperaBit there or elsewhere in PATH. The application must
support its existing Landlock source protection. No checks are disabled.

## Experiment

Every repetition starts a fresh actual CLI scan with automatic engine selection,
the chosen Quick/Full/Deep mode, and a PNG or BIN extension filter. All fixture
files are deleted before scanning; `include_existing` and cloning remain off.
The source image is made read-only after fixture generation.

The final stage runs immediately after that scan. The prototype:

1. Opens the checkpointed candidate catalogue read-only, groups rows by recorded
   length, and skips content reads for lengths with a single candidate.
2. Extracts lazy TSK candidates with the same `icat` arguments and unchanged
   sandboxed process runner used by production recovery, including its 50 ms
   child-completion polling interval. It syncs extracted files and checks length.
3. Hashes complete contents using SHA-256, then byte-compares matching hashes.
   Hash matches alone do not merge findings.
4. Preserves every candidate ID and its original path/provenance in the unchanged
   catalogue; writes a separate SQLite membership table and canonical-content
   reference, committing and renaming the result at completion.

Two cache policies are measured:

- **all:** retain every newly extracted TSK candidate, including identical copies.
- **reuse:** process existing artifacts first and discard a new extraction only
  after confirming it is byte-identical to retained content. Retain newly
  extracted unique contents. A group can use a TSK original name while its content
  comes from a verified PhotoRec or other metadata artifact.

Neither policy deletes or rewrites any original engine artifact. The second
policy reduces *additional* cache use; it does not reclaim space already occupied
by duplicate engine outputs.

Both policies preserve all original file paths. Distinct paths containing the same
bytes share a content group; a group count is not a count of original filenames.

## Measurements and checks

`results.json` includes each repeat's scan time, added finalization time, total,
retained logical/allocated workspace bytes, process/hash/comparison times, I/O
byte counts, engine counts, fixture hashes, versions, and machine details.
`allocated_bytes` uses `st_blocks * 512`; `logical_bytes` sums regular-file lengths.
The reuse prototype also measures peak newly extracted file-cache bytes after
each extraction. That peak excludes the small SQLite transaction/journal and
general process memory; it is not a total peak RAM measurement.

Preparation and independent post-run verification are outside the timed interval.
Total time is the measured scan plus measured finalization of that exact scan,
not a separately estimated or simulated scan. The finalization measurement includes
starting the standalone executable. Temporary images/workspaces are cleaned up;
logs, result databases, and compact measurements remain in the chosen output.

Post-run checks independently hash actual candidate contents, verify every group
membership and canonical content reference, reject false merges and missed exact
duplicates, verify known named files against fixture bytes, check SQLite integrity,
and confirm the original catalogue, manifest, workspace footprint, and source image
hashes are unchanged. Unrecovered source contents are reported separately from
grouping correctness: a completed engine run need not find every deleted file.

These are synthetic, RAM-backed, warm-data microbenchmarks, not HDD/SSD throughput
estimates. Equal-sized PNG fixtures deliberately exercise the expensive length
bucket. They do not establish costs on damaged media, millions of files, changing
mounted sources, E01 input, cloned sources, or every supported filesystem. Production
implementation would also need cancellation, disk-full handling, schema migration,
selection/recovery integration, and UI validation; this spike makes no claim to
implement those features.
