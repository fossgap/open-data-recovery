use super::{
    consolidation, jobs,
    process::{Control, Spec},
    safety, sources,
    store::Store,
    types::*,
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
};

// Artifact-only tests model a disconnected source. Source-protection behavior
// against an actual regular image is exercised by the disposable test below.
fn fixture(root: &Path) -> (Store, File) {
    let input = root.join("unused-input.img");
    fs::write(&input, [0; 8192]).unwrap();
    let source = SourceDescriptor {
        path: root.join("disconnected.img"),
        name: "Consolidation fixture".into(),
        kind: SourceType::Image,
        filesystem: "ext2".into(),
        size: 8192,
        sector_size: 512,
        mounted: false,
        parent_path: None,
        mountpoints: vec![],
        scope: None,
        fingerprint: Fingerprint::default(),
        ancestry: vec!["disconnected-test-source".into()],
        ewf: false,
        segments: vec![],
    };
    let options = ScanOptions {
        workspace: root.join("scans"),
        consolidate: true,
        ..Default::default()
    };
    (
        Store::create(&source, &options, BTreeMap::new()).unwrap(),
        File::open(input).unwrap(),
    )
}
fn artifact(store: &mut Store, path: &str, backend: &str, bytes: &[u8]) -> PathBuf {
    let id = store.count().unwrap() + 1;
    let relative = format!("artifacts/{id}.bin");
    let file = store.directory.join(&relative);
    fs::write(&file, bytes).unwrap();
    store
        .insert(&Candidate {
            path: path.into(),
            bytes: bytes.len() as u64,
            backend: backend.into(),
            backend_version: "fixture-v1".into(),
            locator: id.to_string(),
            artifact: Some(relative),
            carved: backend == "photorec",
            deleted: backend != "photorec",
            ..Default::default()
        })
        .unwrap();
    file
}
fn finish(store: &mut Store, input: &File) {
    consolidation::run(store, input, &Control::default(), &|_| {}).unwrap();
    store.manifest.complete = store.manifest.errors.is_empty();
    store.manifest.state = if store.manifest.complete {
        "complete"
    } else {
        "partial"
    }
    .into();
    store.checkpoint().unwrap();
    store.save().unwrap();
}
fn page(store: &Store) -> Page {
    store
        .page(&Query {
            limit: 100,
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn consolidation_is_opt_in_and_old_options_still_load() {
    assert!(!ScanOptions::default().consolidate);
    assert!(
        !serde_json::from_str::<ScanOptions>(r#"{"mode":"Quick"}"#)
            .unwrap()
            .consolidate
    );
    assert!(
        !serde_json::from_str::<RecoveryOptions>("{}")
            .unwrap()
            .restore_all_paths
    );
}

#[test]
fn active_scans_block_selection_but_interrupted_scans_keep_raw_findings_usable() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, _) = fixture(root.path());
    artifact(&mut store, "one.txt", "testdisk", b"same");
    store.manifest.state = "consolidating".into();
    store.save().unwrap();
    let reopened = Store::open(&store.directory).unwrap();
    assert!(
        reopened
            .select(&[1], true)
            .unwrap_err()
            .to_string()
            .contains("Wait for")
    );
    // Dropping the owner models OS lock release after a crash, without a final
    // manifest update. The original candidate remains selectable/recoverable.
    drop(store);
    reopened.select(&[1], true).unwrap();
    jobs::recover(
        &reopened.directory,
        &RecoveryOptions {
            destination: root.path().join("restored"),
            ..Default::default()
        },
        &Control::default(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(
        fs::read(root.path().join("restored/one.txt")).unwrap(),
        b"same"
    );
}

#[test]
fn stale_raw_selection_cannot_be_applied_after_group_publication() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, input) = fixture(root.path());
    artifact(&mut store, "one.txt", "testdisk", b"same");
    artifact(&mut store, "two.txt", "testdisk", b"same");
    let stale = Store::open(&store.directory).unwrap();
    finish(&mut store, &input);
    drop(store);
    assert!(
        stale
            .select(&[2], true)
            .unwrap_err()
            .to_string()
            .contains("Reload")
    );
    let reopened = Store::open(&stale.directory).unwrap();
    reopened.select(&[1], true).unwrap();
    assert_eq!(page(&reopened).selected, 1);
}

#[test]
fn identical_contents_preserve_provenance_alias_search_selection_and_all_paths() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, input) = fixture(root.path());
    let files = [
        artifact(&mut store, "Pictures/original.jpg", "tsk", b"same"),
        artifact(&mut store, "photorec/f001.jpg", "photorec", b"same"),
        artifact(&mut store, "Pictures/alias.jpg", "ntfsundelete", b"same"),
        artifact(&mut store, "Pictures/original.jpg", "testdisk", b"same"),
        artifact(&mut store, "Pictures/different.jpg", "tsk", b"else"),
        artifact(&mut store, "unique.txt", "tsk", b"different length"),
        artifact(&mut store, "empty-a.txt", "tsk", b""),
        artifact(&mut store, "empty-b.txt", "photorec", b""),
    ];
    let raw_before = page(&store)
        .files
        .iter()
        .map(|c| (c.id, c.path.clone(), c.artifact.clone()))
        .collect::<Vec<_>>();
    finish(&mut store, &input);
    let reopened = Store::open(&store.directory).unwrap();
    let p = page(&reopened);
    assert!(p.consolidated);
    assert_eq!((p.total, p.findings), (4, 8));
    assert_eq!(
        reopened
            .manifest
            .consolidation
            .as_ref()
            .unwrap()
            .cached_bytes,
        0
    );
    let group = p
        .files
        .iter()
        .find(|f| f.path == "Pictures/original.jpg")
        .unwrap();
    assert_eq!((group.findings, group.paths), (4, 3));
    assert_eq!(group.backend, "tsk");
    assert!(group.sha256.is_some());
    let aliases = reopened.findings(group.id, 0).unwrap();
    assert_eq!(aliases.len(), 4);
    assert_eq!(
        aliases
            .iter()
            .map(|f| f.backend.as_str())
            .collect::<Vec<_>>(),
        vec!["tsk", "photorec", "ntfsundelete", "testdisk"]
    );
    let matched = reopened
        .page(&Query {
            text: "alias.jpg".into(),
            limit: 100,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(matched.matched, 1);
    assert_eq!(matched.files[0].id, group.id);
    for raw in raw_before {
        let saved: (i64, String, Option<String>) = store
            .connection
            .query_row(
                "SELECT id,path,artifact FROM candidates WHERE id=?1",
                [raw.0],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(raw, saved);
    }
    assert_eq!(fs::read(&files[0]).unwrap(), b"same");
    assert!(files.iter().all(|p| p.is_file()));
    reopened.select(&[group.id], true).unwrap();
    let selected = page(&reopened);
    assert_eq!(
        (
            selected.selected,
            selected.selected_bytes,
            selected.selected_paths,
            selected.selected_path_bytes
        ),
        (1, 4, 3, 12)
    );
    let one = root.path().join("one-copy");
    jobs::recover(
        &store.directory,
        &RecoveryOptions {
            destination: one.clone(),
            ..Default::default()
        },
        &Control::default(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(
        fs::read(one.join("Pictures/original.jpg")).unwrap(),
        b"same"
    );
    assert!(!one.join("Pictures/alias.jpg").exists());
    let all = root.path().join("all-paths");
    let outcome = std::cell::Cell::new(0);
    jobs::recover(
        &store.directory,
        &RecoveryOptions {
            destination: all.clone(),
            restore_all_paths: true,
            ..Default::default()
        },
        &Control::default(),
        &|event| {
            if let Event::Recovered {
                successful,
                failed,
                skipped,
            } = event
            {
                assert_eq!((failed, skipped), (0, 0));
                outcome.set(successful);
            }
        },
    )
    .unwrap();
    assert_eq!(outcome.get(), 3);
    for name in [
        "Pictures/original.jpg",
        "Pictures/alias.jpg",
        "photorec/f001.jpg",
    ] {
        assert_eq!(fs::read(all.join(name)).unwrap(), b"same");
    }
    // A later change cannot silently alter the meaning of a stable group.
    fs::write(
        safety::confined_artifact(&store.directory, group.artifact.as_deref().unwrap()).unwrap(),
        b"evil",
    )
    .unwrap();
    let error = jobs::recover(
        &store.directory,
        &RecoveryOptions {
            destination: root.path().join("changed-copy"),
            verify_after_write: false,
            continue_on_error: false,
            ..Default::default()
        },
        &Control::default(),
        &|_| {},
    )
    .unwrap_err();
    assert!(error.to_string().contains("no longer matches"));
    assert_eq!(page(&Store::open(&store.directory).unwrap()).total, 4);
}

#[test]
fn cancellation_rolls_back_the_group_view_and_preserves_every_finding() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, input) = fixture(root.path());
    let originals = (0..4)
        .map(|i| artifact(&mut store, &format!("{i}.txt"), "testdisk", b"same"))
        .collect::<Vec<_>>();
    let control = Control::default();
    let result = consolidation::run(&mut store, &input, &control, &|event| {
        if let Event::Stage {
            fraction: Some(f), ..
        } = event
        {
            if f > 0.0 {
                control.cancel();
            }
        }
    });
    assert!(matches!(result, Err(super::Error::Cancelled)));
    assert!(store.manifest.consolidation.is_none());
    let reopened = Store::open(&store.directory).unwrap();
    assert!(!page(&reopened).consolidated);
    assert_eq!(page(&reopened).total, 4);
    assert!(!store.directory.join("artifacts/consolidation").exists());
    assert!(originals.iter().all(|p| fs::read(p).unwrap() == b"same"));
}

#[test]
fn missing_and_wrong_length_findings_stay_separate_and_report_incomplete_verification() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, input) = fixture(root.path());
    artifact(&mut store, "good.txt", "testdisk", b"same");
    let missing = artifact(&mut store, "missing.txt", "photorec", b"same");
    fs::remove_file(missing).unwrap();
    let truncated = artifact(&mut store, "short.txt", "tsk", b"same");
    fs::write(truncated, b"sam").unwrap();
    finish(&mut store, &input);
    assert!(!store.manifest.complete);
    assert_eq!(store.manifest.consolidation.as_ref().unwrap().unverified, 2);
    let p = page(&Store::open(&store.directory).unwrap());
    assert_eq!(p.total, 3);
    assert_eq!(
        p.files
            .iter()
            .filter(|f| f.consolidation_error.is_some())
            .count(),
        2
    );
    assert!(p.files.iter().all(|f| f.findings == 1));
}

#[test]
fn changed_artifacts_abort_publication_instead_of_publishing_stale_content() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, input) = fixture(root.path());
    let first = artifact(&mut store, "one.txt", "testdisk", b"same");
    artifact(&mut store, "two.txt", "photorec", b"same");
    let changed = std::cell::Cell::new(false);
    let result = consolidation::run(&mut store, &input, &Control::default(), &|event| {
        if let Event::Stage {
            fraction: Some(f), ..
        } = event
        {
            if f > 0.0 && !changed.replace(true) {
                fs::write(&first, b"evil").unwrap();
            }
        }
    });
    assert!(result.is_err());
    assert!(store.manifest.consolidation.is_none());
    assert_eq!(page(&Store::open(&store.directory).unwrap()).total, 2);
}

#[test]
fn grouping_and_findings_are_paged_without_losing_large_groups() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, input) = fixture(root.path());
    for i in 0..257 {
        artifact(&mut store, &format!("folder/{i}.txt"), "testdisk", b"same");
    }
    finish(&mut store, &input);
    let group = page(&store).files.remove(0);
    assert_eq!((group.findings, group.paths), (257, 257));
    let a = store.findings(group.id, 0).unwrap();
    let b = store.findings(group.id, a.last().unwrap().id).unwrap();
    let c = store.findings(group.id, b.last().unwrap().id).unwrap();
    assert_eq!((a.len(), b.len(), c.len()), (100, 100, 57));
    let a = store.recovery_paths(&group, None).unwrap();
    let b = store
        .recovery_paths(&group, Some(&a.last().unwrap().path))
        .unwrap();
    let c = store
        .recovery_paths(&group, Some(&b.last().unwrap().path))
        .unwrap();
    assert_eq!((a.len(), b.len(), c.len()), (100, 100, 57));
}

#[test]
fn empty_and_unique_size_catalogues_do_not_extract_lazy_findings() {
    for count in [0, 2] {
        let root = tempfile::tempdir().unwrap();
        let (mut store, input) = fixture(root.path());
        for i in 0..count {
            store
                .insert(&Candidate {
                    path: format!("unique-{i}.bin"),
                    bytes: 100 + i,
                    backend: "tsk".into(),
                    locator: "not-an-inode".into(),
                    ..Default::default()
                })
                .unwrap();
        }
        finish(&mut store, &input);
        assert!(store.manifest.complete);
        assert_eq!(page(&store).total, count);
        assert!(
            page(&store)
                .files
                .iter()
                .all(|c| c.artifact.is_none() && c.sha256.is_none())
        );
        assert_eq!(
            store.manifest.consolidation.as_ref().unwrap().cached_bytes,
            0
        );
    }
}

#[test]
fn insufficient_cache_space_aborts_without_publishing_partial_groups() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, input) = fixture(root.path());
    for i in 0..2 {
        store
            .insert(&Candidate {
                path: format!("huge-{i}.bin"),
                bytes: i64::MAX as u64,
                backend: "tsk".into(),
                locator: "not-an-inode".into(),
                ..Default::default()
            })
            .unwrap();
    }
    let error = consolidation::run(&mut store, &input, &Control::default(), &|_| {}).unwrap_err();
    assert!(error.to_string().contains("space"));
    assert!(store.manifest.consolidation.is_none());
    assert_eq!(page(&Store::open(&store.directory).unwrap()).total, 2);
    assert!(!store.directory.join("artifacts/consolidation").exists());
}

#[test]
fn paused_backend_can_be_cancelled_with_exit_notifications() {
    let root = tempfile::tempdir().unwrap();
    let mut spec = Spec::new("sh", root.path());
    spec.args(&["-c", "printf ready > ready; sleep 60 & wait"]);
    let control = Control::default();
    let worker = control.clone();
    let thread = std::thread::spawn(move || super::process::run(spec, &worker, |_| Ok(())));
    let started = std::time::Instant::now();
    while !root.path().join("ready").exists() {
        assert!(started.elapsed().as_secs() < 5);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    control.pause(true);
    std::thread::sleep(std::time::Duration::from_millis(80));
    control.cancel();
    assert!(matches!(
        thread.join().unwrap(),
        Err(super::Error::Cancelled)
    ));
}

#[test]
#[ignore = "Runs real TSK consolidation/recovery on a disposable image with separate storage"]
fn disposable_consolidation_extracts_once_reopens_and_restores_every_path_without_source() {
    let source_root = tempfile::tempdir().unwrap();
    let work = tempfile::Builder::new()
        .prefix("odr-consolidation-test-")
        .tempdir_in(std::env::var_os("ODR_TEST_STORAGE").expect("separate test storage"))
        .unwrap();
    let image = source_root.path().join("source.img");
    File::create(&image)
        .unwrap()
        .set_len(16 * 1024 * 1024)
        .unwrap();
    assert!(
        Command::new("mkfs.ext2")
            .args(["-F", "-q"])
            .arg(&image)
            .status()
            .unwrap()
            .success()
    );
    let original = source_root.path().join("payload.txt");
    let payload = b"Consolidated recovery preserves every original path.\n";
    fs::write(&original, payload).unwrap();
    for name in ["one.txt", "alias.txt"] {
        assert!(
            Command::new("debugfs")
                .args(["-w", "-R", &format!("write {} {name}", original.display())])
                .arg(&image)
                .status()
                .unwrap()
                .success()
        );
    }
    for name in ["one.txt", "alias.txt"] {
        assert!(
            Command::new("debugfs")
                .args(["-w", "-R", &format!("rm {name}")])
                .arg(&image)
                .status()
                .unwrap()
                .success()
        );
    }
    let before = jobs::hash_file(&mut File::open(&image).unwrap(), &Control::default()).unwrap();
    let options = ScanOptions {
        workspace: work.path().join("scans"),
        mode: Mode::Quick,
        extensions: vec!["txt".into()],
        consolidate: true,
        ..Default::default()
    };
    let dir = jobs::scan(
        sources::inspect(&image).unwrap(),
        options,
        &Control::default(),
        &|_| {},
    )
    .unwrap();
    let store = Store::open(&dir).unwrap();
    assert!(store.manifest.complete, "{:?}", store.manifest.errors);
    assert_eq!((page(&store).total, page(&store).findings), (1, 2));
    assert_eq!(
        store.manifest.consolidation.as_ref().unwrap().cached_bytes,
        payload.len() as u64
    );
    assert_eq!(
        before,
        jobs::hash_file(&mut File::open(&image).unwrap(), &Control::default()).unwrap()
    );
    fs::remove_file(&image).unwrap();
    let group = page(&store).files.remove(0);
    store.select(&[group.id], true).unwrap();
    let destination = work.path().join("recovered");
    jobs::recover(
        &dir,
        &RecoveryOptions {
            destination: destination.clone(),
            restore_all_paths: true,
            ..Default::default()
        },
        &Control::default(),
        &|_| {},
    )
    .unwrap();
    assert_eq!(fs::read(destination.join("one.txt")).unwrap(), payload);
    assert_eq!(fs::read(destination.join("alias.txt")).unwrap(), payload);
}
