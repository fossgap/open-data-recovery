use super::{
    discovery::{self, Options, Table},
    jobs,
    process::Control,
    sources,
    store::{self, Store},
    types::*,
};
use std::{
    fs::{self, File},
    io::{Seek, SeekFrom, Write},
    path::PathBuf,
    process::Command,
};

fn log(table: &str, sector: u32, rows: &str) -> String {
    format!(
        "Disk /proc/self/fd/3 - CHS 20 255 63, sector size={sector}\nPartition table type: {table}\nCurrent partition structure:\n     Linux 0 0 1 0 32 32 2048\nsearch_part()\n{rows}\ninterface_write()\nTestDisk exited normally.\n"
    )
}
#[test]
fn testdisk_chs_ranges_are_bounded_deduplicated_and_overlap_is_preserved() {
    let rows = "     Linux 0 65 2 2 75 9 32768 [Lost Files]\n     ext2 blocksize=1024\nResults\n   * Linux 0 65 2 2 75 9 32768 [Lost Files]\n     ext2 blocksize=1024\n     Linux 0 64 63 2 75 7 32768 [Lost Files]\n     ext2 blocksize=1024 Backup_SB\necho \"0 32768 linear /proc/self/fd/3 4094\" | dmsetup create test0";
    let (table, found, warnings) =
        discovery::parse_log(&log("Intel", 512, rows), 32 * 1024 * 1024, 512).unwrap();
    assert_eq!(table, "Intel");
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].offset_bytes, 4094 * 512);
    assert_eq!(found[1].offset_bytes, 2 * 1024 * 1024);
    assert_eq!(found[1].length_bytes, 16 * 1024 * 1024);
    assert_eq!(found[1].filesystem, "ext2");
    assert_eq!(found[1].label, "Lost Files");
    assert!(found[1].description.starts_with("Linux"));
    assert!(warnings.iter().any(|s| s.contains("overlap")));
}
#[test]
fn testdisk_gpt_uses_lba_and_the_reported_sector_size() {
    for sector in [512, 4096] {
        let (_, found, _) = discovery::parse_log(
            &log(
                "EFI GPT",
                sector,
                "     Linux filesys. data 256 4351 4096 [Archive]\n     ext4 blocksize=4096",
            ),
            64 * 1024 * 1024,
            sector,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].offset_bytes, 256 * sector as u64);
        assert_eq!(found[0].length_bytes, 4096 * sector as u64);
        assert_eq!(found[0].filesystem, "ext4");
    }
}
#[test]
fn incomplete_unknown_or_misaligned_testdisk_output_is_not_accepted() {
    let valid = log("Intel", 512, "     Linux 0 65 2 2 75 9 32768");
    for invalid in [
        valid.replace("TestDisk exited normally.", ""),
        valid.replace("Intel", "None"),
        valid.replace("sector size=512", "sector size=4096"),
        valid.replace("CHS 20 255 63", "CHS 20 0 0"),
        valid.replace("search_part()", "not a search"),
    ] {
        assert!(discovery::parse_log(&invalid, 32 * 1024 * 1024, 512).is_err());
    }
    for rows in [
        "     Linux filesys. data 4096 99999999 99995904",
        "     Linux filesys. data 4096 8192 4096",
        "     Linux filesys. data 18446744073709551600 18446744073709551615 16",
        "     Linux filesys. data 100 99 0",
    ] {
        let (_, found, _) =
            discovery::parse_log(&log("EFI GPT", 512, rows), 32 * 1024 * 1024, 512).unwrap();
        assert!(found.is_empty());
    }
    assert!(
        discovery::parse_log(&log("Intel", 512, ""), 32 * 1024 * 1024, 512)
            .unwrap()
            .1
            .is_empty()
    );
}
#[test]
fn partition_search_commands_have_no_partition_write_or_repair_operations() {
    for (table, prefix) in [
        (Table::Auto, ""),
        (Table::Intel, "partition_i386,"),
        (Table::Gpt, "partition_gpt,"),
    ] {
        for deeper in [false, true] {
            let options = Options {
                workspace: PathBuf::from("/tmp/test"),
                table,
                deeper,
                timeout_seconds: 0,
            };
            assert_eq!(
                options.command(),
                format!("{prefix}analyze{}", if deeper { ",search" } else { "" })
            );
        }
    }
    assert!(serde_json::from_str::<Table>("\"intel,write\"").is_err());
}
fn source() -> SourceDescriptor {
    SourceDescriptor {
        path: "/tmp/disk.img".into(),
        name: "Disk".into(),
        kind: SourceType::Image,
        filesystem: String::new(),
        size: 32 * 1024 * 1024,
        sector_size: 512,
        mounted: false,
        parent_path: None,
        mountpoints: vec![],
        scope: None,
        fingerprint: Fingerprint::default(),
        ancestry: vec!["original-drive".into()],
        ewf: false,
        segments: vec![],
        partition: None,
    }
}
fn partition() -> DiscoveredPartition {
    DiscoveredPartition {
        offset_bytes: 2 * 1024 * 1024,
        length_bytes: 16 * 1024 * 1024,
        sector_size: 512,
        filesystem: "ext2".into(),
        label: "Lost".into(),
        description: "Linux".into(),
        verified: true,
        sample_sha256: "a".repeat(64),
    }
}
#[test]
fn discovered_targets_preserve_original_identity_and_legacy_sources_still_load() {
    let root = source();
    let mut partition = partition();
    let target = sources::partition_source(&root, &partition).unwrap();
    assert_eq!(target.path, root.path);
    assert_eq!(target.fingerprint, root.fingerprint);
    assert_eq!(target.ancestry, root.ancestry);
    assert_eq!(target.size, root.size);
    assert_eq!(target.target_size(), partition.length_bytes);
    assert_eq!(target.target_offset(), partition.offset_bytes);
    assert!(!target.same_target(&root));
    assert!(!discovery::eligible(&target));
    partition.offset_bytes += 512;
    assert!(!target.same_target(&sources::partition_source(&root, &partition).unwrap()));
    partition.offset_bytes = root.size;
    assert!(sources::partition_source(&root, &partition).is_err());
    let mut json = serde_json::to_value(&root).unwrap();
    json.as_object_mut().unwrap().remove("partition");
    assert!(
        serde_json::from_value::<SourceDescriptor>(json)
            .unwrap()
            .partition
            .is_none()
    );
}

#[test]
fn reopening_partition_reports_rejects_invalid_or_incomplete_targets() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("report.json");
    let report = discovery::Report {
        schema_version: 1,
        directory: "/old/report/location".into(),
        source: source(),
        options: Options {
            workspace: root.path().into(),
            table: Table::Intel,
            deeper: false,
            timeout_seconds: 0,
        },
        created_at: 0,
        testdisk_version: "fixture".into(),
        table: "Intel".into(),
        complete: true,
        cancelled: false,
        error: None,
        warnings: vec![],
        partitions: vec![partition()],
    };
    store::write_json(&path, &report).unwrap();
    let loaded = discovery::load(&path).unwrap();
    assert_eq!(loaded.path(), fs::canonicalize(&path).unwrap());
    assert!(loaded.target(0).is_ok());
    assert!(loaded.target(1).is_err());
    for case in 0..9 {
        let mut invalid = report.clone();
        match case {
            0 => invalid.complete = false,
            1 => invalid.cancelled = true,
            2 => invalid.error = Some("interrupted".into()),
            3 => invalid.schema_version = 99,
            4 => invalid.partitions[0].offset_bytes = invalid.source.size,
            5 => invalid.partitions[0].sample_sha256.clear(),
            6 => invalid.partitions[0].sector_size = 4096,
            7 => invalid.source.kind = SourceType::Folder,
            _ => invalid.table = "None".into(),
        }
        store::write_json(&path, &invalid).unwrap();
        assert!(discovery::load(&path).is_err(), "case {case}");
    }
}

#[test]
fn scan_comparison_distinguishes_partitions_on_the_same_disk() {
    let root = tempfile::tempdir().unwrap();
    let target = sources::partition_source(&source(), &partition()).unwrap();
    let options = ScanOptions {
        workspace: root.path().join("scans"),
        ..Default::default()
    };
    let first = Store::create(&target, &options, Default::default()).unwrap();
    let mut second = Store::create(&target, &options, Default::default()).unwrap();
    assert!(store::compare_recorded(&first, &second).is_ok());
    for case in 0..5 {
        let mut different = partition();
        match case {
            0 => different.offset_bytes += 512,
            1 => different.length_bytes -= 512,
            2 => different.sector_size = 4096,
            3 => different.sample_sha256 = "b".repeat(64),
            _ => {}
        }
        second.manifest.source.partition = (case != 4).then_some(different);
        assert!(
            store::compare_recorded(&first, &second).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn cancelled_discovery_does_not_create_a_workspace() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("unused");
    let control = Control::default();
    control.cancel();
    let result = discovery::discover(
        source(),
        Options {
            workspace: workspace.clone(),
            table: Table::Intel,
            deeper: false,
            timeout_seconds: 0,
        },
        &control,
    );
    assert!(matches!(result, Err(super::Error::Cancelled)));
    assert!(!workspace.exists());
}

#[test]
#[ignore = "Runs real TestDisk discovery, TSK scanning and recovery on disposable images"]
fn disposable_testdisk_partition_discovery_and_recovery() {
    let source_root = tempfile::tempdir().unwrap();
    let work = tempfile::Builder::new()
        .prefix("odr-discovery-test-")
        .tempdir_in(std::env::var_os("ODR_TEST_STORAGE").expect("separate test storage"))
        .unwrap();
    let filesystem = source_root.path().join("filesystem.img");
    File::create(&filesystem)
        .unwrap()
        .set_len(16 * 1024 * 1024)
        .unwrap();
    assert!(
        Command::new("mkfs.ext2")
            .args(["-q", "-F", "-L", "LostFiles"])
            .arg(&filesystem)
            .status()
            .unwrap()
            .success()
    );
    let original = source_root.path().join("payload.txt");
    let payload = b"Recovered from a discovered partition without changing its table.\n";
    fs::write(&original, payload).unwrap();
    for request in [
        format!("write {} recovered.txt", original.display()),
        "rm recovered.txt".into(),
    ] {
        assert!(
            Command::new("debugfs")
                .args(["-w", "-R", &request])
                .arg(&filesystem)
                .status()
                .unwrap()
                .success()
        );
    }
    let disk = source_root.path().join("disk.img");
    let mut image = File::create(&disk).unwrap();
    image.set_len(32 * 1024 * 1024).unwrap();
    image.seek(SeekFrom::Start(510)).unwrap();
    image.write_all(&[0x55, 0xaa]).unwrap();
    image.seek(SeekFrom::Start(2 * 1024 * 1024)).unwrap();
    image.write_all(&fs::read(&filesystem).unwrap()).unwrap();
    drop(image);
    let before = jobs::hash_file(&mut File::open(&disk).unwrap(), &Control::default()).unwrap();
    let root = sources::inspect(&disk).unwrap();
    let unsafe_workspace = source_root.path().to_path_buf();
    assert!(
        discovery::discover(
            root.clone(),
            Options {
                workspace: unsafe_workspace.clone(),
                table: Table::Intel,
                deeper: false,
                timeout_seconds: 0
            },
            &Control::default(),
        )
        .is_err()
    );
    assert!(!unsafe_workspace.join("partition-searches").exists());
    let workspace = work.path().join("workspace");
    let mut chosen = None;
    for (table, deeper) in [
        (Table::Intel, false),
        (Table::Intel, true),
        (Table::Gpt, false),
        (Table::Auto, false),
    ] {
        let report = discovery::discover(
            root.clone(),
            Options {
                workspace: workspace.clone(),
                table,
                deeper,
                timeout_seconds: 30,
            },
            &Control::default(),
        )
        .unwrap();
        assert!(report.complete, "{:?}", report.error);
        let index = report
            .partitions
            .iter()
            .position(|p| p.offset_bytes == 2 * 1024 * 1024 && p.length_bytes == 16 * 1024 * 1024)
            .expect("lost partition");
        assert!(report.partitions[index].verified);
        assert_eq!(report.partitions[index].filesystem, "ext2");
        assert_eq!(report.partitions[index].label, "LostFiles");
        let loaded = discovery::load(&report.path()).unwrap();
        chosen = Some(loaded.target(index).unwrap());
        assert!(report.directory.join("testdisk.log").exists());
    }
    assert_eq!(
        jobs::hash_file(&mut File::open(&disk).unwrap(), &Control::default()).unwrap(),
        before
    );
    let target = chosen.unwrap();
    assert!(sources::verify(&target).is_ok());
    // A change within the partition is detected even if the outer disk samples
    // are unchanged. Restore the byte before the actual recovery test.
    use std::os::unix::fs::FileExt;
    let writable = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&disk)
        .unwrap();
    let mut byte = [0];
    writable
        .read_exact_at(&mut byte, 2 * 1024 * 1024 + 512)
        .unwrap();
    writable
        .write_all_at(&[byte[0] ^ 1], 2 * 1024 * 1024 + 512)
        .unwrap();
    assert!(sources::verify(&target).is_err());
    writable.write_all_at(&byte, 2 * 1024 * 1024 + 512).unwrap();
    drop(writable);
    let dir = jobs::scan(
        target,
        ScanOptions {
            workspace: workspace.clone(),
            mode: Mode::Quick,
            extensions: vec!["txt".into()],
            ..Default::default()
        },
        &Control::default(),
        &|_| {},
    )
    .unwrap();
    let store = Store::open(&dir).unwrap();
    assert!(store.manifest.complete, "{:?}", store.manifest.errors);
    assert_eq!(
        fs::metadata(dir.join("source-range.img")).unwrap().len(),
        16 * 1024 * 1024
    );
    assert!(store.manifest.source.partition.is_some());
    let page = store
        .page(&Query {
            limit: 250,
            ..Default::default()
        })
        .unwrap();
    let file = page
        .files
        .iter()
        .find(|f| f.path == "recovered.txt")
        .unwrap();
    store.select(&[file.id], true).unwrap();
    drop(store);
    assert_eq!(
        jobs::hash_file(&mut File::open(&disk).unwrap(), &Control::default()).unwrap(),
        before
    );
    fs::rename(&disk, source_root.path().join("disconnected.img")).unwrap();
    let destination = work.path().join("recovered");
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
        fs::read(destination.join("recovered.txt")).unwrap(),
        payload
    );
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
            assert_eq!(
                Store::open(&directory)
                    .unwrap()
                    .page(&Query {
                        limit: 250,
                        ..Default::default()
                    })
                    .unwrap()
                    .total,
                1
            );
            break;
        }
    }
}
