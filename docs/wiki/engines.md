# Implemented engines and their roles

[Wiki home](README.md) · [Exact scan sequences](scan-modes.md) · [Storage](storage.md)

An **implemented engine** has an adapter in this application. An **available engine** has the necessary executable installed. A **used engine** is one selected by the current scan plan. These are different things: implementing or installing an engine does not make it run in every scan.

Check **Help → Backend availability** or `open-data-recovery --backends` for installed tools and recorded versions/hashes. Engines are external programs; the application does not download or install them during a scan. Distribution dependencies and the optional RecuperaBit, Digler and ext4rescue packages are described in [packaging](../packaging.md).

## Metadata engines

These can be used in Quick or in Full's metadata stage. “Automatic” below means the source filesystem has been identified appropriately and the required tool is installed. The precise order is on the [scan modes page](scan-modes.md).

| Engine / custom identifier | Role in this application | When selected | File contents during scan |
| --- | --- | --- | --- |
| Sleuth Kit `fls` / `icat` — `tsk` | `fls` lists filesystem records and addresses; `icat` extracts a candidate later for recovery, preview or consolidation | Default for ext2/3/4, FAT/exFAT and NTFS; also an explicit custom choice | Normally metadata and addresses only |
| Sleuth Kit `tsk_recover` — `tsk_recover` | Bulk filesystem-aware extraction into the workspace | Custom choice; not added by Auto | Extracted files |
| `ext4magic` | ext3/ext4 journal-based recovery | Automatic additional engine on ext3/ext4, or custom choice | Extracted files |
| `extundelete` | ext3/ext4 journal-based undelete | Automatic additional engine on ext3/ext4, or custom choice | Extracted files |
| `ntfsundelete` | NTFS metadata-based listing, targeted selection and undelete | Automatic additional engine on NTFS, or custom choice | Extracted files |
| RecuperaBit — `recuperabit` | NTFS filesystem reconstruction and extraction; reconstruction state is also retained | Automatic additional engine on NTFS, or custom choice | Extracted files |
| `btrfs restore` — `btrfs` | Native btrfs tree-based extraction; Custom Scan can select tree/subvolume roots, superblock mirror, snapshots and a directory | Default metadata engine on btrfs, or custom choice | Extracted files |
| TestDisk — `testdisk` | Scripted NTFS undelete in file scans; a separate action also discovers lost partitions (see below) | Automatic additional engine on NTFS, or custom choice | Extracted files |
| `ext4rescue` | ext4 recovery using a retained ddrescue clone and its matching map | Custom choice; requires ddrescue clone-first and a complete filesystem source, not a custom byte range | Extracted files and engine cache |

TSK uses `mmls` to enumerate partition ranges. Specialized metadata engines should be given the filesystem partition they expect; an explicit choice does not make an incompatible engine suitable for another filesystem. TestDisk's adapter explicitly requires NTFS. ext4rescue does not accept safecopy's report in place of a ddrescue map.

## Content engines

Deep and Full each select **one** content engine. Auto chooses the first available of **PhotoRec → Scalpel → Foremost**. This is an availability preference, not a sequence that runs all three or automatically switches to the next after a scan failure.

| Engine / custom identifier | Role | Selection |
| --- | --- | --- |
| PhotoRec — `photorec` | General signature-based carving; scope, family selection, fragmented-JPEG brute force, low-memory, keep-corrupted-file and block-size controls | First automatic choice, or custom |
| Scalpel — `scalpel` | Signature-based carving using a generated configuration based on installed signatures | Second automatic choice, or custom |
| Foremost — `foremost` | Signature-based carving | Third automatic choice, or custom |
| Digler — `digler` | Content scanning and dumping matching files | Custom only |
| Magicrescue — `magicrescue` | Recipe-based content extraction using installed recipes | Custom only |
| Recoverjpeg — `recoverjpeg` | JPEG-focused extraction | Custom only |
| bulk_extractor — `bulk_extractor` | Forensic feature/report extraction | Custom only; reports remain artifacts and are not listed as recovered original files |

Carvers save actual contents in the workspace. Original filenames and folder structure may not be available. bulk_extractor shares the content-engine selection interface, but choosing it does not turn its reports into file-recovery candidates.

## Preparation and diagnostic tools

**Menu → Scan → Find lost partitions…** uses TestDisk's scripted Quick/Deeper Search separately from the metadata sequence. It finds MBR/GPT candidate ranges, saves a report and log, and allows a candidate to become a virtual scan target. It never repairs or writes the source partition table. A following file scan retains the selected range and uses the existing filesystem engines. See [partition discovery](partition-discovery.md).

These are supporting operations, not additional competing file-recovery engines.

| Tool | Role and trigger | Retained output |
| --- | --- | --- |
| GNU ddrescue — `ddrescue` | Default clone engine; tunable acquisition with persistent map and separate resume action | Input image and map |
| `safecopy` | Custom alternative clone engine | Input image and bad-block report |
| `ewfinfo` / `ewfexport` | Inspect E01 input and convert its segments to a raw scan input | Raw image and export diagnostics |
| TSK `mmls` | Discover partition ranges for TSK metadata scans | Partition offsets and lengths recorded with findings |
| `dumpe2fs`, `debugfs` | Optional ext filesystem diagnostics before scanning; the adapter requests header/statistics information, not repair writes | Diagnostic reports |
| `btrfs-find-root` | Optional btrfs root diagnostics before scanning; it does not automatically choose a new restore root | Diagnostic report |

Diagnostics run when applicable and installed, including before a Deep scan on those filesystems. Their presence does not mean that Deep ran a metadata recovery sequence.

Current behavior is defined by the [adapter implementations](../../src/engine/adapters.rs), [scan orchestration](../../src/engine/jobs.rs) and [source inspection](../../src/engine/sources.rs). The [development guide](../development.md) explains disposable-image checks; an implemented adapter does not guarantee success on every damaged filesystem.

Engine-specific options, applicability, extraction-filter fallback and imaging resume are detailed in [advanced scan controls](scan-modes.md#advanced-engine-controls).
