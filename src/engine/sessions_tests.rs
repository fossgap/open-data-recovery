use super::{
    sessions::{self, Deletion},
    store::{self, Store},
    types::*,
};
use std::{collections::BTreeMap, fs, os::unix::fs::symlink, path::Path};

fn fixture(root: &Path) -> Store {
    let source = SourceDescriptor {
        path: root.join("disconnected-source.img"),
        name: "Deletion fixture".into(),
        kind: SourceType::Image,
        filesystem: "ext2".into(),
        size: 4096,
        sector_size: 512,
        mounted: false,
        parent_path: None,
        mountpoints: vec![],
        scope: None,
        fingerprint: Fingerprint::default(),
        ancestry: vec!["disconnected-test-source".into()],
        ewf: false,
        segments: vec![],
        partition: None,
    };
    Store::create(
        &source,
        &ScanOptions {
            workspace: root.join("scans"),
            ..Default::default()
        },
        BTreeMap::new(),
    )
    .unwrap()
}
fn saved(store: &Store) -> SavedScan {
    SavedScan {
        directory: store.directory.clone(),
        manifest: store.manifest.clone(),
    }
}

#[test]
fn deleting_a_snapshot_removes_its_folder_but_preserves_external_files_and_other_scans() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let scan = saved(&store);
    let other = fixture(root.path());
    let other_dir = other.directory.clone();
    drop(other);
    fs::create_dir_all(scan.directory.join("artifacts/nested")).unwrap();
    fs::write(
        scan.directory.join("artifacts/nested/cached.bin"),
        b"cached contents",
    )
    .unwrap();
    let recovered = root.path().join("recovered.bin");
    fs::write(&recovered, b"final recovered contents").unwrap();
    fs::hard_link(&recovered, scan.directory.join("retained-input.img")).unwrap();
    let external = root.path().join("external");
    fs::create_dir(&external).unwrap();
    fs::write(external.join("source.img"), b"source contents").unwrap();
    symlink(&external, scan.directory.join("artifacts/external-link")).unwrap();
    drop(store);
    Deletion::prepare(scan.clone()).unwrap().execute().unwrap();
    assert!(!scan.directory.exists());
    assert_eq!(fs::read(&recovered).unwrap(), b"final recovered contents");
    assert_eq!(
        fs::read(external.join("source.img")).unwrap(),
        b"source contents"
    );
    assert!(other_dir.join("manifest.json").exists());
    assert_eq!(store::list(&root.path().join("scans")).unwrap().len(), 1);
}

#[test]
fn active_scans_and_open_catalogues_block_deletion_including_after_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let scan = saved(&store);
    assert!(Deletion::prepare(scan.clone()).is_err());
    drop(store);
    let plan = Deletion::prepare(scan.clone()).unwrap();
    let open = Store::open(&scan.directory).unwrap();
    assert!(plan.execute().unwrap_err().to_string().contains("in use"));
    assert!(scan.directory.join("tree.sqlite").exists());
    drop(open);
    plan.execute().unwrap();
}

#[test]
fn deletion_rejects_a_replaced_folder_and_a_changed_scan_identity() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let scan = saved(&store);
    drop(store);
    let plan = Deletion::prepare(scan.clone()).unwrap();
    let moved = root.path().join("original-scan");
    fs::rename(&scan.directory, &moved).unwrap();
    fs::create_dir(&scan.directory).unwrap();
    fs::write(scan.directory.join("unrelated"), b"keep").unwrap();
    assert!(plan.execute().unwrap_err().to_string().contains("replaced"));
    assert_eq!(fs::read(scan.directory.join("unrelated")).unwrap(), b"keep");
    assert!(moved.join("tree.sqlite").exists());
    fs::remove_file(scan.directory.join("unrelated")).unwrap();
    fs::remove_dir(&scan.directory).unwrap();
    fs::rename(&moved, &scan.directory).unwrap();
    let mut changed = scan.manifest.clone();
    changed.id = "different-scan".into();
    store::write_json(&scan.directory.join("manifest.json"), &changed).unwrap();
    assert!(plan.execute().unwrap_err().to_string().contains("changed"));
    assert!(scan.directory.join("tree.sqlite").exists());
}

#[test]
fn deletion_rejects_symlink_roots_source_overlap_and_nested_mounts() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let scan = saved(&store);
    drop(store);
    let link = root.path().join("scan-link");
    symlink(&scan.directory, &link).unwrap();
    let mut linked = scan.clone();
    linked.directory = link;
    assert!(Deletion::prepare(linked).is_err());
    let mut overlapping = scan.clone();
    overlapping.manifest.source.path = scan.directory.join("source.img");
    store::write_json(&scan.directory.join("manifest.json"), &overlapping.manifest).unwrap();
    assert!(Deletion::prepare(overlapping).is_err());
    assert!(scan.directory.join("tree.sqlite").exists());
    let error =
        sessions::rejects_mount_boundary(Path::new("/"), std::ffi::OsStr::new("proc")).unwrap_err();
    assert!(error.to_string().contains("mount point"));
}

#[test]
fn deletion_can_finish_a_partly_deleted_snapshot_with_its_manifest_remaining() {
    let root = tempfile::tempdir().unwrap();
    let store = fixture(root.path());
    let scan = saved(&store);
    drop(store);
    fs::remove_file(scan.directory.join("tree.sqlite")).unwrap();
    Deletion::prepare(scan.clone()).unwrap().execute().unwrap();
    assert!(!scan.directory.exists());
}
