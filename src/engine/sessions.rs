//! Saved-scan lifetime locks and confined deletion of an explicitly chosen scan.
use super::{
    Result, fail, read_json, safety,
    types::{Manifest, SavedScan},
};
use std::{
    ffi::{CString, OsStr},
    fs::{self, File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
};

/// Shared by scans, recovery, previews and catalogue readers; deletion takes an
/// exclusive lock on the directory inode. No lock file or legacy migration.
pub(crate) struct Lease(File);
impl Lease {
    pub(crate) fn shared(path: &Path) -> Result<Self> {
        Self::acquire(path, false)
    }
    fn acquire(path: &Path, exclusive: bool) -> Result<Self> {
        if safety::existing_parent(path)? != path {
            return fail("The saved-scan directory no longer exists.");
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(path)?;
        let mode = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        if unsafe { libc::flock(file.as_raw_fd(), mode | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                return fail(
                    "This saved scan is in use by a scan, recovery or another window. Try again when it finishes.",
                );
            }
            return Err(error.into());
        }
        Ok(Self(file))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // A concurrently forked child may briefly inherit the descriptor before
        // CLOEXEC takes effect, so explicitly release our lease on normal exit.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Clone)]
pub struct Deletion {
    pub scan: SavedScan,
    device: u64,
    inode: u64,
}
impl Deletion {
    pub fn prepare(scan: SavedScan) -> Result<Self> {
        let lease = Lease::acquire(&scan.directory, true)?;
        validate_manifest(&scan)?;
        let meta = lease.0.metadata()?;
        Ok(Self {
            scan,
            device: meta.dev(),
            inode: meta.ino(),
        })
    }
    pub fn execute(&self) -> Result<()> {
        let lease = Lease::acquire(&self.scan.directory, true)?;
        let meta = lease.0.metadata()?;
        if (meta.dev(), meta.ino()) != (self.device, self.inode) {
            return fail(
                "The saved-scan folder was replaced. Refresh the list before deleting it.",
            );
        }
        validate_manifest(&self.scan)?;
        let parent = self.scan.directory.parent().ok_or_else(|| {
            super::Error::Message("A workspace root cannot be deleted as a saved scan.".into())
        })?;
        let name = self
            .scan
            .directory
            .file_name()
            .ok_or_else(|| super::Error::Message("Invalid saved-scan directory.".into()))?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent)?;
        // Reject a scan directory that has itself become a mount point.
        let root = child_directory(&parent, name)?;
        if !same_directory(&root, &lease.0)? {
            return fail("The saved-scan directory changed before deletion.");
        }
        // Preflight the entire tree before unlinking anything. The deletion
        // pass repeats the same FD-relative, no-symlink/no-mount traversal.
        walk(&root, 0, false)?;
        let result = (|| {
            walk(&root, 0, true)?;
            for file in [
                "selection.sqlite",
                "tree.sqlite",
                "source.json",
                "manifest.json",
            ] {
                unlink(&root, OsStr::new(file), false, true)?;
            }
            if !same_directory(&child_directory(&parent, name)?, &root)? {
                return fail("The saved-scan directory changed during deletion.");
            }
            unlink(&parent, name, true, false)?;
            parent.sync_all()?;
            Ok(())
        })();
        result.map_err(|e: super::Error| super::Error::Message(format!(
            "Deletion did not finish: {e} Some contents may already have been removed. Remaining folder: {}", self.scan.directory.display())))
    }
}

fn validate_manifest(scan: &SavedScan) -> Result<()> {
    let manifest: Manifest = read_json(&safety::confined_artifact(
        &scan.directory,
        "manifest.json",
    )?)?;
    if manifest.id != scan.manifest.id
        || manifest.created_at != scan.manifest.created_at
        || manifest.source.path != scan.manifest.source.path
        || manifest.source.fingerprint != scan.manifest.source.fingerprint
    {
        return fail("The selected saved scan changed. Refresh the list before deleting it.");
    }
    if manifest.id.is_empty() {
        return fail("The selected folder has no saved-scan identity.");
    }
    // A previous deletion may have removed the catalogue before failing to
    // unlink the manifest/folder. Its manifest still identifies the remainder.
    safety::validate_storage_location(&manifest.source, &scan.directory)?;
    Ok(())
}

fn same_directory(a: &File, b: &File) -> Result<bool> {
    let (a, b) = (a.metadata()?, b.metadata()?);
    Ok((a.dev(), a.ino()) == (b.dev(), b.ino()))
}
fn name_c(name: &OsStr) -> Result<CString> {
    Ok(CString::new(name.as_bytes())
        .map_err(|_| super::Error::Message("Invalid snapshot filename.".into()))?)
}
fn child_directory(parent: &File, name: &OsStr) -> Result<File> {
    let name = name_c(name)?;
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64;
    how.resolve = libc::RESOLVE_BENEATH | libc::RESOLVE_NO_SYMLINKS | libc::RESOLVE_NO_XDEV;
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent.as_raw_fd(),
            name.as_ptr(),
            &how,
            std::mem::size_of::<libc::open_how>(),
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EXDEV) {
            return fail(
                "Saved-scan deletion cannot cross a nested mount point. Unmount it before deleting the scan.",
            );
        }
        return Err(error.into());
    }
    Ok(unsafe { File::from_raw_fd(fd as i32) })
}
fn unlink(parent: &File, name: &OsStr, directory: bool, allow_missing: bool) -> Result<()> {
    let name = name_c(name)?;
    if unsafe {
        libc::unlinkat(
            parent.as_raw_fd(),
            name.as_ptr(),
            if directory { libc::AT_REMOVEDIR } else { 0 },
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        if !(allow_missing && error.kind() == std::io::ErrorKind::NotFound) {
            return Err(error.into());
        }
    }
    Ok(())
}
fn walk(directory: &File, depth: usize, remove: bool) -> Result<()> {
    if depth > 128 {
        return fail("Saved-scan directory nesting exceeds 128 levels.");
    }
    let path = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            let child = child_directory(directory, &name)?;
            walk(&child, depth + 1, remove)?;
            if remove {
                if !same_directory(&child, &child_directory(directory, &name)?)? {
                    return fail("A snapshot directory changed during deletion.");
                }
                unlink(directory, &name, true, false)?;
            }
        } else if remove
            && !(depth == 0
                && [
                    "manifest.json",
                    "source.json",
                    "tree.sqlite",
                    "selection.sqlite",
                ]
                .iter()
                .any(|s| name == *s))
        {
            // unlinkat removes a symlink itself, never its target.
            unlink(directory, &name, false, false)?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn rejects_mount_boundary(parent: &Path, name: &OsStr) -> Result<File> {
    child_directory(&File::open(parent)?, name)
}
