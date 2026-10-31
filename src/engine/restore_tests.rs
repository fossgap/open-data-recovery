use super::*;
use std::io::Cursor;

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
fn disposable_restoration_copies_exact_bytes_verifies_and_keeps_source_and_target_tail() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.img");
    let contents: Vec<u8> = (0..3 * 1024 * 1024 + 512)
        .map(|i| (i % 251) as u8)
        .collect();
    fs::write(&path, &contents).unwrap();
    let mut source = open_image(&path).unwrap();
    let mut disk = tempfile::tempfile().unwrap();
    disk.write_all(&vec![0x55; contents.len() + 4096]).unwrap();
    disk.rewind().unwrap();
    let mut positions = Vec::new();
    let hash = copy_image(
        &mut source,
        &mut disk,
        contents.len() as u64,
        &Control::default(),
        |n| positions.push(n),
    )
    .unwrap();
    disk.sync_all().unwrap();
    verify_image(
        &mut disk,
        contents.len() as u64,
        &hash,
        &Control::default(),
        |_| {},
    )
    .unwrap();
    assert_eq!(positions.first(), Some(&0));
    assert_eq!(positions.last(), Some(&(contents.len() as u64)));
    assert_eq!(fs::read(&path).unwrap(), contents);
    disk.rewind().unwrap();
    let mut actual = Vec::new();
    disk.read_to_end(&mut actual).unwrap();
    assert_eq!(actual[..contents.len()], contents);
    assert_eq!(actual[contents.len()..], vec![0x55; 4096]);
    disk.rewind().unwrap();
    disk.write_all(b"corrupt").unwrap();
    assert!(
        verify_image(
            &mut disk,
            contents.len() as u64,
            &hash,
            &Control::default(),
            |_| {}
        )
        .is_err()
    );
}

#[test]
fn cancellation_stops_before_writes_and_between_chunks_without_success() {
    let bytes = vec![1u8; 3 * 1024 * 1024];
    let control = Control::default();
    control.cancel();
    let mut output = Vec::new();
    assert!(matches!(
        copy_image(
            &mut bytes.as_slice(),
            &mut output,
            bytes.len() as u64,
            &control,
            |_| {}
        ),
        Err(Error::Cancelled)
    ));
    assert!(output.is_empty());
    let control = Control::default();
    assert!(matches!(
        copy_image(
            &mut bytes.as_slice(),
            &mut output,
            bytes.len() as u64,
            &control,
            |n| {
                if n > 0 {
                    control.cancel();
                }
            }
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(output.len(), 1024 * 1024);
    assert!(matches!(
        verify_image(&mut Cursor::new(output), 1024 * 1024, "", &control, |_| {}),
        Err(Error::Cancelled)
    ));
}

#[test]
fn short_source_and_write_errors_are_reported_without_false_completion() {
    let mut target = Vec::new();
    assert!(
        copy_image(
            &mut [1u8; 512].as_slice(),
            &mut target,
            4096,
            &Control::default(),
            |_| {}
        )
        .is_err()
    );
    assert!(target.is_empty());
    struct Failing;
    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disconnected"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let error = copy_image(
        &mut [1u8; 512].as_slice(),
        &mut Failing,
        512,
        &Control::default(),
        |_| {},
    )
    .unwrap_err();
    assert!(error.to_string().contains("disconnected"));
    assert!(
        verify_image(
            &mut Cursor::new([1u8; 32]),
            512,
            "",
            &Control::default(),
            |_| {}
        )
        .is_err()
    );
}

#[test]
fn restoration_confinement_only_writes_through_the_preopened_target() {
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
        let hash = copy_image(
            &mut source,
            &mut destination,
            4096,
            &Control::default(),
            |_| {},
        )
        .unwrap();
        destination.sync_all().unwrap();
        verify_image(&mut destination, 4096, &hash, &Control::default(), |_| {}).unwrap();
    })
    .join()
    .unwrap();
    assert_eq!(fs::read(image).unwrap(), vec![42u8; 4096]);
    assert_eq!(&fs::read(target).unwrap()[..4096], vec![42u8; 4096]);
}
