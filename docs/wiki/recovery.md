# Recovering selected files

[Wiki home](README.md) · [Storage](storage.md) · [Scan modes](scan-modes.md) · [Consolidation](consolidation.md)

Scanning finds candidates and may already extract contents. **Recovery writes the selected final files to the destination.** It does not rerun the full scan or apply every installed engine to each selected file.

## Where the recovered bytes come from

| Selected result | Recovery action |
| --- | --- |
| Finding with a retained engine file | Copy that workspace file to the destination |
| Consolidated group with retained verified content | Copy the group's retained content; check its recorded content hash |
| TSK finding without a retained artifact | Run `icat` using its recorded metadata address and partition offset, reading the retained scan input/clone if present or the verified original source otherwise |

Recovery writes independent destination copies. It keeps the workspace and source intact. If a required artifact is missing, or neither the needed original source nor retained input is available, that file cannot be recovered through its saved reference. The application reports the failure rather than automatically rescanning with another engine.

This means that recovering already-extracted files is chiefly a controlled copy operation, while recovering metadata-only candidates performs extraction at that time. Both paths use the same destination checks and audit records.

## Select and review

1. Open the finished scan or use **Menu → Scan → Saved Scan** to reopen one.
2. Search, filter and preview results. Lists are paged; the page's select-all and folder controls apply to the displayed page. Selections persist across pages, filters and application restarts, so a hidden selected file is still selected.
3. For consolidated results, inspect **N findings…** to see alternate paths and engine records. Selecting a group selects one recovery copy by default.
4. Click **Recover** to open the review dialog, choose a destination, review the selection count and storage amount, and adjust the options below.
5. Start recovery once destination validation succeeds. Review the saved, failed and skipped counts and inspect the resulting files.

Changing or adding a source in the sidebar leaves the current scan and its selections open. The result area identifies their source with **Results from…**; **Next scan: …** marks a different selected target. Review recovery also identifies the open scan's source. Selecting another source does not redirect recovery to it.

Small image/text previews and metadata can help selection. Preview extraction is limited to files up to 8 MiB. A preview is not proof that every original byte is intact, and the scan's file count is not a guaranteed recovery count.

## Destination validation

For device and mounted-folder sources, the destination must be on physical storage separate from the original source. Regular raw/dd/E01 image files may share their hosting drive with the destination. Output folders must not contain the image or any segment. The destination must neither be inside the saved-scan directory nor contain that directory. Workspace and destination can be sibling directories on the same permitted drive, as shown in [storage](storage.md).

Validation checks source/storage identity, output path confinement and available bytes. Every-path recovery includes the bytes for all distinct recorded paths in the destination-space check. Existing destination files are never overwritten. Directory identities and space are checked again as recovery proceeds.

## Review recovery options

| Option | Default | Behavior |
| --- | --- | --- |
| Preserve folder structure | On | Recreate recorded relative folders using confined output paths. Turning it off writes basenames into the destination. |
| If a filename already exists | Rename | Create a new name for the recovered copy. The alternative is to skip that output. Neither choice overwrites an existing file. |
| Preserve modification timestamps | On | Apply the recorded modification time when available; engine artifacts do not always retain an original file's timestamp. |
| Preserve Linux file metadata | Off | Restore available file permissions, ownership, supported attributes, symbolic/hard links and sparse layout; capture must have been enabled during scanning. |
| Verify SHA-256 after writing | On | Reread each written file and compare its SHA-256 with the digest computed after extraction/copy. |
| Continue after individual file errors | On | Record a per-file error and continue with other selected files. Fatal job/storage failures and cancellation still stop the job. |
| Recover every recorded path | Off; shown for consolidated scans | Write a copy for each distinct recorded path in each selected group, including alternate engine-generated names. The default writes one copy per group. |

Recovered paths may differ from recorded names because of filename conflicts or path sanitization. The recovery log records both the original path and the actual output path.

## What verification does and does not establish

Recovery checks the written length against the candidate's recorded size and computes SHA-256 for successfully extracted/copied contents. With output verification enabled, it rereads the destination and compares the digest. When a consolidated candidate has a recorded content hash, recovery also checks that hash even if optional rereading is off.

These checks establish consistency with recorded lengths, retained verified content when available, and the bytes written. **They do not prove that a file is identical to its contents before deletion or damage.** An overwritten source region, damaged metadata or an incomplete carved file can still limit recovery quality.

Each output attempt is recorded in the session's `events.log`: candidate/group ID, recorded path, actual destination path, bytes, hash when available, verification status, error and timestamp. Failed or cancelled writes may leave partial files; their paths remain recorded. A skipped conflict is also recorded.

## Reopening, retrying and finishing

Opening a saved scan retains its findings and selections. A usable retained artifact or input image can allow recovery after the original source has been disconnected. Metadata-only findings with no retained input still need the source. Normal identity validation applies if the original source path is present.

Repeating recovery writes again according to the conflict option; it does not remove earlier output copies. Scanning again is a separate operation that creates a new saved attempt rather than resuming an interrupted backend at its internal checkpoint.

After checking the final files, keep the saved scan for later selection, recovery or audit, or use **Saved Scan → select the snapshot → Delete…** when it is no longer needed. Deletion requires confirmation of the exact folder and is permanent. Workspace cleanup is not automatic. See [storage and cleanup](storage.md).

## CLI recovery

```sh
# Inspect IDs and the saved selection.
open-data-recovery --list /media/recovery/scans/SESSION

# Recover the selection already saved with the scan.
open-data-recovery --recover /media/recovery/scans/SESSION \
  --destination /media/recovery/restored

# Select every result, then recover one copy per group if consolidated.
open-data-recovery --recover /media/recovery/scans/SESSION \
  --destination /media/recovery/restored --all
```

`--ids 1,2` adds those IDs to the persisted selection before recovery; it does not clear previously selected IDs. `--all` selects every result across pages. These two selection flags are mutually exclusive. With neither flag, recovery uses the existing selection.

Add `--all-paths` to restore each distinct recorded path from selected groups, `--flat` to omit recorded folders, or `--skip` to skip existing destination names. All-paths and all-results are different: `--all-paths` controls copies within selected groups, while `--all` controls which groups/results are selected. The CLI uses the default timestamp, output-verification and continue-on-error settings.

See the [recovery implementation](../../src/engine/jobs.rs), [recovery defaults](../../src/engine/types.rs) and [CLI options](../../src/main.rs) for the underlying behavior.

## Optional Linux file metadata

Enable **Capture available file metadata and links** in Custom Scan, then **Preserve Linux file metadata, links and sparse layout** in Recover. Both are off by default. TSK supplies file mode, owner/group IDs and link information. Retained metadata-engine artifacts supply attributes available in their extracted output; Btrfs is instructed to restore file metadata and extended attributes during extraction. Carvers generally cannot supply original Linux metadata.

Original hard-link identity is preserved between selected destination files when available; identical contents alone do not imply a hard link. Workspace files are never linked into the destination. Symbolic links are recreated without following their targets and are not followed for previews. Without preservation enabled, captured symbolic-link candidates are skipped explicitly. Sparse writes/hole punching preserve file contents while avoiding allocation of large zero-filled regions when supported.

Ordinary file permissions, owner/group IDs, user extended attributes and file POSIX ACLs are attempted. Permission or filesystem limitations are recorded per output as metadata warnings; successful content recovery is reported separately. Privilege-elevating permission bits and security/trusted attributes are not applied. Directory metadata is not reconstructed; newly created output directories retain ODR's private defaults. Missing original metadata cannot be reconstructed from engine-created defaults, and the option does not certify original-file integrity.

```sh
open-data-recovery --recover /media/recovery/scans/SESSION \
  --destination /media/recovery/restored --all --preserve-linux-metadata
```
