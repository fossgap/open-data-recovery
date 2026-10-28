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

**Scan dropdown → Custom…** opens the Scan, Engines, Engine options and Advanced tabs. Set your options and click **Start scan** to begin directly. Controls become insensitive when their engine or stage is not used.

The **Metadata engine** dropdown contains checkboxes. **Auto** uses the applicable default sequence; checking individual engines replaces Auto with those engines, in the order checked. The resolved order appears below the dropdowns. Choosing Auto clears the manual selection; removing the last checked engine returns to Auto. Saved selections retain their order. Carving and clone-engine dropdowns remain single-choice. Uninstalled or unavailable engines are greyed out, with a tooltip explaining why. If a saved selection becomes unavailable, choose an available engine or Auto before starting.

The resolved sequence and carver are fixed for that attempt; execution rechecks availability. Quick ignores the carving choice; Deep ignores metadata choices; Full uses both stages. Hover over **Extensions** for the extraction-filter explanation.

| Setting | Default | Effect |
| --- | --- | --- |
| Metadata and content engines | Auto | Uses the selection rules above |
| Final consolidation | Off | When enabled, compares and groups contents after all planned engines finish |
| Clone-first | Off | When enabled, retains an input clone before scanning; ddrescue is the default clone engine |
| Extensions and size limits | No filter | Limits findings imported into the catalogue |
| Include existing files | Off | TSK targets deleted entries by default; inclusion behavior depends on the selected adapter |
| PhotoRec keep corrupted files | Off | Controls that PhotoRec option |
| PhotoRec low memory | Off | Controls that PhotoRec option |
| Custom byte range | None | When set, copies the requested range to the workspace and scans that retained input |
| Backend timeout | No configured timeout | Custom Scan can set a timeout |

Custom Scan also exposes clone retries and a btrfs tree root. These options do not make every engine applicable to every source. See [engine-specific requirements](engines.md).

**Extension and size filters are not universal guarantees about workspace use.** Exact result filters remain applied. PhotoRec and Foremost restrict supported file families; Scalpel selects matching installed signatures; Magicrescue selects matching recipes; NTFS undelete lists and filters records before extracting them. TSK filters its metadata before on-demand extraction. If an extension has no supported engine mapping, that adapter falls back to result filtering, as shown in the scan plan. A family such as ZIP includes DOCX/XLSX and can still extract other family members. Other engines retain their existing result-filter behavior. The existing-files toggle likewise does not turn every metadata adapter or carver into an exclusively deleted-file search.

## Results, duplicates and completion

All engines contribute findings to the same saved catalogue. **Without consolidation, identical contents found by different engines remain separate rows**, in both Quick and Full. Different original paths with identical contents can also remain separate. No automatic cross-engine content grouping occurs just because several engines share a catalogue.

With [optional final consolidation](consolidation.md), the complete grouped view appears after finalization succeeds. One group can contain metadata and carving findings, and every original finding and recorded path remains available. The option changes grouping and may require additional content extraction; it does not run another carver.

Pause/resume controls the active job; cancel retains partial results. Cancellation during consolidation rolls back unfinished grouping and leaves the raw findings available. Reopening a saved scan does not resume an interrupted file-scanning engine at its internal checkpoint. Interrupted ddrescue acquisition has a separate **Resume imaging…** action described below. **Scan again** creates a new attempt and can use a retained input image.

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

## Advanced engine controls

**Custom… → Engine options** groups controls by engine. **Engines** selects one or more metadata engines, a single carver and a clone engine. Disabled controls are not applied. Auto resolves against the detected or explicitly interpreted filesystem and installed engines. The CLI's `--plan` reports source, workspace, stages, engine versions, effective settings and extraction-filter behavior; `plan.json` and per-engine `commands.jsonl` retain the executed configuration. There is no separate plan step in Custom Scan.

| Engine | Controls |
| --- | --- |
| PhotoRec | Whole input or unallocated space; inferred or explicit supported file families; fragmented-JPEG brute force; keep corrupted files; low-memory mode (off); block-size override |
| ddrescue | Retry count, direct input reads, reversed passes, scraping, slow-read threshold, maximum read rate, timeout since last successful read, read-error limit, optional existing image/map import |
| extundelete | All files, one file, directory or inode list; deletion-time bounds; saved journal; backup superblock and filesystem block size |
| ext4magic | All files, one file, directory or inode; journal-history interval; saved journal; targeted journal transaction |
| Btrfs | Tree-root address, subvolume ID or subvolume-root address, superblock mirror, directory subtree, snapshots and continue-on-error; **List roots** and **Preview paths** perform read-only inspections |
| NTFS undelete | Filename glob, case sensitivity, inode IDs/ranges, minimum estimated recoverable percentage; extension and size selection before extraction |

PhotoRec unallocated-space scanning requires a recognized ext, FAT or NTFS filesystem partition; use whole-input scanning for an unknown or reformatted filesystem. Brute force applies to fragmented JPEG recovery and can be expensive. Explicit file families override automatic family inference; exact extension result filters still apply. Supported family identifiers are validated before execution. Unknown extension mappings do not silently remove the requested type.

The two ext engines have different time semantics: extundelete bounds deletion timestamps; ext4magic selects journal history. Values are UTC Unix seconds. Transaction selection is specific to ext4magic and cannot be combined with a time interval. Backup-superblock and filesystem-block-size options belong to extundelete. Select an explicit engine when using these specialized controls.

Btrfs directory selection is converted to a literal, ancestor-aware path expression. Extension-only selection remains a result filter for Btrfs. Its preview writes only diagnostic reports, not recovered content. Inspection of a discovered/custom byte range requires first retaining that range as a filesystem image. Metadata availability and recovery success depend on the filesystem and installed tool version.

## Filesystem interpretation

**Advanced** can override the filesystem interpretation and sector size, and select a starting TSK directory inode. Filesystem interpretation influences engine selection and is explicitly passed to TSK. The sector override is used by TSK, PhotoRec and ddrescue; other tools interpret their own filesystem geometry. TSK recovery, previews and consolidation reuse the saved interpretation. These options do not unlock encrypted volumes or make an incompatible filesystem supported.

## Resumable ddrescue imaging

Enable clone-first and choose ddrescue. Image length alone does not establish completion: ODR reads the map and records rescued, pending and bad bytes. If unread regions remain, the acquisition and map are retained; automatic scanning does not start.

Use **Saved Scan → select the attempt → Resume imaging…** to change ddrescue tuning and continue the same acquisition after interruption or application restart. Source identity and the map are validated, and the session must not be in use. After acquisition finishes, **Scan again** scans the retained image in a new attempt. File-scanning engines themselves are restarted, not resumed at arbitrary internal checkpoints.

Resume applies to private, unscanned acquisition files. A clone already published as scan input, or hard-linked into another saved attempt, is kept immutable. **Scan again** can explicitly inspect a retained partial image, but doing so prevents later in-place acquisition updates to that shared image.

Custom Scan also accepts an existing raw image and its matching ddrescue map. Both are copied into the new workspace; the originals are not modified. Map coverage and image length are checked, and samples from rescued regions must match the selected source. This is sampled correspondence validation, not proof that every imported byte belongs to that source. Importing requires space for another image copy.

## Advanced CLI configuration

`--options FILE.json` accepts the serialized scan settings, with omitted fields using defaults. CLI flags explicitly supplied alongside it override the corresponding common settings. `--plan` resolves and prints the plan without scanning or creating a workspace.

```json
{
  "mode": "Full",
  "workspace": "/media/recovery/scans",
  "extensions": ["jpg", "pdf"],
  "metadata_sequence": ["tsk", "extundelete"],
  "carver": "photorec",
  "clone_first": true,
  "retry_passes": 2,
  "ddrescue": { "direct": true, "scrape": false },
  "photorec": { "free_space": false, "brute_force": true },
  "capture_metadata": true,
  "interpretation": { "filesystem": "ext4", "sector_size": 512 }
}
```

```sh
open-data-recovery --scan /media/source/partition.img --options scan.json --plan
open-data-recovery --scan /media/source/partition.img --options scan.json
open-data-recovery --resume-imaging /media/recovery/scans/SESSION
```

For targeted runs, merge one of these fragments into a scan-options file (and
set `mode` to `Quick` or `Full`):

```json
{"metadata_backend":"extundelete","ext":{"target_kind":"directory","target":"home/alice/Documents","after":1767225600}}
```

```json
{"metadata_backend":"btrfs","btrfs":{"subvolume":256,"path":"home/alice/Documents","superblock":0,"snapshots":true},"capture_metadata":true}
```

```json
{"metadata_backend":"ntfsundelete","extensions":["pdf","docx"],"ntfs":{"pattern":"Report*","inodes":"42,70-80","minimum_percent":80,"case_sensitive":false}}
```

Targets are relative to the filesystem root. Supplied NTFS constraints are
combined; the inode list does not bypass the name, extension or percentage
filters. Extundelete's `after` example is a UTC deletion-time bound. Btrfs root
IDs/addresses must come from the source's own inspection output.

The complete saved settings are in each session's `manifest.json` under `options`. Engine-specific structures are `photorec`, `ddrescue`, `ext`, `btrfs`, `ntfs` and `interpretation`; the CLI and GUI use the same validation and plan resolver.
