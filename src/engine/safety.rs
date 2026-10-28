//! Source topology and directory-relative output operations. Unknown topology fails closed.
use super::{
    Error, Result, fail,
    types::{Conflict, SourceDescriptor},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    ffi::CString,
    fs::{self, File},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
};

fn device_key(device: u64) -> String {
    format!("{}:{}", libc::major(device), libc::minor(device))
}
fn cpath(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::Message("A path contains a NUL byte.".into()))
}

pub fn block_ancestry(device: &str) -> Result<Vec<String>> {
    fn visit(
        device: &str,
        visiting: &mut BTreeSet<String>,
        leaves: &mut BTreeSet<String>,
    ) -> Result<()> {
        if !visiting.insert(device.into()) {
            return fail("A cycle was found in the device graph.");
        }
        let sys = fs::canonicalize(Path::new("/sys/dev/block").join(device)).map_err(|_| {
            Error::Message(format!(
                "Cannot verify storage ancestry for device {device}."
            ))
        })?;
        let partition = sys.join("partition");
        let slaves: Vec<_> = if partition.exists() {
            vec![]
        } else {
            fs::read_dir(sys.join("slaves"))?.collect::<std::io::Result<Vec<_>>>()?
        };
        if partition.exists() {
            let parent = fs::read_to_string(
                sys.parent()
                    .ok_or_else(|| Error::Message("Missing partition parent.".into()))?
                    .join("dev"),
            )?;
            visit(parent.trim(), visiting, leaves)?;
        } else if !slaves.is_empty() {
            for slave in slaves {
                visit(
                    fs::read_to_string(slave.path().join("dev"))?.trim(),
                    visiting,
                    leaves,
                )?;
            }
        } else if sys.join("loop/backing_file").exists() {
            let backing = fs::read_to_string(sys.join("loop/backing_file"))?;
            let backing = PathBuf::from(format!("/{}", backing.trim().trim_start_matches('/')));
            leaves.extend(path_ancestry(&backing)?);
        } else {
            // Device IDs identify shared physical ancestry during this boot. Stable IDs are recorded separately.
            leaves.insert(format!("block:{device}"));
        }
        visiting.remove(device);
        Ok(())
    }
    let mut leaves = BTreeSet::new();
    visit(device, &mut BTreeSet::new(), &mut leaves)?;
    if leaves.is_empty() {
        return fail("The source device ancestry is unknown.");
    }
    Ok(leaves.into_iter().collect())
}

pub fn path_ancestry(path: &Path) -> Result<Vec<String>> {
    let metadata = fs::metadata(path)?;
    use std::os::unix::fs::FileTypeExt;
    if metadata.file_type().is_block_device() {
        return block_ancestry(&device_key(metadata.rdev()));
    }
    let output = super::process::command("findmnt")
        .args(["--json", "--target"])
        .arg(path)
        .args(["--output", "TARGET,SOURCE,FSTYPE,MAJ:MIN,UUID"])
        .output()?;
    if !output.status.success() {
        return fail("Cannot determine the destination filesystem.");
    }
    let json: Value = serde_json::from_slice(&output.stdout)?;
    let mount = json["filesystems"]
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| Error::Message("No filesystem identity was returned.".into()))?;
    let filesystem = mount["fstype"].as_str().unwrap_or("");
    let device = mount["maj:min"].as_str().unwrap_or("");
    if device != device_key(metadata.dev()) {
        return fail("Filesystem identity changed during validation. Choose the path again.");
    }
    if matches!(filesystem, "tmpfs" | "ramfs") {
        return Ok(vec![format!("memory:{device}")]);
    }
    if filesystem == "btrfs" {
        let uuid = mount["uuid"]
            .as_str()
            .ok_or_else(|| Error::Message("Cannot identify every btrfs member device.".into()))?;
        let mut devices = BTreeSet::new();
        for member in fs::read_dir(Path::new("/sys/fs/btrfs").join(uuid).join("devices"))? {
            let id = fs::read_to_string(member?.path().join("dev"))?;
            devices.extend(block_ancestry(id.trim())?);
        }
        if devices.is_empty() {
            return fail("No btrfs member devices could be verified.");
        }
        return Ok(devices.into_iter().collect());
    }
    if matches!(
        filesystem,
        "overlay" | "aufs" | "nfs" | "nfs4" | "cifs" | "fuse" | "fuseblk" | "9p"
    ) || device.starts_with("0:")
    {
        return fail(format!(
            "Cannot verify physical device ancestry for {filesystem}. Choose a directly mounted local filesystem."
        ));
    }
    block_ancestry(device)
}

/// Returns the nearest existing directory without following symlinks anywhere in the path.
pub fn existing_parent(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return fail(crate::i18n::tr(
            "Use an absolute destination/workspace path.",
        ));
    }
    let mut existing = PathBuf::from("/");
    let mut missing = false;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => {
                if missing {
                    continue;
                }
                let next = existing.join(name);
                match fs::symlink_metadata(&next) {
                    Ok(meta) if meta.file_type().is_symlink() => {
                        return fail(crate::i18n::tr(
                            "Storage paths must not contain symbolic links.",
                        ));
                    }
                    Ok(meta) if !meta.is_dir() => {
                        return fail(format!("{} is not a directory.", next.display()));
                    }
                    Ok(_) => existing = next,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => missing = true,
                    Err(error) => return Err(error.into()),
                }
            }
            _ => {
                return fail(crate::i18n::tr(
                    "Storage paths must not contain '..' components.",
                ));
            }
        }
    }
    Ok(existing)
}

pub fn free_bytes(path: &Path) -> Result<u64> {
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let path = cpath(path)?;
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

pub fn validate_storage(
    source: &SourceDescriptor,
    target: &Path,
    required_bytes: u64,
) -> Result<Vec<String>> {
    let target_devices = validate_storage_location(source, target)?;
    let existing = existing_parent(target)?;
    if free_bytes(&existing)? < required_bytes.saturating_add(8 * 1024 * 1024) {
        return fail(crate::i18n::tr(
            "The chosen storage does not have enough available space (including an 8 MiB reserve).",
        ));
    }
    Ok(target_devices)
}

/// Validate source separation without requiring free space. Removing a saved
/// scan must remain possible when its workspace is full.
pub fn validate_storage_location(source: &SourceDescriptor, target: &Path) -> Result<Vec<String>> {
    let existing = existing_parent(target)?;
    if source.ancestry.is_empty() {
        return fail("The source storage ancestry could not be verified.");
    }
    let target_devices = path_ancestry(&existing)?;
    if !source.is_image_file()
        && source
            .ancestry
            .iter()
            .any(|device| target_devices.contains(device))
    {
        return fail(crate::i18n::tr(
            "The workspace/destination shares a physical device with the source. Choose a separate drive (another partition on the same disk is not sufficient).",
        ));
    }
    // An image is an allocated file, not the free space of its hosting drive.
    // Keep every input (including E01 segments) outside output directories;
    // backend writes remain confined and recovery creates files with O_EXCL.
    for input in std::iter::once(&source.path).chain(source.segments.iter()) {
        if !input.is_absolute()
            || input
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return fail("The recorded source path is invalid.");
        }
        if target.starts_with(input) || input.starts_with(target) {
            return fail(crate::i18n::tr(
                "The source image, its segments and output folders must not contain each other. Choose a different output folder.",
            ));
        }
        if source.is_image_file() {
            match fs::symlink_metadata(input) {
                Ok(meta) => {
                    if !meta.is_file() || fs::canonicalize(input)? != *input {
                        return fail(
                            "Sharing an image's drive requires regular image files, without replaced paths, devices or symbolic links.",
                        );
                    }
                }
                // Saved scans may recover retained contents, rescan or be
                // deleted after their original image has been disconnected.
                // New scans still verify and open the original input first.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(target_devices)
}

pub struct GuardedDir {
    pub path: PathBuf,
    directory: File,
    device: u64,
    inode: u64,
}
impl GuardedDir {
    pub fn create(source: &SourceDescriptor, path: &Path, required: u64) -> Result<Self> {
        let expected = validate_storage(source, path, required)?;
        let parent = existing_parent(path)?;
        let parent_c = cpath(&parent)?;
        let fd = unsafe {
            libc::open(
                parent_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let mut directory = unsafe { File::from_raw_fd(fd) };
        let metadata = directory.metadata()?;
        if path_ancestry(&parent)? != expected
            || metadata.dev() != fs::metadata(&parent)?.dev()
            || metadata.ino() != fs::metadata(&parent)?.ino()
        {
            return fail("The chosen storage changed while being opened.");
        }
        let device = metadata.dev();
        for component in path
            .strip_prefix(&parent)
            .map_err(|_| Error::Message("Invalid output parent.".into()))?
            .components()
        {
            let Component::Normal(name) = component else {
                return fail("Invalid output directory component.");
            };
            directory = Self::child_directory(&directory, name.as_bytes(), device)?;
        }
        let inode = directory.metadata()?.ino();
        Ok(Self {
            path: path.into(),
            directory,
            device,
            inode,
        })
    }
    pub fn existing(path: &Path) -> Result<Self> {
        if existing_parent(path)? != path {
            return fail("The session directory does not exist.");
        }
        let path_c = cpath(path)?;
        let fd = unsafe {
            libc::open(
                path_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let directory = unsafe { File::from_raw_fd(fd) };
        let metadata = directory.metadata()?;
        Ok(Self {
            path: path.into(),
            directory,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    pub fn revalidate(&self, source: &SourceDescriptor, required: u64) -> Result<()> {
        validate_storage(source, &self.path, required)?;
        self.check_space(required)
    }
    /// Cheap checks between writes after ancestry has been validated. Output
    /// creation stays anchored to the directory FD and its original device.
    pub fn check_space(&self, required: u64) -> Result<()> {
        self.check_identity()?;
        if free_bytes(&self.path)? < required.saturating_add(8 * 1024 * 1024) {
            return fail("Insufficient workspace space, including the 8 MiB reserve.");
        }
        Ok(())
    }
    pub fn check_identity(&self) -> Result<()> {
        if existing_parent(&self.path)? != self.path {
            return fail("The approved directory is no longer available.");
        }
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.is_dir() || metadata.dev() != self.device || metadata.ino() != self.inode {
            return fail("The approved directory was replaced or remounted.");
        }
        Ok(())
    }
    fn child_directory(parent: &File, name: &[u8], device: u64) -> Result<File> {
        let name =
            CString::new(name).map_err(|_| Error::Message("Invalid directory name.".into()))?;
        let created = unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) };
        if created < 0
            && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        if file.metadata()?.dev() != device {
            return fail("A nested mount would cross the approved destination filesystem.");
        }
        Ok(file)
    }
    pub fn create_file(
        &self,
        original: &str,
        preserve_tree: bool,
        conflict: Conflict,
    ) -> Result<Option<(File, PathBuf)>> {
        let relative = safe_relative(original, preserve_tree)?;
        let mut directory = self.directory.try_clone()?;
        if let Some(parent) = relative.parent() {
            for component in parent.components() {
                let Component::Normal(name) = component else {
                    return fail("Invalid recovered directory.");
                };
                directory = Self::child_directory(&directory, name.as_bytes(), self.device)?;
            }
        }
        let name = relative
            .file_name()
            .ok_or_else(|| Error::Message("Candidate has no filename.".into()))?
            .to_string_lossy();
        for suffix in 0..10000 {
            let filename = if suffix == 0 {
                name.to_string()
            } else {
                let path = Path::new(name.as_ref());
                format!(
                    "{} ({suffix}){}",
                    path.file_stem().unwrap_or_default().to_string_lossy(),
                    path.extension()
                        .map(|ext| format!(".{}", ext.to_string_lossy()))
                        .unwrap_or_default()
                )
            };
            let encoded = CString::new(filename.as_bytes()).unwrap();
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    encoded.as_ptr(),
                    libc::O_RDWR
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd >= 0 {
                let file = unsafe { File::from_raw_fd(fd) };
                return Ok(Some((
                    file,
                    self.path
                        .join(relative.parent().unwrap_or(Path::new("")))
                        .join(filename),
                )));
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
            if conflict == Conflict::Skip {
                return Ok(None);
            }
        }
        fail("Too many filename conflicts in the destination.")
    }
    /// Create a link without following any destination component or replacing a name.
    pub fn create_link(
        &self,
        original: &str,
        preserve_tree: bool,
        conflict: Conflict,
        symbolic: Option<&str>,
        hardlink: Option<&File>,
    ) -> Result<Option<PathBuf>> {
        self.check_identity()?;
        let relative = safe_relative(original, preserve_tree)?;
        let mut directory = self.directory.try_clone()?;
        if let Some(parent) = relative.parent() {
            for component in parent.components() {
                let Component::Normal(name) = component else {
                    return fail("Invalid recovered directory.");
                };
                directory = Self::child_directory(&directory, name.as_bytes(), self.device)?;
            }
        }
        let name = relative
            .file_name()
            .ok_or_else(|| Error::Message("Missing link filename.".into()))?
            .to_string_lossy();
        let target = if let Some(target) = symbolic {
            if target.len() > 4096 {
                return fail("Symbolic link target exceeds 4096 bytes.");
            }
            CString::new(target)
                .map_err(|_| Error::Message("Invalid symbolic link target.".into()))?
        } else {
            let file =
                hardlink.ok_or_else(|| Error::Message("Missing hard-link source.".into()))?;
            if !file.metadata()?.is_file() || file.metadata()?.dev() != self.device {
                return fail(
                    "Hard links must refer to a recovered regular file on the destination filesystem.",
                );
            }
            CString::new(format!("/proc/self/fd/{}", file.as_raw_fd())).unwrap()
        };
        for suffix in 0..10000 {
            let filename = if suffix == 0 {
                name.to_string()
            } else {
                format!(
                    "{} ({suffix}){}",
                    Path::new(name.as_ref())
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    Path::new(name.as_ref())
                        .extension()
                        .map(|e| format!(".{}", e.to_string_lossy()))
                        .unwrap_or_default()
                )
            };
            let encoded = CString::new(filename.as_bytes()).unwrap();
            let result = unsafe {
                if symbolic.is_some() {
                    libc::symlinkat(target.as_ptr(), directory.as_raw_fd(), encoded.as_ptr())
                } else {
                    libc::linkat(
                        libc::AT_FDCWD,
                        target.as_ptr(),
                        directory.as_raw_fd(),
                        encoded.as_ptr(),
                        libc::AT_SYMLINK_FOLLOW,
                    )
                }
            };
            if result == 0 {
                return Ok(Some(
                    self.path
                        .join(relative.parent().unwrap_or(Path::new("")))
                        .join(filename),
                ));
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error.into());
            }
            if conflict == Conflict::Skip {
                return Ok(None);
            }
        }
        fail("Too many filename conflicts in the destination.")
    }
}

pub fn safe_relative(name: &str, preserve_tree: bool) -> Result<PathBuf> {
    let trimmed = name.trim_start_matches('/');
    if trimmed.is_empty() {
        return fail("Empty candidate path.");
    }
    let mut result = PathBuf::new();
    for component in Path::new(trimmed).components() {
        let Component::Normal(value) = component else {
            return fail("A candidate path contains unsafe traversal components.");
        };
        let mut sanitized: String = value
            .to_string_lossy()
            .chars()
            .map(|c| {
                if c.is_control() || c == ':' || c == '\\' {
                    '_'
                } else {
                    c
                }
            })
            .take(120)
            .collect();
        while sanitized.len() > 180 {
            sanitized.pop();
        }
        if sanitized.is_empty() {
            return fail("Empty filename component.");
        }
        result.push(sanitized);
    }
    if !preserve_tree {
        result = PathBuf::from(
            result
                .file_name()
                .ok_or_else(|| Error::Message("Empty filename.".into()))?,
        );
    }
    Ok(result)
}

pub fn confined_artifact(root: &Path, relative: &str) -> Result<PathBuf> {
    if Path::new(relative).is_absolute()
        || Path::new(relative)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return fail("Invalid artifact path in saved scan.");
    }
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        path.push(component);
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return fail("Saved artifacts may not be symbolic links.");
        }
    }
    if !fs::metadata(&path)?.is_file() {
        return fail("Saved artifact is not a regular file.");
    }
    Ok(path)
}
