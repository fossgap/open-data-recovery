# Storage, workspace and destination

[Wiki home](README.md) · [Scan modes](scan-modes.md) · [Recovery](recovery.md)

There are three main storage locations. An optional clone is part of the workspace, so it does not require a fourth location or a third physical drive.

| Location | Contents | Storage rule |
| --- | --- | --- |
| Source | Original disk, partition, image or filesystem containing the selected folder | Opened read-only by the application |
| Workspace | Saved scans, catalogues, logs, extracted files, comparison/preview caches, and optional retained input images | Must be on a different physical drive from the original source |
| Recovery destination | Selected final files, written using the recovery options | Must be on a different physical drive from the source and outside the saved-scan directory |

Workspace and destination **can share the same non-source drive**. For example:

```text
Drive A: original source

Drive B: /media/recovery/
  scans/
    SESSION/              saved scan and all of its working data
  restored/               selected recovered files
```

Another partition on Drive A does not satisfy the separation rule. The recovery destination must neither be inside the saved-scan directory nor contain that directory. A separate Drive C for the destination is allowed, but is not required. Storage whose physical relationship to the source cannot be verified is rejected.

The default workspace is `$XDG_DATA_HOME/open-data-recovery/snapshots`, or `~/.local/share/open-data-recovery/snapshots`. This default is still checked against the selected source and may need changing in **Custom… → Scan → Workspace**.

## Does the workspace contain actual file contents?

**Yes, depending on the engine and options.** Metadata describes how an engine finds files; it does not specify whether that engine extracts them during the scan.

| Operation | What it adds to the workspace |
| --- | --- |
| TSK metadata listing | Catalogue entries and source addresses; normally no file-content extraction at this stage |
| Other metadata extractors, such as ntfsundelete or btrfs restore | Actual extracted files as well as catalogue entries |
| Content carving, such as PhotoRec | Actual carved files; names and original paths may be unavailable |
| Optional consolidation | Group records and potentially new extracted contents; existing verified artifacts are reused |
| Preview of a TSK candidate without a retained artifact | A small extracted preview file; this does not turn every catalogue entry into a retained recovery copy |
| Clone-first | `clone/clone.img` plus a ddrescue map or safecopy report |
| E01 input | A retained raw export, `ewf.raw`, before scanning |
| Custom byte range | A retained `source-range.img` covering the requested range |

Consequently, **Quick is not guaranteed to use little workspace**, and **Deep is not the only mode that writes file contents**. See [the exact engine sequences](scan-modes.md).

## Why also have a recovery destination?

The workspace preserves the scan and what its engines produced. It may contain unselected files, duplicate engine outputs, partial files, generated names and reports. The destination contains the file copies you chose to recover, arranged according to the folder and filename-conflict options.

For a finding whose contents are already retained, recovery **copies** those contents to the destination. It does not move the workspace file or replace the output with a hard link. The duplication is real, and it lets the saved scan remain usable for another recovery.

For a metadata-only finding, recovery reads its contents from the original source or retained scan input directly into the destination. That finding does not automatically acquire another permanent full-content cache during recovery.

Consolidation reduces duplicate result rows and the number of default recovery copies. **It does not delete the engines' existing duplicate artifacts.** It deletes only new temporary extractions that it has verified are duplicates of retained content. See [consolidation storage behavior](consolidation.md).

## What is inside a saved scan?

These paths are relative to `WORKSPACE/SESSION/`; optional files exist only when their operation was used.

| Path | Purpose |
| --- | --- |
| `manifest.json`, `source.json` | Source identity, scan settings, state, errors and recorded engine versions |
| `tree.sqlite` | Original findings and, when enabled and finished, final groups and their paths |
| `selection.sqlite` | Persisted selection; created when files are selected |
| `events.log` | Scan events and recovery audit records, including output paths, errors and hashes |
| `artifacts/` | Engine outputs, extracted contents and diagnostic reports |
| `artifacts/consolidation/` | New unique contents retained during consolidation |
| `artifacts/previews/` | Extracted previews when needed |
| `artifacts/recovery-…/` | Recovery job working directory |
| `clone/clone.img`, `clone/clone.map` or `clone/clone.badblocks` | Optional clone and its associated map/report |
| `ewf.raw` | Optional E01-to-raw conversion |
| `source-range.img` | Optional custom-range input |
| `retained-input.img` | Input reused by a new attempt from an older saved scan |

## Space needed through the workflow

Budget additional storage for:

```text
retained input images (clone, E01 export, custom range, if used)
+ extracted engine contents and consolidation cache
+ catalogue, logs, previews and other reports
+ final recovered file copies
+ temporary extraction and database headroom
```

Count all enabled preparation steps. For example, E01 conversion followed by clone-first can retain both the raw export and a clone; a custom range adds its own image. A whole-source clone represents the source size, not just the size of selected files. Actual allocated storage also depends on the filesystem and image contents.

If the workspace retains **10 GB** of file contents and you recover all of them, the two locations together need approximately **20 GB**, excluding retained input images and other overhead. A hypothetical additional **500 GB allocated clone** makes that approximately **520 GB**. Recovering only 2 GB from that 10 GB cache instead gives approximately 12 GB before the clone and overhead.

These are arithmetic examples, not scan-size predictions. Several engines may retain multiple copies, and extension filters limit the catalogue without necessarily preventing engines from saving other file types. Restoring every recorded path from a group also writes more destination copies. The application checks available space, but cannot predict every engine's eventual output size before scanning. [Measured consolidation costs](../consolidation-implementation.md) show actual off/on differences on the documented fixtures.

## Reopening and cleanup

**Menu → Scan → Saved Scan** reopens a saved attempt. Metadata-only findings still need the original source or a retained input image for recovery. Retained content can be recovered without the original source, subject to the saved scan's validation checks. Consolidation does not make every scan self-contained: unique-size TSK findings can remain metadata-only.

Scanning again creates a new attempt. A retained input may be shared through a hard link when possible, or copied otherwise; the old catalogue is kept. This input reuse is separate from recovery, which writes independent destination copies.

Recovery does not automatically remove workspace contents. After checking the recovered files, remove an unneeded snapshot through **Menu → Scan → Saved Scan**:

1. Choose its workspace and select the saved scan.
2. Click **Delete…**. The confirmation identifies the source, saved date and exact folder.
3. Choose **Cancel** to keep it, or **Delete saved scan** to permanently remove that whole folder. This includes its catalogue, selection, logs, extracted contents, caches and any clone. There is no trash or undo operation.

The application performs deletion in the background and refreshes the saved-scans list. Deleting the open snapshot also clears its displayed results and preview; deleting a different snapshot leaves the current scan open. The original source and independent recovered files outside that folder remain intact.

Deletion refuses a snapshot in active use by a current application scan/recovery operation. It checks the folder identity again after confirmation, does not follow symbolic links, and refuses traversal into nested mounts. If a nested filesystem is mounted inside a snapshot, unmount it before deletion. No free-space reserve is required to delete a snapshot from a full workspace. A failure can leave a partly deleted folder; the error identifies it, and remaining contents can be removed with another deletion attempt if its manifest is still present.

Removing only engine artifacts manually can break later previews and recovery. When another attempt shares a retained image through a hard link, deleting one snapshot removes its link but does not free the image's storage until the remaining links are removed.

Read-only source access applies to this application and its tools. It does not stop other applications from changing a mounted source filesystem.
