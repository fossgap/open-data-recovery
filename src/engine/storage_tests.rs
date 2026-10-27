use super::{
    jobs,
    process::Control,
    safety::{self, GuardedDir},
    sessions::Deletion,
    sources,
    store::Store,
    types::*,
};
use std::{
    fs::{self, File},
    io::Write,
    os::unix::fs::symlink,
    path::Path,
    process::Command,
};

fn image(root: &Path) -> SourceDescriptor {
    let path = root.join("source.img");
    fs::write(&path, vec![7; 8192]).unwrap();
    sources::inspect(&path).unwrap()
}

#[test]
fn images_can_share_storage_but_devices_folders_and_device_ranges_cannot() {
    let root = tempfile::tempdir().unwrap();
    let source = image(root.path());
    let workspace = root.path().join("scans");
    assert_eq!(source.ancestry, safety::path_ancestry(root.path()).unwrap());
    let store = Store::create(
        &source,
        &ScanOptions {
            workspace,
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap();
    assert!(store.directory.join("tree.sqlite").is_file());
    for (i, kind) in [SourceType::Block, SourceType::Folder, SourceType::Image]
        .into_iter()
        .enumerate()
    {
        let mut device = source.clone();
        device.kind = kind;
        if i > 0 {
            device.scope = Some("/folder".into());
        }
        device.partition = Some(DiscoveredPartition {
            offset_bytes: 0,
            length_bytes: 8192,
            sector_size: 512,
            filesystem: "ext2".into(),
            label: String::new(),
            description: String::new(),
            verified: true,
            sample_sha256: "a".repeat(64),
        });
        let rejected = root.path().join(format!("device-{i}"));
        assert!(
            Store::create(
                &device,
                &ScanOptions {
                    workspace: rejected.clone(),
                    clone_first: true,
                    ..Default::default()
                },
                Default::default()
            )
            .is_err()
        );
        assert!(!rejected.exists());
    }
}

#[test]
fn image_output_folders_cannot_contain_the_image_or_any_segment() {
    let root = tempfile::tempdir().unwrap();
    let mut source = image(root.path());
    let segment_dir = root.path().join("segments");
    fs::create_dir(&segment_dir).unwrap();
    let segment = segment_dir.join("source.E02");
    fs::write(&segment, b"evidence segment").unwrap();
    source.ewf = true;
    source.segments.push(segment.clone());
    for destination in [
        root.path().to_path_buf(),
        source.path.clone(),
        segment_dir,
        segment,
    ] {
        assert!(safety::validate_storage(&source, &destination, 0).is_err());
    }
    assert!(safety::validate_storage(&source, &root.path().join("output"), 0).is_ok());
    source.segments.push("../untrusted.E03".into());
    assert!(safety::validate_storage(&source, &root.path().join("output"), 0).is_err());
}

#[test]
fn replaced_or_nonregular_image_paths_do_not_qualify_for_shared_storage() {
    let root = tempfile::tempdir().unwrap();
    let mut source = image(root.path());
    let destination = root.path().join("output");
    let original = root.path().join("original.img");
    fs::rename(&source.path, &original).unwrap();
    symlink(&original, &source.path).unwrap();
    assert!(safety::validate_storage(&source, &destination, 0).is_err());
    fs::remove_file(&source.path).unwrap();
    fs::create_dir(&source.path).unwrap();
    assert!(safety::validate_storage(&source, &destination, 0).is_err());
    assert!(sources::open_readonly(&source).is_err());
    source.path = "/dev/null".into();
    source.segments.clear();
    assert!(safety::validate_storage(&source, &destination, 0).is_err());
    assert!(sources::open_readonly(&source).is_err());
    // A replaced parent cannot hide an input below an output directory.
    let link = root.path().join("alias");
    symlink(root.path(), &link).unwrap();
    source.path = link.join("original.img");
    assert!(safety::validate_storage(&source, &destination, 0).is_err());
    assert!(!destination.exists());
}

#[test]
fn recovery_filename_collisions_cannot_overwrite_image_hardlinks_or_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let source = image(root.path());
    let before = fs::read(&source.path).unwrap();
    let output = GuardedDir::create(&source, &root.path().join("restored"), 0).unwrap();
    fs::hard_link(&source.path, output.path.join("hardlink.img")).unwrap();
    symlink(&source.path, output.path.join("symlink.img")).unwrap();
    for name in ["hardlink.img", "symlink.img"] {
        assert!(
            output
                .create_file(name, true, Conflict::Skip)
                .unwrap()
                .is_none()
        );
        let (mut file, path) = output
            .create_file(name, true, Conflict::Rename)
            .unwrap()
            .unwrap();
        file.write_all(b"recovered bytes").unwrap();
        assert_ne!(path, output.path.join(name));
        assert_eq!(fs::read(&source.path).unwrap(), before);
    }
}

#[test]
fn disconnected_images_keep_the_exception_but_unknown_ancestry_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut source = image(root.path());
    fs::remove_file(&source.path).unwrap();
    let output = root.path().join("restored");
    assert!(safety::validate_storage(&source, &output, 0).is_ok());
    assert!(safety::validate_storage(&source, root.path(), 0).is_err());
    source.ancestry.clear();
    assert!(safety::validate_storage(&source, &output, 0).is_err());
    assert!(!output.exists());
}

#[test]
fn source_type_verification_cannot_be_overridden_by_recorded_metadata() {
    let root = tempfile::tempdir().unwrap();
    let source = image(root.path());
    for kind in [SourceType::Block, SourceType::Folder] {
        let mut changed = source.clone();
        changed.kind = kind;
        assert!(
            sources::verify(&changed)
                .unwrap_err()
                .to_string()
                .contains("source type changed")
        );
    }
    assert!(sources::verify(&source).is_ok());
}

#[test]
fn same_drive_snapshot_deletion_keeps_the_source_image_and_recovered_files() {
    let root = tempfile::tempdir().unwrap();
    let source = image(root.path());
    let before = fs::read(&source.path).unwrap();
    let store = Store::create(
        &source,
        &ScanOptions {
            workspace: root.path().join("scans"),
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap();
    let scan = SavedScan {
        directory: store.directory.clone(),
        manifest: store.manifest.clone(),
    };
    let recovered = root.path().join("restored.txt");
    fs::write(&recovered, b"recovered bytes").unwrap();
    drop(store);
    Deletion::prepare(scan.clone()).unwrap().execute().unwrap();
    assert!(!scan.directory.exists());
    assert_eq!(fs::read(&source.path).unwrap(), before);
    assert_eq!(fs::read(&recovered).unwrap(), b"recovered bytes");
}

fn restore(directory: &Path, destination: &Path, payload: &[u8]) {
    let store = Store::open(directory).unwrap();
    assert!(store.manifest.complete, "{:?}", store.manifest.errors);
    let files = store
        .page(&Query {
            limit: 250,
            ..Default::default()
        })
        .unwrap()
        .files;
    assert!(
        !files.is_empty(),
        "No findings for {:?} from {}",
        store.manifest.options.mode,
        store.manifest.source.path.display()
    );
    store
        .select(&files.iter().map(|f| f.id).collect::<Vec<_>>(), true)
        .unwrap();
    jobs::recover(
        directory,
        &RecoveryOptions {
            destination: destination.into(),
            preserve_tree: false,
            ..Default::default()
        },
        &Control::default(),
        &|_| {},
    )
    .unwrap();
    assert!(
        fs::read_dir(destination)
            .unwrap()
            .any(|f| fs::read(f.unwrap().path()).is_ok_and(|data| data == payload))
    );
    assert!(
        fs::read_to_string(directory.join("events.log"))
            .unwrap()
            .contains("\"verified\":true")
    );
}

#[test]
#[ignore = "Runs Quick/Deep/Full and segmented E01 recovery with all storage on the image's filesystem"]
fn disposable_same_drive_image_scan_modes_and_segmented_ewf() {
    let root = tempfile::tempdir().unwrap();
    let disk = root.path().join("source.img");
    File::create(&disk)
        .unwrap()
        .set_len(16 * 1024 * 1024)
        .unwrap();
    assert!(
        Command::new("mkfs.ext2")
            .args(["-q", "-F"])
            .arg(&disk)
            .status()
            .unwrap()
            .success()
    );
    let payload = include_bytes!("../../image.png");
    let original = root.path().join("original.png");
    fs::write(&original, payload).unwrap();
    for request in [
        format!("write {} recovered.png", original.display()),
        "rm recovered.png".into(),
    ] {
        assert!(
            Command::new("debugfs")
                .args(["-w", "-R", &request])
                .arg(&disk)
                .status()
                .unwrap()
                .success()
        );
    }
    // ext2 indirect blocks fragment this large PNG. Also place a contiguous
    // carving copy beyond the 16 MiB filesystem, without altering its records.
    // This fixture tests output placement, not fragmented PNG reconstruction.
    use std::os::unix::fs::FileExt;
    let writable = fs::OpenOptions::new().write(true).open(&disk).unwrap();
    writable.set_len(20 * 1024 * 1024).unwrap();
    writable.write_all_at(payload, 17 * 1024 * 1024).unwrap();
    drop(writable);
    let before = jobs::hash_file(&mut File::open(&disk).unwrap(), &Control::default()).unwrap();
    for mode in [Mode::Quick, Mode::Deep, Mode::Full] {
        let source = sources::inspect(&disk).unwrap();
        assert_eq!(source.ancestry, safety::path_ancestry(root.path()).unwrap());
        let dir = jobs::scan(
            source,
            ScanOptions {
                workspace: root.path().join("scans"),
                mode,
                extensions: vec!["png".into()],
                metadata_backend: "tsk".into(),
                carver: "photorec".into(),
                consolidate: mode == Mode::Full,
                timeout_seconds: 30,
                ..Default::default()
            },
            &Control::default(),
            &|event| eprintln!("{mode:?}: {event:?}"),
        )
        .unwrap();
        restore(
            &dir,
            &root.path().join(format!("restored-{mode:?}")),
            payload,
        );
        assert_eq!(
            jobs::hash_file(&mut File::open(&disk).unwrap(), &Control::default()).unwrap(),
            before
        );
    }
    let evidence = root.path().join("evidence");
    assert!(
        Command::new("ewfacquire")
            .args(["-u", "-q", "-c", "none", "-S", "1048576", "-t"])
            .arg(&evidence)
            .arg(&disk)
            .status()
            .unwrap()
            .success()
    );
    let source = sources::inspect(&evidence.with_extension("E01")).unwrap();
    assert!(
        source.segments.len() > 1,
        "fixture must contain multiple E01 segments"
    );
    let segments: Vec<_> = source
        .segments
        .iter()
        .map(|p| (p.clone(), fs::read(p).unwrap()))
        .collect();
    let dir = jobs::scan(
        source,
        ScanOptions {
            workspace: root.path().join("scans"),
            mode: Mode::Quick,
            extensions: vec!["png".into()],
            timeout_seconds: 30,
            ..Default::default()
        },
        &Control::default(),
        &|_| {},
    )
    .unwrap();
    for (segment, bytes) in segments {
        assert_eq!(fs::read(&segment).unwrap(), bytes);
        fs::rename(
            &segment,
            segment.with_file_name(format!(
                "{}.offline",
                segment.file_name().unwrap().to_string_lossy()
            )),
        )
        .unwrap();
    }
    restore(&dir, &root.path().join("restored-ewf"), payload);
    let job = jobs::restart(dir);
    loop {
        if let Event::Finished {
            error,
            cancelled,
            directory,
        } = job
            .events
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap()
        {
            assert!(!cancelled);
            assert!(error.is_none(), "{error:?}");
            assert!(Store::open(&directory).unwrap().manifest.complete);
            break;
        }
    }
    assert_eq!(
        jobs::hash_file(&mut File::open(&disk).unwrap(), &Control::default()).unwrap(),
        before
    );
}
