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
| `store.rs` | SQLite candidates and selection, JSON manifests, legacy projections, logs and recorded-version comparison |

## Source protection

Every real source descriptor is opened read-only. For protected block devices, UDisks `OpenDevice("r")` returns an FD after polkit authorization; the application does not run a root shell or unmount filesystems. Source fingerprints are rechecked before a new live-source attempt.

Storage validation resolves physical leaf ancestry through sysfs, including partition parents, mapper/RAID slaves, loop backing files and known btrfs member devices. Workspace and recovery destination must not overlap the original source's physical storage. Unknown topology fails closed. Symlink-containing destination paths, parent traversal, changed directory identity, nested output mounts and insufficient space are rejected.

Backend processes receive a read-only source as FD 3 and literal argument arrays. Landlock ABI 3 denies write, truncation, creation, rename and deletion everywhere except that backend's approved output directory. A backend cannot write a retained clone located outside its artifact directory. Output FDs for extraction are created by the application with exclusive creation and passed deliberately. Low disk space, timeout and cancellation terminate the process group; partial outputs remain auditable.

These controls protect against unintended writes by the application and its engines. They do not freeze a live filesystem or stop other processes from modifying it. A clone of a mounted filesystem is not automatically a consistent filesystem snapshot.

## Scanning and recovery

Quick uses metadata adapters; Deep uses signature/content extraction; Full performs both. Defaults prefer TSK and PhotoRec, adding installed journal/MFT alternatives when the identified filesystem supports them. btrfs uses its native restore tools. Explicit custom engines are available through the same traits. Metadata extractors that write files eagerly retain them under `artifacts/`; TSK candidates retain addresses for later `icat` extraction. bulk_extractor feature reports remain reports.

Whole-disk TSK scans use mmls partition ranges. Compound NTFS addresses are strings, never truncated to integer inode numbers. Candidate offsets and lengths are in bytes relative to the session scan input; a custom range's source offset is recorded in the manifest. Extension and size filtering is exact at result import, even when a backend groups several extensions into one file family.

Recovery reads either retained artifacts or verified source/clone metadata addresses. Directory-relative file creation never follows destination symlinks and never overwrites existing files. Completed bytes receive a SHA-256 record; optional verification rereads the output. A mismatch with the candidate's recorded length is reported as an error. Failures preserve an explicit partial-output path in `events.log`.

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

## Verification and limits

Unit tests cover source/output isolation, unsafe paths, conflicts, compound addresses, option parsing, timeouts, legacy read behavior and comparison. Integration tests format **regular temporary image files only**, recover known content, compare hashes, disconnect the original, reopen/rescan a clone, and carve an embedded image. GTK tests run both the fixture workflow and production dialogs under a private X server.

Adapters implement the required filesystem/backend matrix, but successful recovery depends on the installed tool and the damaged filesystem. Not every damage pattern or filesystem feature has a fixture. Optional engine absence is reported, unsupported encrypted/ZFS sources are rejected, APFS/HFS/XFS use the documented carving scope, and unknown source/destination ancestry is not bypassed.
