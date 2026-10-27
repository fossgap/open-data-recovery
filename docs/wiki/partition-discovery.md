# Finding lost partitions with TestDisk

[Wiki home](README.md) · [Scan modes](scan-modes.md) · [Storage](storage.md) · [Recovery](recovery.md)

Use **Menu → Scan → Find lost partitions…** when a disk's partition table is missing or damaged. This is a separate TestDisk search for filesystem boundaries. It does not recover files, mount a partition, repair a filesystem or write a partition table.

## Workflow

1. Select the whole disk or add its raw/dd image in **Sources**.
2. Open **Find lost partitions…** and choose a workspace on a different physical drive for the search report and TestDisk log.
3. Choose **Auto**, **MBR / Intel**, or **GPT**. Auto asks TestDisk to detect the table type. If the table is too damaged to identify, select the original table type explicitly.
4. Start **Quick Search**. Try **Deeper Search** if useful partitions are missing. These names refer to partition discovery, independently of the Quick/Deep/Full file-scan modes. Deeper Search can take a long time on a large disk. **Cancel search** stops the worker before closing the dialog.
5. Review the candidates. Each shows its filesystem hint, size and exact byte offset. **Filesystem header verified** means `blkid` recognized a filesystem at that bounded location; it does not certify file recoverability. An **Unverified candidate** may have a damaged primary header, a backup superblock, or stale metadata.
6. Select **Use partition**. The candidate appears under its original source in the sidebar as a virtual partition. The current scan, selections, search and preview stay open until you start a new file scan.
7. Choose a file-scan mode. To recover files lost because the partition disappeared, use **Custom Scan → Include existing files in metadata results**: those files may still be allocated in the filesystem. The ordinary default scan options remain unchanged.
8. Scan, select files and recover normally. The partition range is copied into the new saved scan as `source-range.img` before the existing engines run. Custom byte-range fields show that fixed range and cannot be edited for this target.

Candidates can include partitions already present in the table, overlapping alternatives and obsolete filesystem boundaries. The application deduplicates exact ranges, retains overlapping alternatives, and reports overlaps. It does not claim every candidate is a lost, usable partition. A completed search with no candidates is reported separately from a failed or cancelled search.

## Storage and source protection

Searches save `WORKSPACE/partition-searches/SEARCH-ID/report.json` and `testdisk.log`. The report records source identity, search settings, TestDisk version/hash, candidate ranges and verification status. Discovery itself retains metadata and logs, not partition contents. Failed or cancelled searches retain reports/logs when their workspace was already created; incomplete reports cannot supply scan targets.

**Open report…** reopens a completed report, including after restarting the application. The original source must still be available and pass identity and partition-content sample checks before starting a file scan. These samples are identity checks, not full-disk integrity hashes.

The subsequent file scan requires space for the selected partition range, plus its engine outputs and any optional consolidation cache. Enabling clone-first also retains the normal full-source clone before extracting the range. Recovery can use a successfully retained range after the original source is disconnected. The original disk's physical ancestry remains attached to the virtual target, so source/workspace/destination separation checks still apply.

Search reports are separate from saved file scans and do not appear in **Saved Scan**. After a search finishes, an unneeded `partition-searches/SEARCH-ID` folder can be removed manually. Removing a report does not remove a saved file scan's retained range; deleting that saved scan does not remove its earlier discovery report.

TestDisk receives a read-only source descriptor and runs with the existing backend write sandbox. The command list is fixed to table selection, `analyze`, and optionally `search`; no write or repair commands are exposed. Search and scan access cannot stop other applications from modifying a mounted source.

## Scope

- MBR/Intel and GPT discovery on raw/dd images and block devices. Raw images currently use 512-byte sectors; devices use their reported logical sector size.
- E01 inputs require an exported raw image or a retained clone for this operation. Mounted-folder sources and already-discovered ranges cannot be used as whole-disk search inputs.
- Discovery reads the selected source directly. Clone-first is a file-scan option, not a preparation step for the partition search.
- Filesystem recovery support is unchanged. Discovering a boundary does not add a new filesystem engine or unlock encryption. See [implemented engines](engines.md).

## CLI

```sh
# Prints a JSON report and saves report.json plus testdisk.log in the workspace.
open-data-recovery --discover-partitions /media/source/disk.img \
  --workspace /media/recovery/scans --table intel

# Add --deeper for Deeper Search; --timeout SECONDS limits the TestDisk process.
# --table accepts auto (default), intel/mbr or gpt. Timeout 0 is unlimited.

# Partition numbers start at 1 in the report's sorted candidate order.
open-data-recovery --scan-partition \
  /media/recovery/scans/partition-searches/SEARCH-ID/report.json \
  --partition 1 --workspace /media/recovery/scans --mode quick --include-existing
```

The scan accepts the normal engine, extension, clone and consolidation options. It creates an ordinary saved scan; use `--list` and `--recover` as described in [recovery](recovery.md).

The read-only integration uses the upstream [TestDisk scripted search commands](https://www.cgsecurity.org/testdisk_doc/scripted_run.html). The app does not expose the partition-table writes described elsewhere in TestDisk's documentation.
