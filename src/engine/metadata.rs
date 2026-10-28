//! Optional file metadata. Missing engine evidence stays missing; metadata failures are reported.
use super::{Error, Result, fail, process::Control, types::LinuxMetadata};
use std::{
    ffi::CString,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::Path,
};

/// Normalize only private engine output. O_PATH avoids opening unreadable data,
/// and the pinned descriptor prevents chmod from following a replaced path.
pub fn prepare_artifact(path: &Path, directory: bool) -> Result<()> {
    let handle = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
        .open(path)?;
    let meta = handle.metadata()?;
    if !(if directory {
        meta.is_dir()
    } else {
        meta.is_file()
    }) {
        return fail("Engine output changed type before importing.");
    }
    fs::set_permissions(
        format!("/proc/self/fd/{}", handle.as_raw_fd()),
        fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
    )?;
    Ok(())
}

pub fn capture_artifact(path: &Path) -> Result<LinuxMetadata> {
    match capture(path) {
        Err(Error::Io(e)) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let handle = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_PATH | libc::O_NOFOLLOW)
                .open(path)?;
            let original = handle.metadata()?;
            if !original.is_file() {
                return fail("Engine output changed type before importing.");
            }
            let mode = original.mode() & 0o7777;
            let pinned = format!("/proc/self/fd/{}", handle.as_raw_fd());
            fs::set_permissions(&pinned, fs::Permissions::from_mode(mode | 0o400))?;
            let mut metadata = capture(path)?;
            metadata.mode = Some(mode);
            // Adding owner-read changes only ACL_USER_OBJ. Keep the captured
            // ACL's original owner permissions; named entries and mask are intact.
            if let Some(acl) = metadata.xattrs.get_mut("system.posix_acl_access") {
                if acl.len() >= 4 && acl[..4] == 2u32.to_le_bytes() && (acl.len() - 4) % 8 == 0 {
                    for entry in acl[4..].chunks_exact_mut(8) {
                        if u16::from_le_bytes([entry[0], entry[1]]) == 1 {
                            entry[2..4].copy_from_slice(&((mode >> 6 & 7) as u16).to_le_bytes());
                        }
                    }
                }
            }
            Ok(metadata)
        }
        result => result,
    }
}

pub fn from_body(mode: &str, uid: &str, gid: &str, identity: String) -> LinuxMetadata {
    let bits = mode.chars().rev().take(9).collect::<Vec<_>>();
    let mut permission = 0u32;
    for (i, c) in bits.iter().enumerate() {
        if matches!(c, 'r' | 'w' | 'x' | 's' | 't') {
            permission |= 1 << i;
        }
    }
    LinuxMetadata {
        mode: (bits.len() == 9).then_some(permission),
        uid: uid.parse().ok(),
        gid: gid.parse().ok(),
        hardlink: Some(identity),
        symbolic: mode.starts_with('l') || mode.contains("/l"),
        ..Default::default()
    }
}
pub fn capture(path: &Path) -> Result<LinuxMetadata> {
    let m = fs::symlink_metadata(path)?;
    let mut metadata = LinuxMetadata {
        mode: Some(m.mode() & 0o7777),
        uid: Some(m.uid()),
        gid: Some(m.gid()),
        symbolic: m.file_type().is_symlink(),
        hardlink: (m.nlink() > 1 && !m.file_type().is_symlink())
            .then(|| format!("{}:{}", m.dev(), m.ino())),
        ..Default::default()
    };
    if metadata.symbolic {
        metadata.link_target = Some(fs::read_link(path)?.to_string_lossy().into_owned());
        return Ok(metadata);
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let n = unsafe { libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
    if n < 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::ENOTSUP) {
            return Ok(metadata);
        }
        return Err(e.into());
    }
    if n > 1024 * 1024 {
        return fail("File extended attributes exceed 1 MiB.");
    }
    let mut names = vec![0; n as usize];
    let n = unsafe { libc::flistxattr(file.as_raw_fd(), names.as_mut_ptr().cast(), names.len()) };
    if n < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut total = 0;
    for name in names[..n as usize]
        .split(|b| *b == 0)
        .filter(|n| !n.is_empty())
    {
        let text = String::from_utf8_lossy(name).into_owned();
        let key =
            CString::new(name).map_err(|_| Error::Message("Invalid attribute name.".into()))?;
        let size =
            unsafe { libc::fgetxattr(file.as_raw_fd(), key.as_ptr(), std::ptr::null_mut(), 0) };
        if size < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        total += size as usize;
        if total > 256 * 1024 {
            return fail("File extended attributes exceed the 256 KiB capture limit.");
        }
        let mut data = vec![0; size as usize];
        let size = unsafe {
            libc::fgetxattr(
                file.as_raw_fd(),
                key.as_ptr(),
                data.as_mut_ptr().cast(),
                data.len(),
            )
        };
        if size < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        data.truncate(size as usize);
        metadata.xattrs.insert(text, data);
    }
    Ok(metadata)
}
pub fn apply(file: &File, metadata: &LinuxMetadata) -> Vec<String> {
    let mut warnings = vec![];
    if metadata.uid.is_some() || metadata.gid.is_some() {
        if unsafe {
            libc::fchown(
                file.as_raw_fd(),
                metadata.uid.unwrap_or(u32::MAX),
                metadata.gid.unwrap_or(u32::MAX),
            )
        } < 0
        {
            warnings.push(format!(
                "Ownership not restored: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    if let Some(mode) = metadata.mode {
        // Executable permissions are useful; privilege-elevating bits are not restored.
        if let Err(e) = file.set_permissions(fs::Permissions::from_mode(mode & 0o777)) {
            warnings.push(format!("Permissions not restored: {e}"));
        }
        if mode & 0o7000 != 0 {
            warnings.push("Special permission bits were omitted.".into());
        }
    }
    for (key, data) in &metadata.xattrs {
        // User data and POSIX ACLs are recoverable without restoring security capabilities.
        if !key.starts_with("user.") && key != "system.posix_acl_access" {
            warnings.push(format!("Attribute {key} was recorded but not applied."));
            continue;
        }
        let Ok(key_c) = CString::new(key.as_bytes()) else {
            warnings.push("Invalid attribute name.".into());
            continue;
        };
        if unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                key_c.as_ptr(),
                data.as_ptr().cast(),
                data.len(),
                0,
            )
        } < 0
        {
            warnings.push(format!(
                "Attribute {key} not restored: {}",
                std::io::Error::last_os_error()
            ));
        }
    }
    warnings
}
pub fn copy(input: &mut File, output: &mut File, sparse: bool, control: &Control) -> Result<u64> {
    let mut buffer = vec![0; 1024 * 1024];
    let mut total = 0;
    loop {
        control.checkpoint()?;
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        if sparse && buffer[..n].iter().all(|b| *b == 0) {
            output.seek(SeekFrom::Current(n as i64))?;
        } else {
            output.write_all(&buffer[..n])?;
        }
        total += n as u64;
    }
    output.set_len(total)?;
    Ok(total)
}

pub fn sparsify(file: &mut File, control: &Control) -> Result<()> {
    let size = file.metadata()?.len();
    file.seek(SeekFrom::Start(0))?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut offset = 0u64;
    while offset < size {
        control.checkpoint()?;
        let want = buffer.len().min((size - offset) as usize);
        file.read_exact(&mut buffer[..want])?;
        if buffer[..want].iter().all(|b| *b == 0)
            && unsafe {
                libc::fallocate(
                    file.as_raw_fd(),
                    libc::FALLOC_FL_PUNCH_HOLE | libc::FALLOC_FL_KEEP_SIZE,
                    offset as i64,
                    want as i64,
                )
            } < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        offset += want as u64;
    }
    Ok(())
}
