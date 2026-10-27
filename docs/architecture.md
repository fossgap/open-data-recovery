# Implementation architecture

## Boundaries

`src/live.rs` owns the production GTK controller. `src/ui.rs` and `src/model.rs` implement the explicitly selected fixture demo. `src/main.rs` routes GUI, demo, smoke checks, discovery, scan, listing, and recovery commands. Both production GUI and CLI use `src/engine`.

| Module | Responsibility |
| --- | --- |
| `types.rs` | Serializable source identity, scan/recovery options, candidates, manifests and events |
| `sources.rs` | lsblk/findmnt discovery, raw/E01 inspection, fingerprint checks, read-only open and UDisks/polkit FD acquisition |
| `safety.rs` | Physical ancestry, mapper/RAID/loop dependencies, btrfs members, storage/free-space validation and directory-relative output creation |
| `process.rs` | Shell-free arguments, read-only input FD, Landlock write isolation, process-group pause/cancel/timeout, bounded diagnostic output |
| `adapters.rs` | Scanner/Recoverer traits, tool adapters, TSK body-file parsing, partition ranges and retained-artifact import |
| `jobs.rs` | Scan preparation, E01 export, clone/range retention, worker orchestration, recovery, preview and audit hashes |
| `consolidation.rs` | Optional final content grouping, bounded comparison/extraction, verified artifact reuse and atomic publication |
| `store.rs` | SQLite candidates and selection, JSON manifests, legacy projections, logs and recorded-version comparison |
| `sessions.rs` | Shared session lifetime locks and confined, identity-checked saved-scan deletion |

## Source protection

Every real source descriptor is opened read-only. For protected block devices, UDisks `OpenDevice("r")` returns an FD after polkit authorization; the application does not run a root shell or unmount filesystems. Source fingerprints are rechecked before a new live-source attempt.

Storage validation resolves physical leaf ancestry through sysfs, including partition parents, mapper/RAID slaves, loop backing files and known btrfs member devices. Workspace and recovery destination must not overlap the original source's physical storage. Unknown topology fails closed. Symlink-containing destination paths, parent traversal, changed directory identity, nested output mounts and insufficient space are rejected.

Backend processes receive a read-only source as FD 3 and literal argument arrays. Landlock ABI 3 denies write, truncation, creation, rename and deletion everywhere except that backend's approved output directory. A backend cannot write a retained clone located outside its artifact directory. Output FDs for extraction are created by the application with exclusive creation and passed deliberately. Low disk space, timeout and cancellation terminate the process group; partial outputs remain auditable.

These controls protect against unintended writes by the application and its engines. They do not freeze a live filesystem or stop other processes from modifying it. A clone of a mounted filesystem is not automatically a consistent filesystem snapshot.

## Scanning and recovery

Quick uses metadata adapters; Deep uses signature/content extraction; Full performs both. Defaults prefer TSK and PhotoRec, adding installed journal/MFT alternatives when the identified filesystem supports them. btrfs uses its native restore tools. Explicit custom engines are available through the same traits. Metadata extractors that write files eagerly retain them under `artifacts/`; TSK candidates retain addresses for later `icat` extraction. bulk_extractor feature reports remain reports.

Whole-disk TSK scans use mmls partition ranges. Compound NTFS addresses are strings, never truncated to integer inode numbers. Candidate offsets and lengths are in bytes relative to the session scan input; a custom range's source offset is recorded in the manifest. Extension and size filtering is exact at result import, even when a backend groups several extensions into one file family.

Recovery reads either retained artifacts or verified source/clone metadata addresses. Directory-relative file creation never follows destination symlinks and never overwrites existing files. Completed bytes receive a SHA-256 record; optional verification rereads the output. A mismatch with the candidate's recorded length is reported as an error. Failures preserve an explicit partial-output path in `events.log`.

### Final consolidation

`ScanOptions.consolidate` defaults to false, including for deserialized older options. Enabled scans enter `consolidating` after their engine plan finishes. A size index is created while the candidate table is empty, only for enabled scans. The finalizer uses size buckets and keyset pages of at most 128 findings, processes existing artifacts before lazy TSK addresses, hashes full contents and byte-compares hash matches. It retains only new unique extractions; original artifacts remain intact. Content and displayed-path preference are independent: TSK metadata names take precedence over other metadata and carved names.

Groups, member IDs and distinct recorded paths are written in one SQLite transaction. Raw candidate rows remain unchanged. A complete grouping transaction is committed before the atomic manifest update exposes the `consolidated_candidates` view. Cache files and directories are synced first. Cancelled/fatal attempts roll back grouping and clean only their own new cache; incomplete manifests reopen the raw table. Findings that cannot be read completely for comparison become separate unverified groups and make the attempt partial. Unique-size singletons require no content read. Canonical file identity, length and modification/change times are checked before publication; recovery checks the recorded content hash again.

An advisory scan lock blocks selecting raw IDs while an enabled scan is running. The operating system releases it when the scan process exits, so a stale active state in an interrupted manifest does not prevent recovery of the raw findings. Selection from a stale raw view is rejected after a completed grouping is published; callers must reload the final groups.

Enabled sessions use manifest format 2; format 0/1 scans still load without candidate-tree migration. The SQLite view provides preferred names and canonical content to normal paging, preview and selection. Search matches every recorded path. Provenance and alternate-path recovery use bounded pages. Group selection restores one copy; `RecoveryOptions.restore_all_paths` expands to distinct recorded paths and includes their full byte count in destination validation. Original findings stay queryable through the UI and `--findings`.

Short extraction jobs use a Linux PID descriptor to wake the supervisor immediately when a child exits, while pause/cancel/timeout and free-space supervision retain their existing intervals. If PID descriptors are unavailable, the previous polling behavior is used. This avoids the spike's fixed 50 ms wait per short extraction without removing the Landlock sandbox or read-only source descriptors. See the [Linux pidfd documentation](https://man7.org/linux/man-pages/man2/pidfd_open.2.html).

## Persistence

A scan directory contains:

```text
manifest.json          Attempt state, settings, source identity, tool versions/hashes, errors
source.json            Original source descriptor
tree.sqlite           Candidate paths, sizes, addresses, partition ranges and artifact references
selection.sqlite       Separate, persistent candidate selection (created on first selection)
events.log             JSON-lines lifecycle and recovery audit records
artifacts/             Per-engine outputs, bounded diagnostics and small previews
clone/clone.img         Optional retained clone
clone/clone.map         ddrescue map (or clone.badblocks for safecopy)
ewf.raw                Optional normalized E01 input
source-range.img       Optional normalized custom byte range
```

Manifest updates use a temporary file, fsync and rename. Candidate inserts are batched. The GTK tree queries bounded pages; it does not load a whole drive's result set into memory. Older `files`/`entries`/`candidates` tables are projected onto the current candidate model without schema migration during load. A saved clone can be referenced in a new attempt by a retained hard link or a checked copy; the prior tree is unchanged.

Comparison requires matching source fingerprints and uses the versions/hashes recorded in both manifests. The currently installed tools do not redefine old attempts. Selecting and recovering files writes a sidecar and log only after validating the session's storage again.

The production controller stores the next selected source separately from the source of its open scan. Source selection/addition retains the current catalogue page, persisted selection, search and preview. Result and recovery labels identify the open scan; starting a scan, opening another attempt or deleting the open attempt changes that context. Asynchronous open responses carry a generation so obsolete results cannot reopen a deleted/replaced view.

Saved-scan deletion uses a prepared identity (manifest ID/source and directory device/inode), then revalidates it after UI confirmation. Every `Store` holds a shared advisory lock on its directory; deletion requires an exclusive lock, protecting active scans, recovery and readers across current application instances without migrating legacy files. A two-pass traversal checks the tree before deletion and uses directory-relative `openat2` with `RESOLVE_BENEATH`, `RESOLVE_NO_SYMLINKS` and `RESOLVE_NO_XDEV`, followed by `unlinkat`. Symlinks are unlinked, never followed; nested mounts are rejected. The catalogue and manifest are removed last to leave a recognizable remainder after most partial failures. Source-separation validation remains required, but deletion does not require free-space headroom. These operations run on the worker thread.

## Verification and limits

Unit tests cover source/output isolation, unsafe paths, conflicts, compound addresses, option parsing, timeouts, legacy read behavior and comparison. Integration tests format **regular temporary image files only**, recover known content, compare hashes, disconnect the original, reopen/rescan a clone, and carve an embedded image. GTK tests run both the fixture workflow and production dialogs under a private X server.

Adapters implement the required filesystem/backend matrix, but successful recovery depends on the installed tool and the damaged filesystem. Not every damage pattern or filesystem feature has a fixture. Optional engine absence is reported, unsupported encrypted/ZFS sources are rejected, APFS/HFS/XFS use the documented carving scope, and unknown source/destination ancestry is not bypassed.
