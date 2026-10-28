//! Validation and deterministic argument fragments shared by UI plans and adapters.
use super::{Result, fail, types::*};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
};

pub const FILESYSTEMS: &[&str] = &[
    "auto", "ext2", "ext3", "ext4", "ntfs", "fat12", "fat16", "fat32", "exfat", "btrfs",
];
pub const PHOTO_FAMILIES: &[&str] = &[
    "jpg", "png", "gif", "pdf", "bmp", "mp3", "flac", "ogg", "7z", "rar", "gz", "bz2", "exe",
    "sqlite", "zip", "doc", "riff", "mov", "tif",
];
pub fn filesystem<'a>(source: &'a SourceDescriptor, options: &'a ScanOptions) -> &'a str {
    if options.interpretation.filesystem == "auto" {
        &source.filesystem
    } else {
        &options.interpretation.filesystem
    }
}
pub fn sector(source: &SourceDescriptor, options: &ScanOptions) -> u32 {
    options
        .interpretation
        .sector_size
        .unwrap_or(source.sector_size)
}
pub fn relative_target(value: &str) -> bool {
    !value.is_empty()
        && !Path::new(value).is_absolute()
        && !value.contains(['\n', '\r', '\0'])
        && Path::new(value)
            .components()
            .all(|p| matches!(p, Component::Normal(_)))
}
pub fn inode_list(value: &str, ranges: bool) -> bool {
    !value.is_empty()
        && value.split(',').all(|part| {
            let numbers: Vec<_> = part.split('-').collect();
            (numbers.len() == 1 || ranges && numbers.len() == 2)
                && numbers
                    .iter()
                    .all(|n| n.parse::<u64>().is_ok_and(|n| n > 0))
                && (numbers.len() != 2
                    || numbers[0].parse::<u64>().unwrap() <= numbers[1].parse::<u64>().unwrap())
        })
}
pub fn validate(o: &ScanOptions) -> Result<()> {
    if o.photorec
        .families
        .iter()
        .any(|f| !PHOTO_FAMILIES.contains(&f.as_str()))
    {
        return fail(format!(
            "Choose PhotoRec families from: {}",
            PHOTO_FAMILIES.join(", ")
        ));
    }
    if !FILESYSTEMS.contains(&o.interpretation.filesystem.as_str()) {
        return fail("Choose a supported filesystem interpretation.");
    }
    if o.interpretation
        .sector_size
        .is_some_and(|n| ![512, 1024, 2048, 4096].contains(&n))
    {
        return fail("Sector size must be 512, 1024, 2048 or 4096 bytes.");
    }
    if o.photorec
        .block_size
        .is_some_and(|n| n < 512 || !n.is_power_of_two() || n > 1024 * 1024)
    {
        return fail("PhotoRec block size must be a power of two from 512 to 1048576 bytes.");
    }
    if o.photorec.block_offset != 0 {
        return fail(
            "PhotoRec scripted runs do not expose block alignment; use the source byte range instead.",
        );
    }
    if o.ddrescue.max_read_rate != 0 && o.ddrescue.min_read_rate > o.ddrescue.max_read_rate {
        return fail("The slow-read threshold cannot exceed the maximum read rate.");
    }
    if o.ddrescue.import_image.is_some() != o.ddrescue.import_map.is_some() {
        return fail("Select both the existing image and its ddrescue map.");
    }
    if o.ddrescue
        .import_image
        .iter()
        .chain(o.ddrescue.import_map.iter())
        .any(|p| !p.is_absolute())
    {
        return fail("Imported image and map paths must be absolute.");
    }
    if !["all", "file", "directory", "inode"].contains(&o.ext.target_kind.as_str()) {
        return fail("Invalid ext recovery target type.");
    }
    if o.ext.target_kind == "inode" && !inode_list(&o.ext.target, false) {
        return fail("Enter positive inode numbers separated by commas.");
    }
    if matches!(o.ext.target_kind.as_str(), "file" | "directory") && !relative_target(&o.ext.target)
    {
        return fail(
            "The ext target must be a path relative to the filesystem root, without traversal.",
        );
    }
    if o.ext.after.zip(o.ext.before).is_some_and(|(a, b)| a >= b) {
        return fail("The start time must precede the end time.");
    }
    if o.ext.journal.as_ref().is_some_and(|p| !p.is_absolute()) {
        return fail("Use an absolute path for the saved journal.");
    }
    if o.ext
        .block_size
        .is_some_and(|n| ![1024, 2048, 4096, 8192, 16384, 32768, 65536].contains(&n))
    {
        return fail("Invalid ext filesystem block size.");
    }
    if o.btrfs.superblock > 2 {
        return fail("Btrfs superblock mirror must be 0, 1 or 2.");
    }
    if o.btrfs.subvolume.is_some() && o.btrfs.subvolume_root.is_some() {
        return fail("Choose a Btrfs subvolume ID or subvolume root address, not both.");
    }
    if !o.btrfs.path.is_empty() && !relative_target(&o.btrfs.path) {
        return fail("Enter a Btrfs directory relative to its filesystem root.");
    }
    if o.ntfs.minimum_percent > 100 {
        return fail("NTFS recoverable percentage must be between 0 and 100.");
    }
    if !o.ntfs.inodes.is_empty() && !inode_list(&o.ntfs.inodes, true) {
        return fail("NTFS inodes must be numbers or ranges, for example 42,70-80.");
    }
    if o.ntfs.pattern.contains(['\0', '\n', '\r']) || o.ntfs.pattern.len() > 1024 {
        return fail("Invalid NTFS filename pattern.");
    }
    let mut seen = BTreeSet::new();
    for name in &o.metadata_sequence {
        if name == "auto"
            || !super::adapters::METADATA.contains(&name.as_str())
            || !seen.insert(name)
        {
            return fail("The metadata sequence must contain distinct engine names.");
        }
    }
    Ok(())
}

/// Known PhotoRec families. Unknown suffixes keep all families so no request is silently dropped.
pub fn photorec_families(extensions: &[String]) -> Option<Vec<String>> {
    if extensions.is_empty() {
        return None;
    }
    let mut families = BTreeSet::new();
    for extension in extensions {
        let family = match extension.as_str() {
            "jpeg" | "jpe" | "jpg" => "jpg",
            "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "epub" | "jar" | "zip" => "zip",
            "doc" | "xls" | "ppt" | "vsd" => "doc",
            "avi" | "wav" | "cdr" | "webp" => "riff",
            "mp4" | "mov" | "m4v" | "3gp" | "m4a" => "mov",
            "tif" | "tiff" | "dng" | "cr2" | "nef" => "tif",
            "png" | "gif" | "pdf" | "bmp" | "mp3" | "flac" | "ogg" | "7z" | "rar" | "gz"
            | "bz2" | "exe" | "sqlite" => extension,
            _ => return None,
        };
        families.insert(family.to_string());
    }
    Some(families.into_iter().collect())
}
pub fn photorec_command(o: &ScanOptions, fs: &str, sector: u32) -> String {
    let mut command = format!("partition_none,geometry,N,{sector},options,");
    if fs.starts_with("ext") {
        command.push_str("mode_ext2,");
    }
    if o.low_memory {
        command.push_str("lowmem,");
    }
    command.push_str(if o.photorec.brute_force {
        "paranoid_bf,"
    } else {
        "paranoid,"
    });
    command.push_str(if o.keep_corrupted {
        "keep_corrupted_file,"
    } else {
        "keep_corrupted_file_no,"
    });
    command.push_str("fileopt,everything,");
    if let Some(families) = if o.photorec.families.is_empty() {
        photorec_families(&o.extensions)
    } else {
        Some(o.photorec.families.clone())
    } {
        command.push_str("disable,");
        for family in families {
            command.push_str(&format!("{family},enable,"));
        }
    } else {
        command.push_str("enable,");
    }
    if let Some(size) = o.photorec.block_size {
        command.push_str(&format!("blocksize,{size},"));
    }
    command.push_str(if o.photorec.free_space {
        "freespace,search"
    } else {
        "wholespace,search"
    });
    command
}
pub fn foremost_types(extensions: &[String]) -> Option<String> {
    let mut types = BTreeSet::new();
    if extensions.is_empty() {
        return None;
    }
    for e in extensions {
        types.insert(match e.as_str() {
            "jpeg" | "jpg" => "jpg",
            "doc" | "xls" | "ppt" => "ole",
            "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp" | "zip" => "zip",
            "png" | "gif" | "bmp" | "avi" | "exe" | "mpg" | "wav" | "riff" | "wmv" | "mov"
            | "pdf" | "rar" | "htm" => e.as_str(),
            _ => return None,
        });
    }
    Some(types.into_iter().collect::<Vec<_>>().join(","))
}
pub fn regex_literal(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ".[](){}*+?^$|\\".contains(ch) {
            result.push('\\');
        }
        result.push(ch);
    }
    result
}
pub fn btrfs_regex(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    // Match ancestor directories as well as selected leaves, allowing restore to descend.
    let parts: Vec<_> = path.split('/').map(regex_literal).collect();
    let mut tail = "(|/.*)".to_string();
    for component in parts.iter().skip(1).rev() {
        tail = format!("(|/{component}{tail})");
    }
    Some(format!("^/(|{}{})$", parts[0], tail))
}
