# Quick, Deep and Full scans

[Wiki home](README.md) · [Engine roles](engines.md) · [Consolidation](consolidation.md) · [Recovery](recovery.md)

**One recovery engine runs at a time, but a scan can run several engines sequentially.** There is no parallel multi-engine scanning or multi-carver sequence.

| Mode | File-recovery sequence | Typical saved data |
| --- | --- | --- |
| Quick | Applicable metadata engine sequence | Catalogue and addresses from TSK; actual extracted files from other metadata engines |
| Deep | One chosen content engine | Carved file contents, or reports when bulk_extractor is explicitly selected |
| Full | Quick's metadata sequence, followed by one chosen content engine | Metadata findings plus extracted/carved contents |

Quick describes the method, not a promise of low storage use or a fixed duration. Deep does not uniquely own content extraction: metadata engines can also extract files, and consolidation can require more extraction.

## Automatic metadata selection

For Quick and Full, **Auto** constructs the following sequence from the identified source filesystem. Optional additional engines run only if installed, in the order shown.

| Source filesystem | Automatic metadata sequence |
| --- | --- |
| ext2 | TSK |
| ext3 / ext4 | TSK → ext4magic, if installed → extundelete, if installed |
| NTFS | TSK → ntfsundelete, if installed → RecuperaBit, if installed → TestDisk, if installed |
| btrfs | btrfs restore |
| FAT12/16/32 / exFAT | TSK |

TSK is required for the TSK-led sequences; Auto does not replace it with `tsk_recover` when it is missing. btrfs restore is required for the automatic btrfs sequence. `tsk_recover` and `ext4rescue` are implemented **custom choices**, not additional Auto steps.

For a mounted-folder source, automatic additional metadata engines are not appended; the primary metadata adapter's results are filtered to the selected folder. Deep and Full require selecting the containing filesystem because content signatures cannot establish membership of the selected folder.

On whole-disk sources, TSK discovers partition ranges. Additional metadata-engine selection uses the filesystem identified for the selected source, not a separate automatic engine plan for each discovered partition. Select an individual filesystem partition to use its specialized engines.

## Automatic content selection

Deep and Full choose exactly one engine: **PhotoRec if available; otherwise Scalpel; otherwise Foremost**. If none is available, Auto cannot start that content scan. An installed lower-priority carver does not also run after the chosen engine.

For example, an NTFS source with all optional metadata engines and PhotoRec installed gives:

```text
Quick: TSK → ntfsundelete → RecuperaBit → TestDisk
Deep:  PhotoRec
Full:  TSK → ntfsundelete → RecuperaBit → TestDisk → PhotoRec
```

Preparation and diagnostics precede these sequences. Optional consolidation follows them. Every arrow means “finish this stage, then start the next,” not simultaneous execution. If a metadata engine fails, its error is saved and the remaining planned engines can still run; the attempt is marked partial. The application does not respond by running every other installed engine.

## Default and custom scans

The main **Scan** button uses the selected mode, workspace and main clone checkbox, with default scan options. It does not reuse a previous custom engine, filter or consolidation setting. The initial mode is Full.

**Scan dropdown → Custom…** opens the compact Scan, Engines and Advanced tabs. An explicit metadata choice replaces the entire automatic metadata sequence with that single adapter. An explicit content choice replaces the automatic carver choice. Quick does not use a content choice; Deep does not use a metadata choice; Full uses both stages.

| Setting | Default | Effect |
| --- | --- | --- |
| Metadata and content engines | Auto | Uses the selection rules above |
| Final consolidation | Off | When enabled, compares and groups contents after all planned engines finish |
| Clone-first | Off | When enabled, retains an input clone before scanning; ddrescue is the default clone engine |
| Extensions and size limits | No filter | Limits findings imported into the catalogue |
| Include existing files | Off | TSK targets deleted entries by default; inclusion behavior depends on the selected adapter |
| PhotoRec keep corrupted files | Off | Controls that PhotoRec option |
| PhotoRec low memory | On | Controls that PhotoRec option |
| Custom byte range | None | When set, copies the requested range to the workspace and scans that retained input |
| Backend timeout | No configured timeout | Custom Scan can set a timeout |

Custom Scan also exposes clone retries and a btrfs tree root. These options do not make every engine applicable to every source. See [engine-specific requirements](engines.md).

**Extension and size filters are not guarantees about workspace use.** The application applies exact filters when importing findings. An engine may still scan and retain contents outside that filter; PhotoRec, for example, scans its enabled file families and the application filters the catalogue afterward. The existing-files toggle likewise does not turn every metadata adapter or carver into an exclusively deleted-file search.

## Results, duplicates and completion

All engines contribute findings to the same saved catalogue. **Without consolidation, identical contents found by different engines remain separate rows**, in both Quick and Full. Different original paths with identical contents can also remain separate. No automatic cross-engine content grouping occurs just because several engines share a catalogue.

With [optional final consolidation](consolidation.md), the complete grouped view appears after finalization succeeds. One group can contain metadata and carving findings, and every original finding and recorded path remains available. The option changes grouping and may require additional content extraction; it does not run another carver.

Pause/resume controls the active job; cancel retains partial results. Cancellation during consolidation rolls back unfinished grouping and leaves the raw findings available. Reopening a saved scan does not resume an interrupted external engine at its internal checkpoint. **Scan again** creates a new attempt and can use a retained input image.

## Filesystem scope

[Lost-partition discovery](partition-discovery.md) is a separate, explicit TestDisk operation before a file scan. Its Quick/Deeper Search finds boundaries; it does not add TestDisk stages to these scan modes. Selecting a discovered target fixes and retains its byte range. Enable **Include existing files** when the partition was lost but its files were not individually deleted.

The implemented scope includes ext2/3/4, btrfs, FAT12/16/32, exFAT and NTFS. XFS is best-effort carving; APFS/HFS are carving-only. Quick is rejected for these carving-only filesystems; Full skips metadata recovery and runs its content stage. Locked encrypted volumes and ZFS are rejected. An already-unlocked mapper can be selected as a source.

## CLI examples

Replace the example paths with a source and workspace on separate physical drives.

```sh
# Automatic metadata sequence for the identified filesystem.
open-data-recovery --scan /media/source/disk.img \
  --workspace /media/recovery/scans --mode quick

# One explicit metadata adapter followed by one explicit carver.
open-data-recovery --scan /media/source/disk.img \
  --workspace /media/recovery/scans --mode full \
  --metadata tsk --carver photorec --consolidate
```

The [implementation](../../src/engine/jobs.rs) and [engine selection code](../../src/engine/adapters.rs) define these sequences. [Recovery](recovery.md) is a separate operation on the resulting selection.
