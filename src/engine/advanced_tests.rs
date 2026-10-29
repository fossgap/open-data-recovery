use super::{types::*, *};
use std::{
    collections::BTreeMap, fs, io::Write, os::unix::fs::MetadataExt, path::Path, process::Command,
};

fn source(fsname: &str) -> SourceDescriptor {
    SourceDescriptor {
        path: "/tmp/test-image".into(),
        name: "Test image".into(),
        kind: SourceType::Image,
        filesystem: fsname.into(),
        size: 16 * 1024 * 1024,
        sector_size: 512,
        mounted: false,
        parent_path: None,
        mountpoints: vec![],
        scope: None,
        fingerprint: Fingerprint::default(),
        ancestry: vec![],
        ewf: false,
        segments: vec![],
        partition: None,
    }
}
fn versions() -> BTreeMap<String, String> {
    [
        "fls",
        "icat",
        "extundelete",
        "ext4magic",
        "btrfs",
        "ntfsundelete",
        "photorec",
        "foremost",
        "ddrescue",
    ]
    .into_iter()
    .map(|s| (s.into(), "test".into()))
    .collect()
}
#[test]
fn advanced_options_are_backward_compatible_and_context_is_resolved() {
    let mut o: ScanOptions = serde_json::from_str(r#"{"mode":"Full","low_memory":true}"#).unwrap();
    assert!(o.low_memory);
    assert!(!ScanOptions::default().low_memory);
    let p = plan::resolve(&source("ext4"), &o, &versions()).unwrap();
    assert_eq!(p.metadata, vec!["tsk", "ext4magic", "extundelete"]);
    assert_eq!(p.carver.as_deref(), Some("photorec"));
    o.metadata_sequence = vec!["extundelete".into(), "tsk".into()];
    assert_eq!(
        plan::resolve(&source("ext4"), &o, &versions())
            .unwrap()
            .metadata,
        o.metadata_sequence
    );
    o.mode = Mode::Deep;
    assert!(
        plan::resolve(&source("ext4"), &o, &versions())
            .unwrap()
            .metadata
            .is_empty()
    );
    o.photorec.free_space = true;
    assert!(plan::resolve(&source("unknown"), &o, &versions()).is_err());
    o.interpretation.filesystem = "ntfs".into();
    assert!(plan::resolve(&source("unknown"), &o, &versions()).is_ok());
}
#[test]
fn retired_metadata_engine_is_rejected_in_saved_scan_options() {
    let mut installed = versions();
    installed.insert("ext4rescue".into(), "legacy installation".into());
    for saved in [
        r#"{"metadata_backend":"ext4rescue"}"#,
        r#"{"metadata_sequence":["tsk","ext4rescue"]}"#,
    ] {
        let mut options: ScanOptions = serde_json::from_str(saved).unwrap();
        for mode in [Mode::Quick, Mode::Full] {
            options.mode = mode;
            assert!(plan::resolve(&source("ext4"), &options, &installed).is_err());
        }
    }
}
#[test]
fn families_filter_extraction_without_silently_dropping_unknown_extensions() {
    let mut o = ScanOptions {
        extensions: vec!["docx".into(), "jpg".into()],
        ..Default::default()
    };
    o.photorec.brute_force = true;
    o.photorec.free_space = true;
    let args = options::photorec_command(&o, "ext4", 4096);
    assert!(args.contains("geometry,N,4096"));
    assert!(args.contains("mode_ext2"));
    assert!(args.contains("paranoid_bf"));
    assert!(!args.contains("lowmem"));
    assert!(args.contains("everything,disable,jpg,enable,zip,enable"));
    assert!(args.ends_with("freespace,search"));
    o.extensions.push("unknownformat".into());
    assert!(options::photorec_command(&o, "ext4", 512).contains("everything,enable"));
    assert_eq!(
        options::foremost_types(&["docx".into(), "pdf".into()]).as_deref(),
        Some("pdf,zip")
    );
}
#[test]
fn recovery_targets_are_validated_and_native_ntfs_rows_preserve_spaces() {
    let mut o = ScanOptions::default();
    o.ext.target_kind = "directory".into();
    o.ext.target = "../../escape".into();
    assert!(o.validate().is_err());
    o.ext.target = "home/alice".into();
    o.ext.after = Some(20);
    o.ext.before = Some(10);
    assert!(o.validate().is_err());
    assert!(options::inode_list("42,70-80", true));
    assert!(!options::inode_list("80-70", true));
    assert!(!options::inode_list("42 --force", true));
    assert_eq!(
        advanced::ntfs_record("42  FN..  100%  2024-01-01 12:30  123  Project notes.txt"),
        Some((42, "Project notes.txt".into()))
    );
    assert!(advanced::ntfs_record("Inode Flags %age Date Time Size Filename").is_none());
}
#[test]
fn maps_require_complete_coverage_and_distinguish_image_size_from_acquisition() {
    let (s, r) = imaging::parse_map(
        "# map\n0x10 + 1\n0x0 0x10 +\n0x10 0x10 ?\n0x20 0x10 -\n",
        48,
    )
    .unwrap();
    assert_eq!(s.rescued, 16);
    assert_eq!(s.pending, 16);
    assert_eq!(s.bad, 16);
    assert!(!s.complete);
    assert_eq!(r.len(), 3);
    for text in [
        "0 + 1\n0 20 +\n10 38 ?",
        "0 + 1\n0 16 +",
        "0 + 1\n0 49 +",
        "0 + 1\n0 0 +",
        "0 + 1\n0 48 Z",
    ] {
        assert!(imaging::parse_map(text, 48).is_err());
    }
    let mut o = ScanOptions::default();
    o.ddrescue.direct = true;
    o.ddrescue.reverse = true;
    o.ddrescue.scrape = true;
    o.retry_passes = 3;
    let args = imaging::arguments(&o, 4096);
    assert!(args.contains(&"4096".into()));
    assert!(args.contains(&"--idirect".into()));
    assert!(args.contains(&"--reverse".into()));
    assert!(!args.contains(&"--no-scrape".into()));
}
#[test]
fn recovered_links_never_follow_destination_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let guard = safety::GuardedDir::existing(root.path()).unwrap();
    let (mut file, path) = guard
        .create_file("original.txt", true, Conflict::Rename)
        .unwrap()
        .unwrap();
    file.write_all(b"evidence").unwrap();
    let linked = guard
        .create_link("copy.txt", true, Conflict::Rename, None, Some(&file))
        .unwrap()
        .unwrap();
    assert_eq!(
        fs::metadata(path).unwrap().ino(),
        fs::metadata(linked).unwrap().ino()
    );
    guard
        .create_link(
            "redirect",
            true,
            Conflict::Rename,
            Some(outside.path().to_str().unwrap()),
            None,
        )
        .unwrap();
    assert!(
        guard
            .create_link(
                "redirect/escape",
                true,
                Conflict::Rename,
                Some("value"),
                None
            )
            .is_err()
    );
    assert!(!outside.path().join("escape").exists());
}

#[test]
fn linux_attributes_and_sparse_content_survive_copy_and_reopen() {
    use std::os::fd::AsRawFd;
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("original");
    let mut file = fs::File::create(&original).unwrap();
    file.set_len(3 * 1024 * 1024).unwrap();
    file.write_all(b"header").unwrap();
    let key = std::ffi::CString::new("user.odr-test").unwrap();
    let value = b"original attribute";
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                key.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        },
        0
    );
    let captured = metadata::capture(&original).unwrap();
    let dest = root.path().join("copy");
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&dest)
        .unwrap();
    metadata::copy(
        &mut fs::File::open(&original).unwrap(),
        &mut output,
        true,
        &process::Control::default(),
    )
    .unwrap();
    assert!(metadata::apply(&output, &captured).is_empty());
    assert_eq!(metadata::capture(&dest).unwrap().xattrs, captured.xattrs);
    assert_eq!(fs::read(&original).unwrap(), fs::read(&dest).unwrap());
    assert!(output.metadata().unwrap().blocks() * 512 < output.metadata().unwrap().len());
}

#[test]
fn restrictive_artifact_permissions_keep_the_original_acl() {
    use std::os::fd::AsRawFd;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("restricted");
    let file = fs::File::create(&path).unwrap();
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, permission, id) in [
        (1u16, 0u16, u32::MAX),
        (2, 4, unsafe { libc::getuid() }),
        (4, 0, u32::MAX),
        (16, 4, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        acl.extend(tag.to_le_bytes());
        acl.extend(permission.to_le_bytes());
        acl.extend(id.to_le_bytes());
    }
    let key = std::ffi::CString::new("system.posix_acl_access").unwrap();
    assert_eq!(
        unsafe {
            libc::fsetxattr(
                file.as_raw_fd(),
                key.as_ptr(),
                acl.as_ptr().cast(),
                acl.len(),
                0,
            )
        },
        0,
        "{}",
        std::io::Error::last_os_error()
    );
    let metadata = metadata::capture_artifact(&path).unwrap();
    assert_eq!(metadata.mode, Some(0o040));
    assert_eq!(metadata.xattrs["system.posix_acl_access"], acl);
    metadata::prepare_artifact(&path, false).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
}

#[test]
#[ignore = "Runs native Btrfs restore on a disposable filesystem image"]
fn disposable_btrfs_target_directory_and_metadata() {
    let root = tempfile::tempdir().unwrap();
    let files = root.path().join("input");
    fs::create_dir_all(files.join("chosen")).unwrap();
    fs::write(files.join("chosen/report.txt"), b"Btrfs selected file").unwrap();
    fs::write(files.join("outside.txt"), b"not selected").unwrap();
    let image = root.path().join("btrfs.img");
    fs::File::create(&image)
        .unwrap()
        .set_len(128 * 1024 * 1024)
        .unwrap();
    assert!(
        Command::new("mkfs.btrfs")
            .args(["-q", "-f", "--rootdir"])
            .arg(&files)
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    let mut o = ScanOptions {
        workspace: root.path().join("work"),
        mode: Mode::Quick,
        metadata_backend: "btrfs".into(),
        capture_metadata: true,
        ..Default::default()
    };
    o.btrfs.path = "chosen".into();
    let src = sources::inspect(&image).unwrap();
    let roots = advanced::inspect_btrfs(&src, &o, true).unwrap();
    assert!(roots.contains("tree key"), "{roots}");
    let preview = advanced::inspect_btrfs(&src, &o, false).unwrap();
    assert!(preview.contains("report.txt"), "{preview}");
    assert!(!preview.contains("outside.txt"), "{preview}");
    let session = jobs::scan(src, o, &process::Control::default(), &|e| {
        if let Event::Message(m) = e {
            eprintln!("{m}");
        }
    })
    .unwrap();
    let store = store::Store::open(&session).unwrap();
    assert!(store.manifest.complete, "{:?}", store.manifest.errors);
    let page = store
        .page(&Query {
            limit: 250,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.files.len(), 1, "{:?}", page.files);
    assert!(page.files[0].path.ends_with("report.txt"));
    assert!(page.files[0].linux_metadata.is_some());
}

#[test]
#[ignore = "Creates and undeletes two resident NTFS records on a disposable image"]
fn disposable_ntfs_filters_records_before_extraction() {
    use std::os::unix::fs::FileExt;
    let root = tempfile::tempdir().unwrap();
    let image = root.path().join("ntfs.img");
    fs::File::create(&image)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    assert!(
        Command::new("mkfs.ntfs")
            .args(["-F", "-Q"])
            .arg(&image)
            .output()
            .unwrap()
            .status
            .success()
    );
    let payload = root.path().join("payload");
    fs::write(&payload, b"Resident NTFS test content").unwrap();
    for name in ["wanted.txt", "excluded.bin"] {
        assert!(
            Command::new("ntfscp")
                .arg(&image)
                .arg(&payload)
                .arg(format!("/{name}"))
                .status()
                .unwrap()
                .success()
        );
    }
    let listing = Command::new("ntfsls")
        .arg("-i")
        .arg(&image)
        .output()
        .unwrap();
    let listing = String::from_utf8(listing.stdout).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&image)
        .unwrap();
    let mut boot = [0u8; 512];
    file.read_exact_at(&mut boot, 0).unwrap();
    let sector = u16::from_le_bytes(boot[11..13].try_into().unwrap()) as u64;
    let cluster = sector * boot[13] as u64;
    let mft = u64::from_le_bytes(boot[48..56].try_into().unwrap()) * cluster;
    let unit = boot[64] as i8;
    let record = if unit < 0 {
        1u64 << (-unit as u32)
    } else {
        unit as u64 * cluster
    };
    let mut mft_record = vec![0; record as usize];
    file.read_exact_at(&mut mft_record, mft).unwrap();
    let mut attr = u16::from_le_bytes(mft_record[20..22].try_into().unwrap()) as usize;
    let mut bitmap = 0;
    while attr + 64 < mft_record.len() {
        let kind = u32::from_le_bytes(mft_record[attr..attr + 4].try_into().unwrap());
        if kind == 0xffffffff {
            break;
        }
        let len = u32::from_le_bytes(mft_record[attr + 4..attr + 8].try_into().unwrap()) as usize;
        if kind == 0xb0 {
            let run = attr
                + u16::from_le_bytes(mft_record[attr + 32..attr + 34].try_into().unwrap()) as usize;
            let n = (mft_record[run] & 15) as usize;
            let offset = (mft_record[run] >> 4) as usize;
            let mut data = [0; 8];
            data[..offset].copy_from_slice(&mft_record[run + 1 + n..run + 1 + n + offset]);
            bitmap = u64::from_le_bytes(data) * cluster;
            break;
        }
        attr += len;
    }
    assert!(bitmap > 0);
    for line in listing.lines() {
        if line.contains("wanted.txt") || line.contains("excluded.bin") {
            let inode: u64 = line.split_whitespace().next().unwrap().parse().unwrap();
            let offset = mft + inode * record + 22;
            let mut flags = [0; 2];
            file.read_exact_at(&mut flags, offset).unwrap();
            flags[0] &= !1;
            file.write_all_at(&flags, offset).unwrap();
            let mut bit = [0];
            file.read_exact_at(&mut bit, bitmap + inode / 8).unwrap();
            bit[0] &= !(1 << (inode % 8));
            file.write_all_at(&bit, bitmap + inode / 8).unwrap();
        }
    }
    file.sync_all().unwrap();
    let o = ScanOptions {
        workspace: root.path().join("work"),
        mode: Mode::Quick,
        metadata_backend: "ntfsundelete".into(),
        extensions: vec!["txt".into()],
        ..Default::default()
    };
    let session = jobs::scan(
        sources::inspect(&image).unwrap(),
        o,
        &process::Control::default(),
        &|e| {
            if let Event::Message(m) = e {
                eprintln!("{m}");
            }
        },
    )
    .unwrap();
    let store = store::Store::open(&session).unwrap();
    assert!(store.manifest.complete, "{:?}", store.manifest.errors);
    let files = store
        .page(&Query {
            limit: 250,
            ..Default::default()
        })
        .unwrap()
        .files;
    assert_eq!(files.len(), 1, "{:?}", files);
    assert!(files[0].path.ends_with("wanted.txt"));
    let artifacts = session.join("artifacts/ntfsundelete-0/files");
    assert_eq!(fs::read_dir(artifacts).unwrap().count(), 1);
    assert_eq!(
        fs::read(session.join(files[0].artifact.as_ref().unwrap())).unwrap(),
        b"Resident NTFS test content"
    );
}
fn ext_image(root: &Path) -> std::path::PathBuf {
    let files = root.join("files");
    fs::create_dir(&files).unwrap();
    fs::write(files.join("notes.txt"), b"recovery content\n").unwrap();
    fs::hard_link(files.join("notes.txt"), files.join("alias.txt")).unwrap();
    std::os::unix::fs::symlink("notes.txt", files.join("shortcut")).unwrap();
    let image = root.join("source.img");
    fs::File::create(&image)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    assert!(
        Command::new("mkfs.ext2")
            .args(["-q", "-F", "-d"])
            .arg(files)
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    image
}
#[test]
#[ignore = "Runs real engines on a disposable ext image"]
fn disposable_advanced_tsk_metadata_and_interpretation_recovery() {
    let root = tempfile::tempdir().unwrap();
    let image = ext_image(root.path());
    let o = ScanOptions {
        workspace: root.path().join("work"),
        mode: Mode::Quick,
        metadata_backend: "tsk".into(),
        include_existing: true,
        capture_metadata: true,
        ..Default::default()
    };
    let session = jobs::scan(
        sources::inspect(&image).unwrap(),
        o,
        &process::Control::default(),
        &|event| {
            if let Event::Message(message) = event {
                eprintln!("{message}");
            }
        },
    )
    .unwrap();
    let s = store::Store::open(&session).unwrap();
    assert!(s.manifest.complete, "{:?}", s.manifest.errors);
    let files = s
        .page(&Query {
            limit: 250,
            ..Default::default()
        })
        .unwrap()
        .files;
    assert!(
        files
            .iter()
            .any(|c| c.linux_metadata.as_ref().is_some_and(|m| m.symbolic))
    );
    s.select(&files.iter().map(|c| c.id).collect::<Vec<_>>(), true)
        .unwrap();
    let destination = root.path().join("recovered");
    jobs::recover(
        &session,
        &RecoveryOptions {
            destination: destination.clone(),
            preserve_linux_metadata: true,
            ..Default::default()
        },
        &process::Control::default(),
        &|event| {
            if let Event::Message(message) = event {
                eprintln!("{message}");
            }
        },
    )
    .unwrap();
    assert_eq!(
        fs::read(destination.join("notes.txt")).unwrap(),
        b"recovery content\n"
    );
    assert_eq!(
        fs::read_link(destination.join("shortcut")).unwrap(),
        Path::new("notes.txt")
    );
    assert_eq!(
        fs::metadata(destination.join("notes.txt")).unwrap().ino(),
        fs::metadata(destination.join("alias.txt")).unwrap().ino()
    );
}
#[test]
#[ignore = "Interrupts and resumes real ddrescue on a disposable image"]
fn disposable_ddrescue_restart_resumes_the_saved_map() {
    let root = tempfile::tempdir().unwrap();
    let image = ext_image(root.path());
    let mut o = ScanOptions {
        workspace: root.path().join("work"),
        mode: Mode::Quick,
        metadata_backend: "tsk".into(),
        clone_first: true,
        ..Default::default()
    };
    o.ddrescue.max_read_rate = 1024 * 1024;
    let control = process::Control::default();
    let cancel = control.clone();
    let (started, receiver) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        receiver
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1200));
        cancel.cancel();
    });
    let session = jobs::scan(sources::inspect(&image).unwrap(), o, &control, &|event| {
        if matches!(event, Event::Stage { ref text, .. } if text == "Imaging source with ddrescue")
        {
            started.send(()).unwrap();
        }
    })
    .unwrap();
    thread.join().unwrap();
    let before = store::Store::open(&session).unwrap().manifest;
    assert!(imaging::can_resume(&before), "{:?}", before);
    let original_map = fs::read(session.join("clone/clone.map")).unwrap();
    let original_image = fs::read(session.join("clone/clone.img")).unwrap();
    let mut import = before.options.clone();
    import.workspace = root.path().join("imported");
    import.ddrescue.max_read_rate = 0;
    import.ddrescue.import_image = Some(session.join("clone/clone.img"));
    import.ddrescue.import_map = Some(session.join("clone/clone.map"));
    let imported = jobs::scan(
        sources::inspect(&image).unwrap(),
        import,
        &process::Control::default(),
        &|_| {},
    )
    .unwrap();
    let imported_manifest = store::Store::open(&imported).unwrap().manifest;
    assert!(imported_manifest.complete, "{:?}", imported_manifest.errors);
    assert!(imported_manifest.imaging.unwrap().complete);
    assert_eq!(
        fs::read(session.join("clone/clone.img")).unwrap(),
        original_image
    );
    assert_eq!(
        fs::read(session.join("clone/clone.map")).unwrap(),
        original_map
    );
    assert_eq!(
        fs::read(imported.join("clone/clone.img")).unwrap(),
        fs::read(&image).unwrap()
    );
    let finished = std::cell::Cell::new(false);
    imaging::resume(
        &session,
        DdrescueOptions::default(),
        0,
        &process::Control::default(),
        &|event| {
            if let Event::Imaged { progress, .. } = event {
                finished.set(progress.complete);
            }
        },
    )
    .unwrap();
    assert!(
        finished.get(),
        "Imaging must publish its own terminal event, not a scan-complete event."
    );
    let after = store::Store::open(&session).unwrap().manifest;
    assert!(after.imaging.unwrap().complete);
    assert_eq!(after.state, "imaged");
    assert_eq!(
        jobs::hash_file(
            &mut fs::File::open(&image).unwrap(),
            &process::Control::default()
        )
        .unwrap(),
        jobs::hash_file(
            &mut fs::File::open(session.join("clone/clone.img")).unwrap(),
            &process::Control::default()
        )
        .unwrap()
    );
}
