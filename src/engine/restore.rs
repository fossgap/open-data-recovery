//! Explicit, destructive image-to-disk utility. Scan source access stays read-only.
use super::{Error, Result, fail, process, process::Control, safety};
use gtk::{gio, glib, prelude::*};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, Metadata, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

#[cfg(test)]
#[path = "restore_tests.rs"]
pub(crate) mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    device: u64,
    inode: u64,
    rdev: u64,
    size: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl From<Metadata> for Identity {
    fn from(m: Metadata) -> Self {
        Self {
            device: m.dev(),
            inode: m.ino(),
            rdev: m.rdev(),
            size: m.len(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Image {
    pub path: PathBuf,
    pub size: u64,
    identity: Identity,
    ancestry: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    pub path: PathBuf,
    pub model: String,
    pub serial: String,
    pub size: u64,
    pub sector_size: u64,
    pub unavailable: Option<String>,
    wwn: String,
    device: String,
    sys_path: PathBuf,
    sequence: u64,
    identity: Identity,
    ancestry: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub image: Image,
    pub disk: Disk,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Writing,
    Verifying,
}
pub enum Event {
    Progress {
        phase: Phase,
        bytes: u64,
        total: u64,
    },
    Finished(Result<Completed>),
}
pub struct Completed {
    pub bytes: u64,
    pub sha256: String,
}
pub struct Job {
    pub control: Control,
    pub events: Receiver<Event>,
}

fn string(node: &Value, key: &str) -> String {
    node[key].as_str().unwrap_or_default().trim().into()
}
fn number(node: &Value, key: &str) -> Result<u64> {
    node[key]
        .as_u64()
        .or_else(|| node[key].as_str()?.parse().ok())
        .ok_or_else(|| Error::Message(format!("Cannot verify disk property: {key}.")))
}

/// Include inactive disks in the picker with an explanation instead of hiding them.
pub fn disks() -> Result<Vec<Disk>> {
    let output = process::command("lsblk")
        .args([
            "--json",
            "--bytes",
            "--tree",
            "--output",
            "PATH,TYPE,SIZE,MODEL,SERIAL,WWN,LOG-SEC,MAJ:MIN,RO,MOUNTPOINTS",
        ])
        .env("LC_ALL", "C")
        .output()?;
    if !output.status.success() {
        return fail(format!(
            "Disk discovery failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let json: Value = serde_json::from_slice(&output.stdout)?;
    let nodes = json["blockdevices"]
        .as_array()
        .ok_or_else(|| Error::Message("Invalid disk discovery response.".into()))?;
    let mut disks = Vec::new();
    for node in nodes.iter().filter(|n| string(n, "type") == "disk") {
        // A failed identity/ancestry check excludes this disk, never makes it writable.
        if let Ok(disk) = disk_from_node(node) {
            disks.push(disk);
        }
    }
    Ok(disks)
}

pub(super) fn usage_problem(node: &Value) -> Option<String> {
    match &node["ro"] {
        Value::Bool(false) => {}
        Value::Number(n) if n.as_u64() == Some(0) => {}
        Value::String(s) if s == "0" => {}
        _ => {
            return Some(
                "The disk or a partition is read-only or its write status is unknown.".into(),
            );
        }
    }
    let Some(mounts) = node["mountpoints"].as_array() else {
        return Some("Cannot verify whether this disk is mounted.".into());
    };
    if mounts
        .iter()
        .any(|m| !m.is_null() && m.as_str() != Some(""))
    {
        return Some("The disk is in use. Unmount all its filesystems and deactivate swap before refreshing.".into());
    }
    if let Some(children) = node.get("children") {
        let Some(children) = children.as_array() else {
            return Some("Invalid disk topology.".into());
        };
        for child in children {
            if string(child, "type") != "part" {
                return Some(
                    "The disk has active RAID, LVM or encrypted mappings. Deactivate them first."
                        .into(),
                );
            }
            if let Some(problem) = usage_problem(child) {
                return Some(problem);
            }
        }
    }
    None
}

fn disk_from_node(node: &Value) -> Result<Disk> {
    let path = PathBuf::from(string(node, "path"));
    let meta = fs::symlink_metadata(&path)?;
    if !meta.file_type().is_block_device() || !path.starts_with("/dev") {
        return fail("The target must be a whole physical disk.");
    }
    let device = format!("{}:{}", libc::major(meta.rdev()), libc::minor(meta.rdev()));
    if device != string(node, "maj:min") {
        return fail("Disk identity changed during discovery.");
    }
    let sys_path = fs::canonicalize(Path::new("/sys/dev/block").join(&device))?;
    if sys_path.join("partition").exists() || sys_path.starts_with("/sys/devices/virtual") {
        return fail("Partitions and virtual devices cannot be restoration targets.");
    }
    let sequence = fs::read_to_string(sys_path.join("diskseq"))?
        .trim()
        .parse()
        .map_err(|_| Error::Message("Cannot verify the disk attachment identity.".into()))?;
    let mut unavailable = usage_problem(node);
    // Check kernel holders as well as lsblk's mount/mapping view, including partitions.
    let mut paths = vec![sys_path.clone()];
    for entry in fs::read_dir(&sys_path)? {
        let entry = entry?;
        if entry.path().join("partition").exists() {
            paths.push(entry.path());
        }
    }
    for path in paths {
        if fs::read_dir(path.join("holders"))?
            .next()
            .transpose()?
            .is_some()
        {
            unavailable = Some(
                "The disk has active holders. Deactivate RAID, LVM and encrypted mappings first."
                    .into(),
            );
        }
    }
    Ok(Disk {
        path,
        model: string(node, "model"),
        serial: string(node, "serial"),
        wwn: string(node, "wwn"),
        size: number(node, "size")?,
        sector_size: number(node, "log-sec")?,
        ancestry: safety::block_ancestry(&device)?,
        device,
        sys_path,
        sequence,
        identity: meta.into(),
        unavailable,
    })
}

pub(super) fn open_image(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    let identity = Identity::from(metadata.clone());
    if !metadata.is_file() || size < 512 {
        return fail("Choose a nonempty raw disk image of at least 512 bytes.");
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut header = [0; 512];
    use std::os::unix::fs::FileExt;
    file.read_exact_at(&mut header, 0)?;
    let mut footer = [0; 8];
    file.read_exact_at(&mut footer, size - 512)?;
    if matches!(
        ext.as_str(),
        "e01"
            | "ex01"
            | "ewf"
            | "qcow"
            | "qcow2"
            | "vmdk"
            | "vdi"
            | "vhd"
            | "vhdx"
            | "gz"
            | "xz"
            | "zst"
            | "bz2"
            | "zip"
            | "7z"
    ) || [
        b"EVF".as_slice(),
        b"LVF",
        b"EVF2",
        b"QFI\xfb",
        b"KDMV",
        b"vhdxfile",
        b"<<< Oracle VM VirtualBox",
        b"# Disk DescriptorFile",
        b"\x1f\x8b",
        b"\xfd7zXZ\0",
        b"BZh",
        b"PK\x03\x04",
        b"\x28\xb5\x2f\xfd",
        b"7z\xbc\xaf\x27\x1c",
    ]
    .iter()
    .any(|magic| header.starts_with(magic))
        || header[64..68] == [0x7f, 0x10, 0xda, 0xbe]
        || &footer == b"conectix"
    {
        return fail(
            "This utility accepts raw images only. Export E01, compressed or virtual-machine images to raw format first.",
        );
    }
    validate_image(&file, &identity)?;
    Ok(file)
}

pub fn inspect_image(path: &Path) -> Result<Image> {
    // Reject a symlink at selection, then pin the resolved path and inode at review.
    if !fs::symlink_metadata(path)?.is_file() {
        return fail("Choose a regular raw image file, not a symbolic link or device.");
    }
    let path = fs::canonicalize(path)?;
    let file = open_image(&path)?;
    let metadata = file.metadata()?;
    Ok(Image {
        size: metadata.len(),
        identity: metadata.into(),
        ancestry: safety::path_ancestry(&path)?,
        path,
    })
}

pub fn validate_pair(image: &Image, disk: &Disk) -> Result<()> {
    if let Some(problem) = &disk.unavailable {
        return fail(problem);
    }
    if image.ancestry.is_empty() || disk.ancestry.is_empty() {
        return fail("Cannot verify physical storage separation.");
    }
    if image.ancestry.iter().any(|id| disk.ancestry.contains(id)) {
        return fail("The image is stored on the selected disk. Choose a different target disk.");
    }
    if image.size > disk.size {
        return fail("The selected disk is smaller than the image.");
    }
    if !matches!(disk.sector_size, 512 | 4096)
        || image.size == 0
        || image.size % disk.sector_size != 0
    {
        return fail(
            "The image length must be a multiple of the target disk's supported logical sector size (512 or 4096 bytes).",
        );
    }
    Ok(())
}

fn current_disk(expected: &Disk) -> Result<Disk> {
    let current = disks()?
        .into_iter()
        .find(|d| d.path == expected.path)
        .ok_or_else(|| {
            Error::Message(
                "The selected disk is no longer available. Refresh and select it again.".into(),
            )
        })?;
    validate_disk_identity(expected, &current)?;
    Ok(current)
}

fn validate_disk_identity(expected: &Disk, current: &Disk) -> Result<()> {
    if current != expected {
        return fail("The disk identity or usage changed. Refresh and review the target again.");
    }
    Ok(())
}

pub fn prepare(image: &Image, disk: &Disk) -> Result<Plan> {
    let current = inspect_image(&image.path)?;
    if current.identity != image.identity || current.ancestry != image.ancestry {
        return fail("The image changed after selection. Choose it again.");
    }
    let disk = current_disk(disk)?;
    validate_pair(&current, &disk)?;
    Ok(Plan {
        image: current,
        disk,
    })
}

fn open_disk(disk: &Disk) -> Result<File> {
    let flags = libc::O_EXCL | libc::O_SYNC | libc::O_CLOEXEC | libc::O_NOFOLLOW;
    let opened = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(flags)
        .open(&disk.path);
    let file = match opened {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let mut escaped = String::new();
            for &byte in disk.path.file_name().unwrap_or_default().as_encoded_bytes() {
                if byte.is_ascii_alphanumeric() {
                    escaped.push(byte as char);
                } else {
                    escaped.push_str(&format!("_{byte:02x}"));
                }
            }
            let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
                .map_err(|e| Error::Message(format!("UDisks system bus: {e}")))?;
            let options = BTreeMap::from([("flags", flags.to_variant())]);
            let (reply, descriptors) = bus
                .call_with_unix_fd_list_sync(
                    Some("org.freedesktop.UDisks2"),
                    &format!("/org/freedesktop/UDisks2/block_devices/{escaped}"),
                    "org.freedesktop.UDisks2.Block",
                    "OpenDevice",
                    Some(&("rw", options).to_variant()),
                    None,
                    gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                    120000,
                    gio::UnixFDList::NONE,
                    gio::Cancellable::NONE,
                )
                .map_err(|e| {
                    Error::Message(format!(
                        "Disk write access was not authorized by UDisks/polkit: {e}"
                    ))
                })?;
            let handle = reply
                .child_value(0)
                .get::<glib::variant::Handle>()
                .ok_or_else(|| Error::Message("UDisks returned an invalid handle.".into()))?;
            File::from(
                descriptors
                    .ok_or_else(|| Error::Message("UDisks returned no descriptor.".into()))?
                    .get(handle.0)
                    .map_err(|e| Error::Message(e.to_string()))?,
            )
        }
        Err(e) => return Err(e.into()),
    };
    validate_descriptor(&file, disk)?;
    Ok(file)
}

fn ioctl_value<T: Default>(file: &File, request: libc::c_ulong) -> Result<T> {
    let mut value = T::default();
    if unsafe { libc::ioctl(file.as_raw_fd(), request, &mut value) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(value)
}
fn validate_descriptor(file: &File, disk: &Disk) -> Result<()> {
    let meta = file.metadata()?;
    if !meta.file_type().is_block_device() || Identity::from(meta) != disk.identity {
        return fail("The opened disk does not match the confirmed target.");
    }
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    // O_EXCL is checked by open() but stripped from F_GETFL by the kernel.
    if flags < 0 || flags & libc::O_ACCMODE != libc::O_RDWR || flags & libc::O_SYNC != libc::O_SYNC
    {
        return fail("Exclusive read/write access to the target was not granted.");
    }
    // Linux block ioctls: BLKGETSIZE64, BLKSSZGET, BLKROGET, BLKGETDISKSEQ.
    if ioctl_value::<u64>(file, 0x80081272)? != disk.size
        || ioctl_value::<u32>(file, 0x1268)? as u64 != disk.sector_size
        || ioctl_value::<u32>(file, 0x125e)? != 0
        || ioctl_value::<u64>(file, 0x80081280)? != disk.sequence
    {
        return fail(
            "The disk capacity, sector size, write status or attachment identity changed.",
        );
    }
    Ok(())
}

pub fn start(plan: Plan, confirmation: String) -> Job {
    let control = Control::default();
    let worker = control.clone();
    let (tx, events) = mpsc::sync_channel(32);
    std::thread::spawn(move || {
        let result = restore(plan, &confirmation, &worker, |phase, bytes, total| {
            let _ = tx.try_send(Event::Progress {
                phase,
                bytes,
                total,
            });
        });
        let _ = tx.send(Event::Finished(result));
    });
    Job { control, events }
}

fn restore(
    plan: Plan,
    confirmation: &str,
    control: &Control,
    mut progress: impl FnMut(Phase, u64, u64),
) -> Result<Completed> {
    validate_confirmation(&plan.disk.path, confirmation)?;
    control.checkpoint()?;
    let plan = prepare(&plan.image, &plan.disk)?;
    let mut source = open_image(&plan.image.path)?;
    validate_image(&source, &plan.image.identity)?;
    let mut target = open_disk(&plan.disk)?;
    // Polkit may have taken time; repeat topology checks while holding O_EXCL.
    let checked = prepare(&plan.image, &plan.disk)?;
    validate_descriptor(&target, &checked.disk)?;
    validate_image(&source, &checked.image.identity)?;
    control.checkpoint()?;
    process::confine_restore_thread()?;
    let copied = copy_image(
        &mut source,
        &mut target,
        plan.image.size,
        control,
        |bytes| progress(Phase::Writing, bytes, plan.image.size),
    );
    // Flush even on cancellation or a partial write; never report completion first.
    target.sync_all()?;
    let hash = copied?;
    validate_image(&source, &plan.image.identity)?;
    verify_image(&mut target, plan.image.size, &hash, control, |bytes| {
        progress(Phase::Verifying, bytes, plan.image.size)
    })?;
    Ok(Completed {
        bytes: plan.image.size,
        sha256: hash,
    })
}

pub(super) fn validate_confirmation(path: &Path, confirmation: &str) -> Result<()> {
    if path.to_str() != Some(confirmation) {
        return fail("Restoration confirmation does not match the exact target disk path.");
    }
    Ok(())
}
pub(super) fn validate_image(file: &File, identity: &Identity) -> Result<()> {
    if Identity::from(file.metadata()?) != *identity {
        return fail("The source image changed; restoration cannot continue.");
    }
    Ok(())
}

pub(super) fn copy_image(
    source: &mut impl Read,
    target: &mut impl Write,
    size: u64,
    control: &Control,
    mut progress: impl FnMut(u64),
) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut done = 0;
    progress(0);
    while done < size {
        control.checkpoint()?;
        let length = (size - done).min(buffer.len() as u64) as usize;
        source.read_exact(&mut buffer[..length])?;
        control.checkpoint()?;
        target.write_all(&buffer[..length])?;
        hash.update(&buffer[..length]);
        done += length as u64;
        progress(done);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn verify_image(
    target: &mut (impl Read + Seek),
    size: u64,
    expected: &str,
    control: &Control,
    mut progress: impl FnMut(u64),
) -> Result<()> {
    target.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut done = 0;
    progress(0);
    while done < size {
        control.checkpoint()?;
        let length = (size - done).min(buffer.len() as u64) as usize;
        target.read_exact(&mut buffer[..length])?;
        hash.update(&buffer[..length]);
        done += length as u64;
        progress(done);
    }
    if format!("{:x}", hash.finalize()) != expected {
        return fail("Verification failed: the written disk bytes do not match the image.");
    }
    control.checkpoint()
}
