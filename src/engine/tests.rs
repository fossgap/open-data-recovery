use super::{
    adapters, jobs,
    process::{self, Control, Spec},
    safety, sources,
    store::{self, Store},
    types::*,
};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

#[test]
fn tsk_parser_preserves_compound_addresses_delimiters_and_partition_range() {
    let candidate = adapters::parse_body(
        "0|/Photos/a|b.jpg (deleted)|42-128-3|r/rrw-r--r--|0|0|4096|0|1700000000|0|0",
        1048576,
        67108864,
        "4.12",
    )
    .unwrap()
    .unwrap();
    assert_eq!(candidate.locator, "42-128-3");
    assert_eq!(candidate.path, "Photos/a|b.jpg");
    assert_eq!(candidate.bytes, 4096);
    assert_eq!(candidate.offset_bytes, 1048576);
    assert_eq!(candidate.length_bytes, 67108864);
    assert!(candidate.deleted);
    assert!(
        adapters::parse_body("0|/folder|12|d/drwxr-xr-x|0|0|4096|0|0|0|0", 0, 100, "test")
            .unwrap()
            .is_none()
    );
    assert!(adapters::parse_body("truncated", 0, 100, "test").is_err());
}

#[test]
fn extensions_are_data_and_ranges_are_validated() {
    assert_eq!(
        normalize_extensions("*.JPG, pdf .jpg ; PNG").unwrap(),
        vec!["jpg", "pdf", "png"]
    );
    for unsafe_value in ["jpg;$(id)", "../jpg", "png/../../x", "--output=x"] {
        assert!(normalize_extensions(unsafe_value).is_err());
    }
    let mut options = ScanOptions {
        length_bytes: Some(0),
        ..Default::default()
    };
    assert!(options.validate().is_err());
    options.length_bytes = None;
    options.minimum_size = 100;
    options.maximum_size = Some(20);
    assert!(options.validate().is_err());
}

#[test]
fn recovery_paths_conflicts_and_symlinks_are_confined() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    assert!(safety::safe_relative("../../outside", true).is_err());
    assert_eq!(
        safety::safe_relative("/folder/test.txt", false).unwrap(),
        Path::new("test.txt")
    );
    let guard = safety::GuardedDir::existing(dir.path()).unwrap();
    let (_, first) = guard
        .create_file("folder/test.txt", true, Conflict::Rename)
        .unwrap()
        .unwrap();
    let (_, second) = guard
        .create_file("folder/test.txt", true, Conflict::Rename)
        .unwrap()
        .unwrap();
    assert_ne!(first, second);
    assert!(
        guard
            .create_file("folder/test.txt", true, Conflict::Skip)
            .unwrap()
            .is_none()
    );
    symlink(outside.path(), dir.path().join("escape")).unwrap();
    assert!(
        guard
            .create_file("escape/file", true, Conflict::Rename)
            .is_err()
    );
    assert!(!outside.path().join("file").exists());
}

#[test]
fn device_sources_still_reject_same_storage_before_creating_the_workspace() {
    let root = tempfile::tempdir().unwrap();
    let image = root.path().join("source.img");
    fs::write(&image, vec![0; 8192]).unwrap();
    let destination = root.path().join("must-not-exist/nested");
    let mut source = match sources::inspect(&image) {
        Ok(source) => source,
        Err(error) => {
            assert!(error.to_string().contains("ancestry"));
            assert!(!destination.exists());
            return;
        }
    };
    // Model device ancestry without touching a physical device. Image files
    // have a deliberate exception, while the same ancestry for devices does not.
    source.kind = SourceType::Block;
    assert!(safety::validate_storage(&source, &destination, 0).is_err());
    assert!(
        Store::create(
            &source,
            &ScanOptions {
                workspace: destination.clone(),
                ..Default::default()
            },
            BTreeMap::new()
        )
        .is_err()
    );
    assert!(!destination.exists());
}

#[test]
fn backend_sandbox_denies_source_and_external_writes() {
    let source_dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("source");
    fs::write(&source, b"original").unwrap();
    let mut spec = Spec::new("sh", work.path());
    spec.source(&fs::File::open(&source).unwrap()).unwrap();
    spec.args(&[
        "-c",
        "! printf corrupted > /proc/self/fd/3 && ! touch \"$1\" && printf permitted > result",
        "sh",
    ])
    .arg(source_dir.path().join("outside"));
    let result = process::run(spec, &Control::default(), |_| Ok(())).unwrap();
    assert!(result.status.success(), "{}", result.stderr);
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert!(!source_dir.path().join("outside").exists());
    assert_eq!(fs::read(work.path().join("result")).unwrap(), b"permitted");
}

#[test]
fn backend_timeout_terminates_process_group() {
    let work = tempfile::tempdir().unwrap();
    let mut spec = Spec::new("sh", work.path());
    spec.args(&["-c", "sleep 60 & wait"]);
    spec.timeout = 1;
    let began = std::time::Instant::now();
    let error = process::run(spec, &Control::default(), |_| Ok(()))
        .err()
        .expect("timeout");
    assert!(error.to_string().contains("timeout"));
    assert!(began.elapsed().as_secs() < 5);
}

fn legacy_session(root: &Path, version: &str) -> Store {
    let sourcefile = root.join("source.img");
    fs::write(&sourcefile, vec![0; 8192]).unwrap();
    let source = SourceDescriptor {
        path: sourcefile,
        name: "Legacy fixture".into(),
        kind: SourceType::Image,
        filesystem: "ntfs".into(),
        size: 8192,
        sector_size: 512,
        mounted: false,
        parent_path: None,
        mountpoints: vec![],
        scope: None,
        fingerprint: Fingerprint {
            size: 8192,
            ..Default::default()
        },
        ancestry: vec![],
        ewf: false,
        segments: vec![],
        partition: None,
    };
    let connection = rusqlite::Connection::open(root.join("tree.sqlite")).unwrap();
    connection.execute_batch("CREATE TABLE files(id INTEGER PRIMARY KEY,path TEXT,size INTEGER,mtime INTEGER,inode TEXT,partition_offset INTEGER,partition_length INTEGER); INSERT INTO files VALUES(1,'photos/test.jpg',32,0,'39-128-4',1048576,67108864);").unwrap();
    drop(connection);
    let manifest = Manifest {
        schema_version: 0,
        id: "legacy".into(),
        created_at: 1,
        updated_at: 1,
        state: "complete".into(),
        source,
        options: ScanOptions::default(),
        engines: BTreeMap::from([("tsk".into(), version.into())]),
        clone: None,
        clone_map: None,
        scan_input: None,
        errors: vec![],
        candidate_count: 1,
        complete: true,
        consolidation: None,
        imaging: None,
    };
    store::write_json(&root.join("manifest.json"), &manifest).unwrap();
    Store::open(root).unwrap()
}

#[test]
fn legacy_load_is_read_only_and_uses_recorded_versions() {
    let dir = tempfile::tempdir().unwrap();
    let first = legacy_session(dir.path(), "old engine");
    let before = fs::read(dir.path().join("tree.sqlite")).unwrap();
    let mtime = fs::metadata(dir.path().join("tree.sqlite"))
        .unwrap()
        .modified()
        .unwrap();
    let file = first.get(1).unwrap();
    assert_eq!(file.locator, "39-128-4");
    assert_eq!(file.offset_bytes, 1048576);
    assert_eq!(file.backend_version, "old engine");
    assert_eq!(
        first
            .page(&Query {
                limit: 250,
                ..Default::default()
            })
            .unwrap()
            .files
            .len(),
        1
    );
    assert_eq!(fs::read(dir.path().join("tree.sqlite")).unwrap(), before);
    assert_eq!(
        fs::metadata(dir.path().join("tree.sqlite"))
            .unwrap()
            .modified()
            .unwrap(),
        mtime
    );
    assert!(!dir.path().join("selection.sqlite").exists());
    let mut second = Store::open(dir.path()).unwrap();
    second
        .manifest
        .engines
        .insert("tsk".into(), "new engine".into());
    assert!(
        store::compare_recorded(&first, &second)
            .unwrap()
            .contains("not directly comparable")
    );
}

#[test]
#[ignore = "Runs real recovery tools on disposable image files; run scripts/check-engines.sh"]
fn disposable_ext2_scan_recovery_clone_and_reopen() {
    let source_root = tempfile::tempdir().unwrap();
    let workspace_root = tempfile::Builder::new()
        .prefix("odr-engine-test-")
        .tempdir_in(
            std::env::var_os("ODR_TEST_STORAGE").expect("set ODR_TEST_STORAGE to separate storage"),
        )
        .unwrap();
    let image = source_root.path().join("source.img");
    let file = fs::File::create(&image).unwrap();
    file.set_len(16 * 1024 * 1024).unwrap();
    drop(file);
    assert!(
        Command::new("mkfs.ext2")
            .args(["-q", "-F"])
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    let payload = b"An actual recovered file, with its original bytes.\n";
    let original = source_root.path().join("original.txt");
    fs::write(&original, payload).unwrap();
    let request = format!("write {} recovered.txt", original.display());
    assert!(
        Command::new("debugfs")
            .args(["-w", "-R", &request])
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("debugfs")
            .args(["-w", "-R", "rm recovered.txt"])
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    let mut before = fs::File::open(&image).unwrap();
    let hash = jobs::hash_file(&mut before, &Control::default()).unwrap();
    let source = sources::inspect(&image).unwrap();
    let options = ScanOptions {
        workspace: workspace_root.path().join("scans"),
        mode: Mode::Quick,
        clone_first: true,
        extensions: vec!["txt".into()],
        ..Default::default()
    };
    let directory = jobs::scan(source, options, &Control::default(), &|e| {
        eprintln!("{e:?}")
    })
    .unwrap();
    let store = Store::open(&directory).unwrap();
    assert!(store.manifest.complete, "{:?}", store.manifest.errors);
    let page = store
        .page(&Query {
            limit: 250,
            ..Default::default()
        })
        .unwrap();
    let candidate = page
        .files
        .iter()
        .find(|f| f.path.ends_with("recovered.txt"))
        .expect("deleted file found");
    assert_eq!(candidate.bytes, payload.len() as u64);
    store.select(&[candidate.id], true).unwrap();
    assert_eq!(
        jobs::hash_file(&mut before, &Control::default()).unwrap(),
        hash
    );
    fs::rename(&image, source_root.path().join("disconnected.img")).unwrap();
    let destination = workspace_root.path().join("restored");
    jobs::recover(
        &directory,
        &RecoveryOptions {
            destination: destination.clone(),
            ..Default::default()
        },
        &Control::default(),
        &|e| eprintln!("{e:?}"),
    )
    .unwrap();
    assert_eq!(
        fs::read(destination.join("recovered.txt")).unwrap(),
        payload
    );
    let log = fs::read_to_string(directory.join("events.log")).unwrap();
    assert!(log.contains("\"verified\":true"));
    assert!(log.contains("\"sha256\":\""));
    let job = jobs::restart(directory);
    let mut completed = false;
    while let Ok(event) = job.events.recv_timeout(std::time::Duration::from_secs(30)) {
        if let Event::Finished {
            error, cancelled, ..
        } = event
        {
            assert!(!cancelled);
            assert!(error.is_none(), "{error:?}");
            completed = true;
            break;
        }
    }
    assert!(completed);
}

#[test]
#[ignore = "Runs PhotoRec on a disposable image; run scripts/check-engines.sh"]
fn disposable_photorec_content_carving() {
    let source_root = tempfile::tempdir().unwrap();
    let workspace = tempfile::Builder::new()
        .prefix("odr-carve-test-")
        .tempdir_in(std::env::var_os("ODR_TEST_STORAGE").expect("separate test storage"))
        .unwrap();
    let payload = include_bytes!("../../tests/fixtures/recovery.png");
    let image = source_root.path().join("raw.img");
    let mut bytes = vec![0_u8; 1024 * 1024];
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&vec![0; 1024 * 1024]);
    fs::write(&image, bytes).unwrap();
    let source = sources::inspect(&image).unwrap();
    let options = ScanOptions {
        workspace: workspace.path().join("scans"),
        mode: Mode::Deep,
        carver: "photorec".into(),
        extensions: vec!["png".into()],
        timeout_seconds: 20,
        ..Default::default()
    };
    let directory = jobs::scan(source, options, &Control::default(), &|e| {
        eprintln!("{e:?}")
    })
    .unwrap();
    let store = Store::open(&directory).unwrap();
    assert!(store.manifest.complete, "{:?}", store.manifest.errors);
    let files = store
        .page(&Query {
            limit: 250,
            ..Default::default()
        })
        .unwrap()
        .files;
    assert!(!files.is_empty(), "PhotoRec must find the embedded PNG");
    assert!(files.iter().any(|f| {
        f.artifact
            .as_ref()
            .is_some_and(|p| fs::read(directory.join(p)).is_ok_and(|data| data == payload))
    }));
}

#[test]
#[ignore = "Runs optional packaged engines on disposable images"]
fn disposable_optional_carvers_and_ntfs_reconstruction() {
    let versions = adapters::versions();
    let source_root = tempfile::tempdir().unwrap();
    let workspace = tempfile::Builder::new()
        .prefix("odr-extra-test-")
        .tempdir_in(std::env::var_os("ODR_TEST_STORAGE").expect("separate test storage"))
        .unwrap();
    let payload = include_bytes!("../../tests/fixtures/recovery.png");
    let raw = source_root.path().join("raw.img");
    let mut bytes = vec![0; 1024 * 1024];
    bytes.extend_from_slice(payload);
    bytes.resize(bytes.len() + 1024 * 1024, 0);
    fs::write(&raw, bytes).unwrap();
    for engine in ["scalpel", "foremost", "digler", "magicrescue"] {
        if !versions.contains_key(engine) {
            eprintln!("Skipping unavailable optional engine {engine}");
            continue;
        }
        let options = ScanOptions {
            workspace: workspace.path().join(engine),
            mode: Mode::Deep,
            carver: engine.into(),
            extensions: vec!["png".into()],
            timeout_seconds: 20,
            ..Default::default()
        };
        let dir = jobs::scan(
            sources::inspect(&raw).unwrap(),
            options,
            &Control::default(),
            &|e| eprintln!("{e:?}"),
        )
        .unwrap();
        let store = Store::open(&dir).unwrap();
        assert!(
            store.manifest.complete,
            "{engine}: {:?}",
            store.manifest.errors
        );
        let files = store
            .page(&Query {
                limit: 250,
                ..Default::default()
            })
            .unwrap()
            .files;
        assert!(!files.is_empty(), "{engine} did not find the embedded PNG");
    }
    if !versions.contains_key("recuperabit") {
        eprintln!("Skipping unavailable optional RecuperaBit");
        return;
    }
    let ntfs = source_root.path().join("ntfs.img");
    fs::File::create(&ntfs)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    assert!(
        Command::new("mkfs.ntfs")
            .args(["-F", "-Q", "-q"])
            .arg(&ntfs)
            .status()
            .unwrap()
            .success()
    );
    let original = source_root.path().join("original.txt");
    let contents = b"NTFS reconstruction test payload\n";
    fs::write(&original, contents).unwrap();
    assert!(
        Command::new("ntfscp")
            .arg(&ntfs)
            .arg(&original)
            .arg("/recovered.txt")
            .status()
            .unwrap()
            .success()
    );
    for engine in ["tsk", "recuperabit"] {
        let options = ScanOptions {
            workspace: workspace.path().join(engine),
            mode: Mode::Quick,
            metadata_backend: engine.into(),
            include_existing: true,
            extensions: vec!["txt".into()],
            timeout_seconds: 20,
            ..Default::default()
        };
        let dir = jobs::scan(
            sources::inspect(&ntfs).unwrap(),
            options,
            &Control::default(),
            &|e| eprintln!("{e:?}"),
        )
        .unwrap();
        let store = Store::open(&dir).unwrap();
        assert!(
            store.manifest.complete,
            "{engine}: {:?}",
            store.manifest.errors
        );
        let files = store
            .page(&Query {
                limit: 250,
                ..Default::default()
            })
            .unwrap()
            .files;
        let candidate = files
            .iter()
            .find(|f| f.path.ends_with("recovered.txt"))
            .expect("named NTFS file");
        store.select(&[candidate.id], true).unwrap();
        if engine == "tsk" {
            assert!(
                candidate.locator.contains('-'),
                "NTFS compound address must survive"
            );
        }
        let destination = workspace.path().join(format!("recovered-{engine}"));
        jobs::recover(
            &dir,
            &RecoveryOptions {
                destination: destination.clone(),
                ..Default::default()
            },
            &Control::default(),
            &|_| {},
        )
        .unwrap();
        assert_eq!(
            fs::read(destination.join(&candidate.path)).unwrap(),
            contents
        );
    }
}

#[test]
#[ignore = "Runs E01 normalization on a disposable image"]
fn disposable_ewf_normalization() {
    let source_root = tempfile::tempdir().unwrap();
    let workspace = tempfile::Builder::new()
        .prefix("odr-ewf-test-")
        .tempdir_in(std::env::var_os("ODR_TEST_STORAGE").expect("separate test storage"))
        .unwrap();
    let image = source_root.path().join("ext4.img");
    fs::File::create(&image)
        .unwrap()
        .set_len(32 * 1024 * 1024)
        .unwrap();
    assert!(
        Command::new("mkfs.ext4")
            .args(["-q", "-F", "-O", "^metadata_csum,^64bit"])
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    let original = source_root.path().join("original.txt");
    fs::write(&original, b"Retained E01 and ext4 recovery fixture\n").unwrap();
    assert!(
        Command::new("debugfs")
            .args([
                "-w",
                "-R",
                &format!("write {} recovered.txt", original.display())
            ])
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    let evidence = source_root.path().join("evidence");
    assert!(
        Command::new("ewfacquire")
            .args(["-u", "-q", "-t"])
            .arg(&evidence)
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    let source = sources::inspect(&evidence.with_extension("E01")).unwrap();
    assert_eq!(source.size, 32 * 1024 * 1024);
    let options = ScanOptions {
        workspace: workspace.path().join("ewf"),
        mode: Mode::Quick,
        metadata_backend: "tsk".into(),
        include_existing: true,
        extensions: vec!["txt".into()],
        timeout_seconds: 20,
        ..Default::default()
    };
    let dir = jobs::scan(source, options, &Control::default(), &|e| {
        eprintln!("{e:?}")
    })
    .unwrap();
    let store = Store::open(&dir).unwrap();
    assert!(store.manifest.complete, "{:?}", store.manifest.errors);
    assert!(
        store
            .page(&Query {
                limit: 250,
                ..Default::default()
            })
            .unwrap()
            .files
            .iter()
            .any(|f| f.path.ends_with("recovered.txt"))
    );
}
