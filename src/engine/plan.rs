//! A single resolver for the review page and execution. No writes or source reads.
use super::{Result, adapters, fail, options, types::*};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScanPlan {
    pub metadata: Vec<String>,
    pub carver: Option<String>,
    pub filesystem: String,
    pub sector_size: u32,
    pub description: String,
}
pub fn resolve(
    source: &SourceDescriptor,
    o: &ScanOptions,
    versions: &BTreeMap<String, String>,
) -> Result<ScanPlan> {
    let mut checked = o.clone();
    checked.validate()?;
    let fs = options::filesystem(source, o).to_ascii_lowercase();
    let mut interpreted = source.clone();
    interpreted.filesystem = fs.clone();
    let carve_only = matches!(fs.as_str(), "xfs" | "apfs" | "hfs" | "hfsplus");
    if matches!(
        source.filesystem.to_ascii_lowercase().as_str(),
        "zfs" | "zfs_member" | "crypto_luks" | "bitlocker"
    ) {
        return fail("Select an unlocked supported filesystem; ZFS is unsupported.");
    }
    if source.scope.is_some() && o.mode != Mode::Quick {
        return fail("Folder sources require Quick metadata scanning.");
    }
    if o.mode == Mode::Quick && carve_only {
        return fail("This filesystem requires Deep content carving.");
    }
    let mut metadata = vec![];
    if o.mode != Mode::Deep && !carve_only {
        if !o.metadata_sequence.is_empty() {
            for name in &o.metadata_sequence {
                let mut choice = o.clone();
                choice.metadata_backend = name.clone();
                metadata.push(adapters::metadata_engine(&interpreted, &choice, versions)?);
            }
        } else {
            metadata.push(adapters::metadata_engine(&interpreted, o, versions)?);
            if o.metadata_backend == "auto" && source.scope.is_none() {
                let extras: &[&str] = match fs.as_str() {
                    "ext3" | "ext4" => &["ext4magic", "extundelete"],
                    "ntfs" => &["ntfsundelete", "recuperabit", "testdisk"],
                    _ => &[],
                };
                metadata.extend(
                    extras
                        .iter()
                        .filter(|n| versions.contains_key(**n))
                        .map(|n| n.to_string()),
                );
            }
        }
    }
    let carver = if o.mode == Mode::Quick {
        None
    } else {
        Some(adapters::carving_engine(o, versions)?)
    };
    let has = |name: &str| metadata.iter().any(|n| n == name);
    if has("btrfs") && fs != "btrfs" {
        return fail("Btrfs recovery needs a Btrfs filesystem or explicit Btrfs interpretation.");
    }
    if has("ntfsundelete") || has("recuperabit") || has("testdisk") {
        if fs != "ntfs" {
            return fail("NTFS recovery needs an NTFS filesystem or explicit NTFS interpretation.");
        }
    }
    if (has("extundelete") || has("ext4magic")) && !fs.starts_with("ext") {
        return fail(
            "The selected ext engine needs an ext filesystem or an explicit ext interpretation.",
        );
    }
    if has("ext4magic") && o.ext.target_kind == "inode" && o.ext.target.contains(',') {
        return fail("ext4magic accepts one target inode; choose extundelete for a list.");
    }
    if has("ext4magic") && (o.ext.superblock.is_some() || o.ext.block_size.is_some()) {
        return fail(
            "Backup superblock/block-size overrides are supported by extundelete, not ext4magic.",
        );
    }
    if has("extundelete") && o.ext.transaction.is_some() {
        return fail("Journal transaction selection is available only in ext4magic.");
    }
    if o.ext.transaction.is_some()
        && (o.ext.target_kind == "all" || o.ext.after.is_some() || o.ext.before.is_some())
    {
        return fail("A journal transaction needs a specific ext4magic target and no time window.");
    }
    if o.clone_first {
        if !["ddrescue", "safecopy"].contains(&o.clone_backend.as_str())
            || !versions.contains_key(&o.clone_backend)
        {
            return fail("Install the selected clone engine (ddrescue or safecopy).");
        }
    }
    if carver.as_deref() == Some("photorec") && o.photorec.free_space {
        if !matches!(
            fs.as_str(),
            "ext2" | "ext3" | "ext4" | "fat12" | "fat16" | "fat32" | "vfat" | "ntfs"
        ) {
            return fail(
                "PhotoRec unallocated-space scanning needs an ext, FAT or NTFS filesystem partition.",
            );
        }
        if o.photorec.block_size.is_some() {
            return fail("Use automatic block size for unallocated-space scanning.");
        }
    }
    if has("tsk") && o.interpretation.filesystem == "btrfs" {
        return fail("Use the Btrfs engine for a Btrfs interpretation.");
    }
    let sector_size = options::sector(source, o);
    let offset = source
        .partition
        .as_ref()
        .map_or(o.offset_bytes, |p| p.offset_bytes);
    let length = source
        .partition
        .as_ref()
        .map(|p| p.length_bytes)
        .or(o.length_bytes);
    if offset >= source.size
        || length.is_some_and(|n| offset.checked_add(n).is_none_or(|end| end > source.size))
    {
        return fail("The selected byte range is outside the source.");
    }
    let mut lines = vec![
        format!("Source: {}\n{}", source.name, source.path.display()),
        format!(
            "Mode: {:?} · Filesystem: {} · Sector size: {} bytes",
            o.mode, fs, sector_size
        ),
        format!("Workspace: {}", o.workspace.display()),
    ];
    if source.ewf {
        lines.push("1. Export E01 segments to retained raw input".into());
    }
    if o.clone_first {
        lines.push(format!(
            "Clone first: {} · {} retry passes",
            o.clone_backend, o.retry_passes
        ));
        if o.clone_backend == "ddrescue" {
            if let Some(image) = &o.ddrescue.import_image {
                lines.push(format!("Import a private copy of {} and map {}. Rescued-region samples must match the source.",image.display(),o.ddrescue.import_map.as_ref().unwrap().display()));
            }
            lines.push(format!("Direct reads: {} · Reverse: {} · Scraping: {}\nSlow-read threshold: {} B/s · Rate limit: {} B/s · Read timeout: {} s · Error limit: {:?}",o.ddrescue.direct,o.ddrescue.reverse,o.ddrescue.scrape,o.ddrescue.min_read_rate,o.ddrescue.max_read_rate,o.ddrescue.read_timeout,o.ddrescue.max_read_errors));
        }
    }
    let offset = source
        .partition
        .as_ref()
        .map_or(o.offset_bytes, |p| p.offset_bytes);
    let length = source
        .partition
        .as_ref()
        .map(|p| p.length_bytes)
        .or(o.length_bytes);
    if offset != 0 || length.is_some() {
        lines.push(format!(
            "Retain source range: offset {offset} bytes, length {}",
            length.map_or("to end".into(), |n| format!("{n} bytes"))
        ));
    }
    lines.push("Applicable read-only filesystem diagnostics precede scanning.".into());
    for (i, name) in metadata.iter().chain(carver.iter()).enumerate() {
        lines.push(format!(
            "{}. {} — {}",
            i + 1,
            name,
            filter_description(name, o)
        ));
        if let Some(version) = versions.get(if name == "tsk" { "fls" } else { name }) {
            lines.push(format!("   {}", version.lines().next().unwrap_or(version)));
        }
    }
    if has("ext4magic") || has("extundelete") {
        lines.push(format!("Ext target: {} {} · After: {:?} · Before: {:?}\nSaved journal: {} · Backup superblock: {:?} · Block size: {:?} · Transaction: {:?}",o.ext.target_kind,o.ext.target,o.ext.after,o.ext.before,o.ext.journal.as_ref().map_or("internal".into(),|p|p.display().to_string()),o.ext.superblock,o.ext.block_size,o.ext.transaction));
    }
    if has("btrfs") {
        lines.push(format!("Btrfs: tree {:?} · subvolume {:?} · subvolume address {:?} · superblock {}\nSnapshots: {} · Continue on errors: {} · Directory: {}",o.btrfs_root,o.btrfs.subvolume,o.btrfs.subvolume_root,o.btrfs.superblock,o.btrfs.snapshots,o.btrfs.ignore_errors,o.btrfs.path));
    }
    if has("ntfsundelete") {
        lines.push(format!("NTFS: filename '{}' · inodes '{}' · minimum estimated recoverable {}% · case sensitive: {}",o.ntfs.pattern,o.ntfs.inodes,o.ntfs.minimum_percent,o.ntfs.case_sensitive));
    }
    if has("tsk") {
        lines.push(format!(
            "TSK: starting directory inode {:?} · Include existing: {}",
            o.interpretation.directory_inode, o.include_existing
        ));
    }
    if carver.as_deref() == Some("photorec") {
        if !o.photorec.families.is_empty() {
            lines.push(format!(
                "Explicit PhotoRec families: {}",
                o.photorec.families.join(", ")
            ));
        }
        lines.push(format!("PhotoRec: {} · Fragmented JPEG brute force: {}\nKeep corrupted: {} · Low memory: {} · Block size: {:?}",if o.photorec.free_space{"unallocated space"}else{"whole selected input"},o.photorec.brute_force,o.keep_corrupted,o.low_memory,o.photorec.block_size));
    }
    lines.push(format!(
        "Result extensions: {} · Size: {}–{} bytes · Engine timeout: {} seconds (0 = unlimited)",
        if o.extensions.is_empty() {
            "all".into()
        } else {
            o.extensions.join(", ")
        },
        o.minimum_size,
        o.maximum_size.map_or("unlimited".into(), |n| n.to_string()),
        o.timeout_seconds
    ));
    lines.push(format!(
        "Capture available Linux metadata: {}\nFinal consolidation: {}",
        o.capture_metadata, o.consolidate
    ));
    if !o.extensions.is_empty() {
        lines.push("File families may contain several extensions. Exact result filters remain applied; unsupported extraction filters do not limit workspace use.".into());
    }
    Ok(ScanPlan {
        metadata,
        carver,
        filesystem: fs,
        sector_size,
        description: lines.join("\n\n"),
    })
}
pub fn filter_description(engine: &str, o: &ScanOptions) -> String {
    if engine == "photorec" && !o.photorec.families.is_empty() {
        return format!(
            "extract explicit families ({}) and apply result filters",
            o.photorec.families.join(", ")
        );
    }
    if o.extensions.is_empty() {
        return "all file types".into();
    }
    match engine {
        "tsk" => "filter metadata before on-demand extraction".into(),
        "photorec" if options::photorec_families(&o.extensions).is_some() => format!(
            "extract selected families ({}) and filter results",
            options::photorec_families(&o.extensions)
                .unwrap()
                .join(", ")
        ),
        "foremost" if options::foremost_types(&o.extensions).is_some() => {
            "extract selected families and filter results".into()
        }
        "scalpel" => {
            "select matching installed signatures; unknown types fall back to result filtering"
                .into()
        }
        "magicrescue" => {
            "select matching installed recipes when all extensions map; otherwise filter results"
                .into()
        }
        "ntfsundelete" => "select matching file records before extraction".into(),
        "recoverjpeg" => "JPEG-only extraction; filter results".into(),
        "photorec" | "foremost" => {
            "not every extension maps to a native family; extract all types and filter results"
                .into()
        }
        _ => "filter results after extraction (no supported extraction filter)".into(),
    }
}
