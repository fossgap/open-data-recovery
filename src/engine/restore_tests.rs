use super::*;
use std::io::{Cursor, Write};

pub(crate) fn pair() -> (Image, Disk) {
    let file = tempfile::NamedTempFile::new().unwrap();
    let identity = Identity::from(file.as_file().metadata().unwrap());
    let image = Image {
        path: "/tmp/fixture.img".into(),
        size: 4096,
        identity: identity.clone(),
        ancestry: vec!["block:8:0".into()],
    };
    let disk = Disk {
        path: "/dev/fixture".into(),
        model: "Disposable fixture".into(),
        serial: "fixture-only".into(),
        size: 8192,
        sector_size: 512,
        unavailable: None,
        wwn: "fixture".into(),
        device: "8:16".into(),
        sys_path: "/sys/fixture".into(),
        sequence: 1,
        identity,
        ancestry: vec!["block:8:16".into()],
    };
    (image, disk)
}

#[test]
fn restoration_rejects_replaced_disks_and_regular_file_targets() {
    let (_, disk) = pair();
    validate_disk_identity(&disk, &disk).unwrap();
    let mut replacement = disk.clone();
    replacement.serial = "another disk".into();
    assert!(validate_disk_identity(&disk, &replacement).is_err());
    replacement = disk.clone();
    replacement.sequence += 1;
    assert!(validate_disk_identity(&disk, &replacement).is_err());
    replacement = disk.clone();
    replacement.identity.inode += 1;
    assert!(validate_disk_identity(&disk, &replacement).is_err());
    replacement = disk.clone();
    replacement.size += 512;
    assert!(validate_disk_identity(&disk, &replacement).is_err());
    replacement = disk.clone();
    replacement.unavailable = Some("mounted".into());
    assert!(validate_disk_identity(&disk, &replacement).is_err());
    let file = tempfile::tempfile().unwrap();
    assert!(validate_descriptor(&file, &disk).is_err());
}

#[test]
fn restoration_rejects_overlap_small_disks_unknown_storage_and_sector_misalignment() {
    let (mut image, mut disk) = pair();
    validate_pair(&image, &disk).unwrap();
    disk.ancestry = image.ancestry.clone();
    assert!(
        validate_pair(&image, &disk)
            .unwrap_err()
            .to_string()
            .contains("stored on")
    );
    disk.ancestry.clear();
    assert!(validate_pair(&image, &disk).is_err());
    disk.ancestry = vec!["block:8:16".into()];
    disk.size = image.size - 1;
    assert!(
        validate_pair(&image, &disk)
            .unwrap_err()
            .to_string()
            .contains("smaller")
    );
    disk.size = image.size;
    disk.sector_size = 4096;
    validate_pair(&image, &disk).unwrap();
    image.size -= 512;
    assert!(validate_pair(&image, &disk).is_err());
    disk.sector_size = 0;
    assert!(validate_pair(&image, &disk).is_err());
    disk.sector_size = 512;
    disk.unavailable = Some("Mounted".into());
    assert!(validate_pair(&image, &disk).is_err());
}

#[test]
fn restoration_rejects_mounted_swap_readonly_mapped_and_unknown_targets() {
    let free = serde_json::json!({"type":"disk", "ro":false, "mountpoints":[null],
        "children":[{"type":"part", "ro":false, "mountpoints":[null]}]});
    assert!(usage_problem(&free).is_none());
    for mount in ["/", "/boot", "/home", "[SWAP]"] {
        let mut used = free.clone();
        used["children"][0]["mountpoints"] = serde_json::json!([mount]);
        assert!(usage_problem(&used).is_some());
    }
    for kind in ["lvm", "crypt", "raid1", "mpath"] {
        let mut used = free.clone();
        used["children"][0]["type"] = kind.into();
        assert!(usage_problem(&used).is_some());
    }
    for bad in [
        serde_json::json!(true),
        Value::Null,
        serde_json::json!("unknown"),
    ] {
        let mut used = free.clone();
        used["ro"] = bad;
        assert!(usage_problem(&used).is_some());
    }
    let mut unknown = free;
    unknown["mountpoints"] = Value::Null;
    assert!(usage_problem(&unknown).is_some());
}

#[test]
fn restoration_requires_exact_confirmation_before_any_target_access() {
    let (image, disk) = pair();
    validate_confirmation(&disk.path, "/dev/fixture").unwrap();
    for input in [
        "",
        "yes",
        "/dev/fixture1",
        "/dev/fixture ",
        " /dev/fixture",
        "/dev/disk/by-id/fixture",
    ] {
        let error = restore(
            Plan {
                image: image.clone(),
                disk: disk.clone(),
            },
            input,
            &Control::default(),
            |_, _, _| {},
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("exact target"));
    }
    let control = Control::default();
    control.cancel();
    assert!(matches!(
        restore(Plan { image, disk }, "/dev/fixture", &control, |_, _, _| {}),
        Err(Error::Cancelled)
    ));
}

#[test]
fn raw_image_validation_rejects_containers_nonregular_files_and_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let raw = root.path().join("disk.img");
    fs::write(&raw, vec![0u8; 4096]).unwrap();
    assert!(open_image(&raw).is_ok());
    let link = root.path().join("alias.img");
    std::os::unix::fs::symlink(&raw, &link).unwrap();
    assert!(open_image(&link).is_err());
    assert!(inspect_image(&link).is_err());
    assert!(open_image(root.path()).is_err());
    for name in [
        "disk.E01",
        "disk.EX01",
        "disk.gz",
        "disk.qcow2",
        "disk.vmdk",
        "disk.vhdx",
    ] {
        let path = root.path().join(name);
        fs::write(&path, vec![0u8; 4096]).unwrap();
        assert!(open_image(&path).is_err(), "{name}");
    }
    for magic in [
        b"EVF".as_slice(),
        b"QFI\xfb",
        b"\x1f\x8b",
        b"vhdxfile",
        b"PK\x03\x04",
    ] {
        let mut contents = vec![0u8; 4096];
        contents[..magic.len()].copy_from_slice(magic);
        fs::write(&raw, contents).unwrap();
        assert!(open_image(&raw).is_err());
    }
    // Container signatures also matter when a file has been renamed to .img.
    for (offset, magic) in [
        (64, [0x7f, 0x10, 0xda, 0xbe].as_slice()),
        (3584, b"conectix".as_slice()),
    ] {
        let mut contents = vec![0; 4096];
        contents[offset..offset + magic.len()].copy_from_slice(magic);
        fs::write(&raw, contents).unwrap();
        assert!(open_image(&raw).is_err());
    }
    fs::write(&raw, [0; 511]).unwrap();
    assert!(open_image(&raw).is_err());
}

#[test]
fn image_identity_detects_replacement_size_and_in_place_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("image.raw");
    fs::write(&path, vec![1u8; 4096]).unwrap();
    let file = open_image(&path).unwrap();
    let identity = Identity::from(file.metadata().unwrap());
    validate_image(&file, &identity).unwrap();
    let replacement = root.path().join("replacement.raw");
    fs::write(&replacement, vec![2u8; 4096]).unwrap();
    fs::rename(&replacement, &path).unwrap();
    assert!(validate_image(&open_image(&path).unwrap(), &identity).is_err());
    fs::write(&path, vec![3u8; 8192]).unwrap();
    assert!(validate_image(&open_image(&path).unwrap(), &identity).is_err());
    let new_file = open_image(&path).unwrap();
    let new_identity = Identity::from(new_file.metadata().unwrap());
    use std::os::unix::fs::FileExt;
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .write_all_at(b"changed", 2000)
        .unwrap();
    assert!(validate_image(&new_file, &new_identity).is_err());
}

#[test]
#[ignore = "requires GNU dd; uses only disposable regular files"]
fn disposable_restoration_dd_copies_exact_bytes_verifies_and_keeps_source_and_target_tail() {
    dd::check_available().unwrap();
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source with spaces;$(false).img");
    let contents: Vec<u8> = (0..3 * 1024 * 1024 + 512)
        .map(|i| (i % 251) as u8)
        .collect();
    fs::write(&path, &contents).unwrap();
    let mut source = open_image(&path).unwrap();
    let mut disk = tempfile::tempfile().unwrap();
    disk.write_all(&vec![0x55; contents.len() + 4096]).unwrap();
    let mut positions = Vec::new();
    dd::copy(
        &mut source,
        &mut disk,
        contents.len() as u64,
        &Control::default(),
        |n| positions.push(n),
    )
    .unwrap();
    let hash = verify_image(
        &mut source,
        &mut disk,
        contents.len() as u64,
        &Control::default(),
        |_| {},
    )
    .unwrap();
    assert_eq!(hash, format!("{:x}", Sha256::digest(&contents)));
    assert_eq!(positions.first(), Some(&0));
    assert_eq!(positions.last(), Some(&(contents.len() as u64)));
    assert!(positions.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(fs::read(&path).unwrap(), contents);
    disk.rewind().unwrap();
    let mut actual = Vec::new();
    disk.read_to_end(&mut actual).unwrap();
    assert_eq!(actual[..contents.len()], contents);
    assert_eq!(actual[contents.len()..], vec![0x55; 4096]);
}

#[test]
fn restoration_cancellation_and_short_source_stop_before_starting_dd() {
    let mut source = tempfile::tempfile().unwrap();
    source.write_all(&[1; 512]).unwrap();
    let mut target = tempfile::tempfile().unwrap();
    target.write_all(&[0x55; 4096]).unwrap();
    let control = Control::default();
    control.cancel();
    assert!(matches!(
        dd::copy(&mut source, &mut target, 512, &control, |_| {}),
        Err(Error::Cancelled)
    ));
    assert!(
        dd::copy(&mut source, &mut target, 4096, &Control::default(), |_| {})
            .unwrap_err()
            .to_string()
            .contains("length changed")
    );
    target.rewind().unwrap();
    let mut actual = Vec::new();
    target.read_to_end(&mut actual).unwrap();
    assert_eq!(actual, vec![0x55; 4096]);
}

#[test]
fn restoration_verification_rejects_corruption_short_reads_and_cancellation() {
    let contents = vec![42; 4096];
    let hash = verify_image(
        &mut Cursor::new(&contents),
        &mut Cursor::new(&contents),
        4096,
        &Control::default(),
        |_| {},
    )
    .unwrap();
    assert_eq!(hash, format!("{:x}", Sha256::digest(&contents)));
    let changed = vec![24; 4096];
    let short = vec![42; 32];
    for (source, target) in [
        (&contents, &changed),
        (&short, &contents),
        (&contents, &short),
    ] {
        assert!(
            verify_image(
                &mut Cursor::new(source),
                &mut Cursor::new(target),
                4096,
                &Control::default(),
                |_| {},
            )
            .is_err()
        );
    }
    let control = Control::default();
    control.cancel();
    assert!(matches!(
        verify_image(
            &mut Cursor::new(&contents),
            &mut Cursor::new(&contents),
            4096,
            &control,
            |_| {},
        ),
        Err(Error::Cancelled)
    ));
}

#[test]
#[ignore = "requires GNU dd; uses only disposable regular files"]
fn disposable_restoration_dd_rejects_early_eof_and_write_errors() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.img");
    fs::write(&path, vec![42; 4096]).unwrap();
    let mut source = open_image(&path).unwrap();
    let mut disk = tempfile::tempfile().unwrap();
    disk.write_all(&[0x55; 8192]).unwrap();
    let mut truncated = false;
    let error = dd::copy(&mut source, &mut disk, 4096, &Control::default(), |_| {
        if !truncated {
            // Simulate an external change after the size check but before dd.
            OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_len(512)
                .unwrap();
            truncated = true;
        }
    })
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("did not copy the complete image")
    );
    disk.rewind().unwrap();
    let mut actual = Vec::new();
    disk.read_to_end(&mut actual).unwrap();
    assert_eq!(&actual[..512], &[42; 512]);
    assert_eq!(&actual[512..], &[0x55; 7680]);

    fs::write(&path, vec![42; 4096]).unwrap();
    source = open_image(&path).unwrap();
    let target = root.path().join("readonly-target.img");
    fs::write(&target, [0x55; 8192]).unwrap();
    let mut readonly = File::open(&target).unwrap();
    let error = dd::copy(
        &mut source,
        &mut readonly,
        4096,
        &Control::default(),
        |_| {},
    )
    .unwrap_err();
    assert!(error.to_string().contains("GNU dd failed"), "{error}");
    assert_eq!(fs::read(&path).unwrap(), vec![42; 4096]);
    assert_eq!(fs::read(&target).unwrap(), vec![0x55; 8192]);
}

#[test]
#[ignore = "requires GNU dd; uses only disposable regular files"]
fn disposable_restoration_dd_limits_writes_if_the_source_grows() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.img");
    fs::write(&path, [42; 4096]).unwrap();
    let mut source = open_image(&path).unwrap();
    let identity = Identity::from(source.metadata().unwrap());
    let mut target = tempfile::tempfile().unwrap();
    target.write_all(&[0x55; 8192]).unwrap();
    let mut appended = false;
    dd::copy(&mut source, &mut target, 4096, &Control::default(), |_| {
        if !appended {
            OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap()
                .write_all(&[24; 4096])
                .unwrap();
            appended = true;
        }
    })
    .unwrap();
    assert!(validate_image(&source, &identity).is_err());
    target.rewind().unwrap();
    let mut actual = Vec::new();
    target.read_to_end(&mut actual).unwrap();
    assert_eq!(&actual[..4096], &[42; 4096]);
    assert_eq!(&actual[4096..], &[0x55; 4096]);
}

#[test]
#[ignore = "requires GNU dd; uses a socket input and disposable regular target"]
fn disposable_restoration_dd_cancellation_stops_and_reaps_a_blocked_writer() {
    use std::{
        os::{fd::OwnedFd, unix::net::UnixStream},
        process::Stdio,
        time::{Duration, Instant},
    };
    for ignore_interrupt in [false, true] {
        let (mut input, reader) = UnixStream::pair().unwrap();
        input.write_all(&[42; 512]).unwrap();
        // Keep the input open so real dd blocks after the first block. No timing or
        // large allocation is needed to catch an actively running copy.
        let mut target = tempfile::tempfile().unwrap();
        target.write_all(&[0x55; 8192]).unwrap();
        target.rewind().unwrap();
        let mut command = dd::command();
        command
            .args([
                "bs=512",
                "iflag=fullblock",
                "conv=notrunc,fsync",
                "status=progress",
            ])
            .stdin(Stdio::from(OwnedFd::from(reader)))
            .stdout(Stdio::from(target.try_clone().unwrap()));
        // Validate that the child-death guard was installed before exec as well.
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(move || {
                let mut signal: libc::c_int = 0;
                if libc::prctl(libc::PR_GET_PDEATHSIG, &mut signal) != 0 || signal != libc::SIGKILL
                {
                    return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
                }
                if ignore_interrupt {
                    let mut action: libc::sigaction = std::mem::zeroed();
                    action.sa_sigaction = libc::SIG_IGN;
                    libc::sigemptyset(&mut action.sa_mask);
                    if libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut()) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let control = Control::default();
        let began = Instant::now();
        let mut saw_partial_write = false;
        let error = dd::run(&mut command, &mut target, &control, |bytes| {
            if bytes == 512 {
                saw_partial_write = true;
                control.cancel();
            }
            if began.elapsed() > Duration::from_secs(5) {
                control.cancel();
            }
        })
        .unwrap_err();
        drop(command);
        assert!(matches!(error, Error::Cancelled));
        assert!(saw_partial_write);
        assert!(began.elapsed() < Duration::from_secs(5));
        if ignore_interrupt {
            assert!(began.elapsed() >= Duration::from_secs(2));
        }
        target.sync_all().unwrap();
        // The sole socket reader was closed when dd was reaped.
        assert!(input.write_all(&[0; 512]).is_err());
        target.rewind().unwrap();
        let mut actual = Vec::new();
        target.read_to_end(&mut actual).unwrap();
        assert_eq!(&actual[..512], &[42; 512]);
        assert_eq!(&actual[512..], &[0x55; 7680]);
    }
}

#[test]
#[ignore = "requires GNU dd and Landlock; uses only disposable regular files"]
fn disposable_restoration_dd_confinement_only_writes_through_the_preopened_target() {
    let root = tempfile::tempdir().unwrap();
    let image = root.path().join("image.raw");
    let target = root.path().join("target.raw");
    fs::write(&image, vec![42u8; 4096]).unwrap();
    fs::write(&target, vec![0u8; 8192]).unwrap();
    let image_path = image.clone();
    let target_path = target.clone();
    std::thread::spawn(move || {
        let mut source = open_image(&image_path).unwrap();
        let mut destination = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&target_path)
            .unwrap();
        process::confine_restore_thread().unwrap();
        assert!(OpenOptions::new().write(true).open(&image_path).is_err());
        assert!(OpenOptions::new().write(true).open(&target_path).is_err());
        assert!(fs::write(target_path.with_extension("other"), b"forbidden").is_err());
        dd::copy(
            &mut source,
            &mut destination,
            4096,
            &Control::default(),
            |_| {},
        )
        .unwrap();
        verify_image(
            &mut source,
            &mut destination,
            4096,
            &Control::default(),
            |_| {},
        )
        .unwrap();

        // An exec'd dd inherits Landlock and cannot reopen a writable path.
        let forbidden = dd::command()
            .arg("count=0")
            .arg(format!("of={}", target_path.display()))
            .stdin(std::process::Stdio::from(source.try_clone().unwrap()))
            .stdout(std::process::Stdio::from(destination.try_clone().unwrap()))
            .output()
            .unwrap();
        assert!(!forbidden.status.success());
    })
    .join()
    .unwrap();
    assert_eq!(fs::read(image).unwrap(), vec![42u8; 4096]);
    let actual = fs::read(target).unwrap();
    assert_eq!(&actual[..4096], vec![42u8; 4096]);
    assert_eq!(&actual[4096..], vec![0u8; 4096]);
}
