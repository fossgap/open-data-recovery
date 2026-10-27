# Optional final consolidation

[Wiki home](README.md) · [Scan modes](scan-modes.md) · [Storage](storage.md) · [Recovery](recovery.md)

Enable **Scan dropdown → Custom… → Scan → Consolidate identical files when the scan finishes**. It is **unchecked by default** and available for Quick, Deep and Full. The CLI equivalent is `--consolidate`.

Consolidation compares what the scan found. It does not add another engine, recover missing findings, or run multiple carvers.

## Without consolidation

Each engine finding remains a separate result. Sharing one catalogue does not merge byte-identical results automatically. Selecting several identical findings can therefore recover several copies, subject to the filename-conflict option.

For example, suppose a Full scan finds the same bytes at these three paths:

| Displayed path | Engine |
| --- | --- |
| `Pictures/photo.jpg` | TSK |
| `Backup/photo-copy.jpg` | TSK |
| `photorec/recup_dir.1/f001.jpg` | PhotoRec |

The unconsolidated list contains all three rows.

## With consolidation

The final list contains **one group with three findings** in that example. A metadata name is preferred for display, such as `Pictures/photo.jpg`. Its **3 findings…** button exposes all three paths and engine records; searching `photo-copy.jpg` still finds the group.

Selecting the group recovers **one copy by default**. **Review recovery → Recover every recorded path** restores all three distinct recorded paths, including the generated PhotoRec name. If several engines reported exactly the same path, that path is restored once per group. Different contents at the same path remain different groups and use the normal destination-conflict rules.

The list is published only after the final grouping stage completes. It stays stable when reopened or recovered. Original findings, paths, engine versions and source addresses are retained. Grouping does not progressively hide or delete findings as engines run.

## How a match is established

1. Compare recorded lengths. A length appearing only once cannot have an exact-content duplicate and needs no content read for grouping.
2. For potentially matching lengths, use retained engine files or extract the required contents from the scan input.
3. Hash complete contents with SHA-256.
4. Byte-compare matching hashes before joining findings into a group.

Names, offsets and lengths alone do not establish a match. A hash match alone is also insufficient. Different original files with identical bytes can legitimately share one group, which is why all of their recorded paths are kept.

## Time and storage costs

Consolidation can substantially extend a scan, especially when many TSK findings need separate extraction. It is not a metadata-only database operation.

Existing verified artifacts are reused first. New unique extractions stay in `artifacts/consolidation/` for later recovery; a new extraction verified identical to retained content is discarded. **Original engine artifacts are kept**, including duplicates. A smaller displayed list therefore does not imply a smaller workspace.

Consolidation may reduce destination space when one copy per group is recovered, while increasing workspace space for content comparisons. Restoring every recorded path writes the additional copies again. Unique-size TSK findings remain metadata-only unless another operation extracts them; enabling this option does not guarantee that the entire source can be disconnected afterward.

See the [production timing/storage measurements](../consolidation-implementation.md) for all seven fixture cases and the raw results. Those measurements use RAM-backed filesystems and are not HDD/SSD estimates.

## Cancellation and verification failures

Cancellation, insufficient cache capacity or changed canonical content aborts unfinished grouping and leaves the original ungrouped findings available. Interrupted scans can reopen their raw catalogue; an actively running consolidation blocks selection until it finishes.

A finding that cannot be read completely for comparison, or whose extracted length differs from its recorded length, stays separate with a **Not verified** detail. The attempt reports partial verification. Unique-size findings skipped by the comparison stage have not thereby been content-verified.

Recovery checks a saved group content hash whenever one was recorded, even if optional output rereading is disabled. A changed saved content file produces an explicit recovery failure; the application does not silently change group membership or substitute a different result.

```sh
open-data-recovery --scan /media/source/disk.img \
  --workspace /media/recovery/scans --mode full --consolidate

open-data-recovery --list /media/recovery/scans/SESSION
open-data-recovery --findings /media/recovery/scans/SESSION --id GROUP_ID
```

`GROUP_ID` comes from the final list. Loading an older unconsolidated scan does not consolidate it automatically; the option applies to a new scan attempt.
