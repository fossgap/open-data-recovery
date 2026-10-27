use super::{
    Error, Result, fail, safety,
    types::{Fingerprint, SourceDescriptor, SourceType},
};
use gtk::{gio, glib, prelude::*};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};

fn text(value: &Value, name: &str) -> String {
    value[name].as_str().unwrap_or_default().trim().into()
}
fn number(value: &Value, name: &str) -> u64 {
    value[name]
        .as_u64()
        .or_else(|| value[name].as_str()?.parse().ok())
        .unwrap_or(0)
}

pub fn discover() -> Result<Vec<SourceDescriptor>> {
    let output = super::process::command("lsblk").args(["--json", "--bytes", "--tree", "--output", "NAME,PATH,TYPE,SIZE,FSTYPE,LABEL,UUID,PARTUUID,START,PKNAME,MOUNTPOINTS,MODEL,SERIAL,WWN,LOG-SEC,MAJ:MIN"]).output()?;
    if !output.status.success() {
        return fail(format!(
            "Device discovery failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let json: Value = serde_json::from_slice(&output.stdout)?;
    let mut sources = Vec::new();
    fn add(
        node: &Value,
        parent: Option<PathBuf>,
        inherited_ids: &[String],
        sources: &mut Vec<SourceDescriptor>,
    ) {
        let path = PathBuf::from(text(node, "path"));
        let mut drive_ids = inherited_ids.to_vec();
        if parent.is_none() {
            drive_ids.push(format!(
                "{}|{}|{}|{}",
                text(node, "wwn"),
                text(node, "serial"),
                text(node, "model"),
                number(node, "size")
            ));
        }
        let mountpoints: Vec<PathBuf> = node["mountpoints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|s| s.starts_with('/'))
            .map(PathBuf::from)
            .collect();
        let label = text(node, "label");
        let model = text(node, "model");
        let name = if !label.is_empty() {
            label
        } else if !model.is_empty() {
            model
        } else {
            text(node, "name")
        };
        let filesystem = text(node, "fstype");
        let sector_size = number(node, "log-sec").max(512) as u32;
        let fingerprint = Fingerprint {
            size: number(node, "size"),
            filesystem_uuid: text(node, "uuid"),
            partition_uuid: text(node, "partuuid"),
            partition_offset: number(node, "start").saturating_mul(512),
            drive_ids: drive_ids.clone(),
            sample_sha256: String::new(),
        };
        let ancestry = safety::block_ancestry(&text(node, "maj:min")).unwrap_or_default();
        sources.push(SourceDescriptor {
            path: path.clone(),
            name,
            kind: SourceType::Block,
            filesystem,
            size: fingerprint.size,
            sector_size,
            mounted: !mountpoints.is_empty(),
            parent_path: parent,
            mountpoints,
            scope: None,
            fingerprint,
            ancestry,
            ewf: false,
            segments: vec![],
            partition: None,
        });
        if let Some(children) = node["children"].as_array() {
            for child in children {
                add(child, Some(path.clone()), &drive_ids, sources);
            }
        }
    }
    for node in json["blockdevices"]
        .as_array()
        .ok_or_else(|| Error::Message("Invalid lsblk response.".into()))?
    {
        add(node, None, &[], &mut sources);
    }
    let btrfs_members: BTreeMap<String, BTreeSet<String>> = sources
        .iter()
        .filter(|s| s.filesystem == "btrfs")
        .fold(BTreeMap::new(), |mut map, source| {
            map.entry(source.fingerprint.filesystem_uuid.clone())
                .or_default()
                .extend(source.ancestry.clone());
            map
        });
    for source in &mut sources {
        if source.filesystem == "btrfs" {
            source.ancestry = btrfs_members
                .get(&source.fingerprint.filesystem_uuid)
                .map(|set| set.iter().cloned().collect())
                .unwrap_or_default();
        }
    }
    Ok(sources)
}

pub fn inspect(path: &Path) -> Result<SourceDescriptor> {
    let path = fs::canonicalize(path)?;
    let metadata = fs::metadata(&path)?;
    if metadata.is_dir() {
        let output = super::process::command("findmnt")
            .args(["--json", "--target"])
            .arg(&path)
            .args(["--output", "TARGET,SOURCE,FSTYPE,FSROOT"])
            .output()?;
        let json: Value = serde_json::from_slice(&output.stdout)?;
        let mount = json["filesystems"]
            .as_array()
            .and_then(|list| list.first())
            .ok_or_else(|| {
                Error::Message("Cannot resolve this folder's local filesystem.".into())
            })?;
        let device = text(mount, "source");
        let device = device.split('[').next().unwrap_or("");
        if !device.starts_with("/dev/") {
            return fail("Choose a folder on a mounted local block filesystem.");
        }
        let mut source = inspect(Path::new(device))?;
        let relative = path
            .strip_prefix(text(mount, "target"))
            .map_err(|_| Error::Message("Mount boundary changed.".into()))?;
        let fsroot = PathBuf::from(text(mount, "fsroot"));
        source.scope = Some(format!(
            "/{}",
            fsroot
                .join(relative)
                .to_string_lossy()
                .trim_start_matches('/')
        ));
        source.kind = SourceType::Folder;
        source.name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into();
        source.mounted = true;
        return Ok(source);
    }
    if metadata.file_type().is_block_device() {
        return discover()?
            .into_iter()
            .find(|source| fs::canonicalize(&source.path).is_ok_and(|p| p == path))
            .ok_or_else(|| {
                Error::Message("The block device was not found in current device discovery.".into())
            });
    }
    if !metadata.is_file() || metadata.len() == 0 {
        return fail("Choose a nonempty raw/dd/E01 image, block device, or mounted local folder.");
    }
    let ewf = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("e01") || ext.eq_ignore_ascii_case("ex01"));
    let mut segments = vec![path.clone()];
    if ewf {
        let stem = path.file_stem().unwrap_or_default();
        segments = fs::read_dir(path.parent().unwrap())?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|p| {
                p.file_stem() == Some(stem)
                    && p.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| {
                            let ext = ext.to_ascii_lowercase();
                            (ext.len() == 3 && ext.starts_with('e'))
                                || (ext.len() == 4 && ext.starts_with("ex"))
                        })
            })
            .collect();
        segments.sort();
    }
    let mut ancestry = BTreeSet::new();
    let mut digest = Sha256::new();
    for segment in &segments {
        if fs::symlink_metadata(segment)?.file_type().is_symlink() {
            return fail("E01 segments may not be symbolic links.");
        }
        ancestry.extend(safety::path_ancestry(segment)?);
        let mut file = File::open(segment)?;
        let size = file.metadata()?.len();
        digest.update(size.to_le_bytes());
        let mut buffer = vec![0; 65536];
        let count = file.read(&mut buffer)?;
        digest.update(&buffer[..count]);
        file.seek(SeekFrom::Start(size.saturating_sub(65536)))?;
        let count = file.read(&mut buffer)?;
        digest.update(&buffer[..count]);
    }
    let mut filesystem = String::new();
    let mut uuid = String::new();
    if !ewf {
        if let Ok(output) = super::process::command("blkid")
            .args(["--probe", "--output", "export"])
            .arg(&path)
            .output()
        {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if let Some(value) = line.strip_prefix("TYPE=") {
                    filesystem = value.into();
                }
                if let Some(value) = line.strip_prefix("UUID=") {
                    uuid = value.into();
                }
            }
        }
    }
    let mut size = metadata.len();
    if ewf {
        let output = super::process::command("ewfinfo")
            .arg(&path)
            .output()
            .map_err(|error| Error::Message(format!("E01 access requires ewf-tools: {error}")))?;
        if !output.status.success() {
            return fail(format!(
                "Cannot read E01 metadata: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if line.trim_start().starts_with("Media size:") {
                if let Some((_, bytes)) = line.rsplit_once('(') {
                    if let Some(number) = bytes
                        .split_whitespace()
                        .next()
                        .and_then(|s| s.parse::<u64>().ok())
                    {
                        size = number;
                    }
                }
            }
        }
    }
    Ok(SourceDescriptor {
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into(),
        path,
        kind: SourceType::Image,
        filesystem,
        size,
        sector_size: 512,
        mounted: false,
        parent_path: None,
        mountpoints: vec![],
        scope: None,
        fingerprint: Fingerprint {
            size,
            filesystem_uuid: uuid,
            sample_sha256: format!("{:x}", digest.finalize()),
            ..Default::default()
        },
        ancestry: ancestry.into_iter().collect(),
        ewf,
        segments,
        partition: None,
    })
}

pub fn verify(source: &SourceDescriptor) -> Result<SourceDescriptor> {
    let mut current = inspect(&source.path)?;
    current.scope = source.scope.clone();
    current.kind = source.kind.clone();
    current.name = source.name.clone();
    if current.fingerprint != source.fingerprint {
        return fail(
            "The source identity or image fingerprint changed. Start a new scan for this source.",
        );
    }
    if let Some(partition) = &source.partition {
        partition.validate(current.size)?;
        let input = open_readonly(&current)?;
        if super::discovery::sample(&input, partition)? != partition.sample_sha256 {
            return fail(
                "The discovered partition's contents changed. Search for partitions again.",
            );
        }
        current = partition_source(&current, partition)?;
        current.name = source.name.clone();
    }
    Ok(current)
}

pub fn partition_source(
    source: &SourceDescriptor,
    partition: &super::types::DiscoveredPartition,
) -> Result<SourceDescriptor> {
    if source.kind == SourceType::Folder
        || source.scope.is_some()
        || source.partition.is_some()
        || source.ewf
    {
        return fail("Choose the original raw disk image or device for this partition.");
    }
    partition.validate(source.size)?;
    if partition.sector_size != source.sector_size {
        return fail("The discovered partition's sector size differs from its source.");
    }
    if partition.sample_sha256.len() != 64
        || !partition
            .sample_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return fail("The discovered partition has no valid source fingerprint.");
    }
    let mut target = source.clone();
    target.name = format!(
        "Discovered {} · byte {}{}",
        if partition.filesystem.is_empty() {
            "partition"
        } else {
            &partition.filesystem
        },
        partition.offset_bytes,
        if partition.label.is_empty() {
            String::new()
        } else {
            format!(" · {}", partition.label)
        }
    );
    target.filesystem = partition.filesystem.clone();
    target.sector_size = partition.sector_size;
    target.parent_path = Some(source.path.clone());
    target.partition = Some(partition.clone());
    Ok(target)
}

pub fn open_readonly(source: &SourceDescriptor) -> Result<File> {
    let result = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&source.path);
    let file = match result {
        Ok(file) => file,
        Err(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied
                && source.kind != SourceType::Image =>
        {
            let device = fs::canonicalize(&source.path)?;
            let name = device
                .file_name()
                .ok_or_else(|| Error::Message("Invalid device path.".into()))?
                .as_encoded_bytes();
            let mut escaped = String::new();
            for &byte in name {
                if byte.is_ascii_alphanumeric() {
                    escaped.push(byte as char);
                } else {
                    escaped.push_str(&format!("_{byte:02x}"));
                }
            }
            let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
                .map_err(|e| Error::Message(format!("UDisks system bus: {e}")))?;
            let options: BTreeMap<String, glib::Variant> = BTreeMap::new();
            let params = ("r", options).to_variant();
            let (reply, descriptors) = bus
                .call_with_unix_fd_list_sync(
                    Some("org.freedesktop.UDisks2"),
                    &format!("/org/freedesktop/UDisks2/block_devices/{escaped}"),
                    "org.freedesktop.UDisks2.Block",
                    "OpenDevice",
                    Some(&params),
                    None,
                    gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                    120000,
                    gio::UnixFDList::NONE,
                    gio::Cancellable::NONE,
                )
                .map_err(|e| {
                    Error::Message(format!(
                        "Read-only source access was not authorized by UDisks/polkit: {e}"
                    ))
                })?;
            let handle = reply
                .child_value(0)
                .get::<glib::variant::Handle>()
                .ok_or_else(|| Error::Message("UDisks returned an invalid handle.".into()))?;
            let fd = descriptors
                .ok_or_else(|| Error::Message("UDisks returned no descriptor.".into()))?
                .get(handle.0)
                .map_err(|e| Error::Message(e.to_string()))?;
            File::from(fd)
        }
        Err(error) => return Err(error.into()),
    };
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) } & libc::O_ACCMODE != libc::O_RDONLY {
        return fail("Source descriptor is not read-only.");
    }
    Ok(file)
}
