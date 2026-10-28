//! Cancellable worker pipelines shared by the CLI and GTK application.
use super::{
    Error, Result,
    adapters::{self, Adapter, Recoverer, ScanContext, Scanner},
    fail,
    process::{self, Control, Spec},
    safety::{self, GuardedDir},
    sources,
    store::Store,
    timestamp,
    types::*,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
};

pub struct Job {
    pub control: Control,
    pub events: Receiver<Event>,
}
fn send(tx: &SyncSender<Event>, event: Event) {
    match event {
        Event::Finished { .. }
        | Event::Imaged { .. }
        | Event::Recovered { .. }
        | Event::Session(_) => {
            let _ = tx.send(event);
        }
        _ => {
            let _ = tx.try_send(event);
        }
    }
}
pub fn start_scan(source: SourceDescriptor, options: ScanOptions) -> Job {
    let control = Control::default();
    let worker = control.clone();
    let (tx, events) = mpsc::sync_channel(128);
    thread::spawn(move || {
        let notify = |event| send(&tx, event);
        if let Err(error) = scan(source, options, &worker, &notify) {
            notify(Event::Finished {
                directory: PathBuf::new(),
                cancelled: matches!(error, Error::Cancelled),
                error: Some(error.to_string()),
            });
        }
    });
    Job { control, events }
}
pub fn start_recovery(directory: PathBuf, options: RecoveryOptions) -> Job {
    let control = Control::default();
    let worker = control.clone();
    let (tx, events) = mpsc::sync_channel(128);
    thread::spawn(move || {
        let notify = |event| send(&tx, event);
        if let Err(error) = recover(&directory, &options, &worker, &notify) {
            notify(Event::Finished {
                directory,
                cancelled: matches!(error, Error::Cancelled),
                error: Some(error.to_string()),
            });
        }
    });
    Job { control, events }
}

pub fn restart(directory: PathBuf) -> Job {
    let control = Control::default();
    let worker = control.clone();
    let (tx, events) = mpsc::sync_channel(128);
    thread::spawn(move || {
        let notify = |event| send(&tx, event);
        let result = (|| -> Result<PathBuf> {
            let old = Store::open(&directory)?;
            let source = session_source(&old)?;
            let mut options = old.manifest.options.clone();
            let retained = old
                .manifest
                .scan_input
                .as_ref()
                .or(old.manifest.clone.as_ref())
                .map(|path| safety::confined_artifact(&directory, path))
                .transpose()?;
            if retained.is_some() {
                options.clone_first = false;
                if old.manifest.scan_input.as_deref() == Some("source-range.img") {
                    options.offset_bytes = 0;
                    options.length_bytes = None;
                }
            }
            scan_attempt(source, options, &worker, &notify, retained)
        })();
        if let Err(error) = result {
            notify(Event::Finished {
                directory: PathBuf::new(),
                cancelled: matches!(error, Error::Cancelled),
                error: Some(error.to_string()),
            });
        }
    });
    Job { control, events }
}
pub fn start_imaging_resume(directory: PathBuf, tuning: DdrescueOptions, retries: u32) -> Job {
    let control = Control::default();
    let worker = control.clone();
    let (tx, events) = mpsc::sync_channel(128);
    thread::spawn(move || {
        let notify = |event| send(&tx, event);
        if let Err(error) = super::imaging::resume(&directory, tuning, retries, &worker, &notify) {
            notify(Event::Finished {
                directory,
                cancelled: matches!(error, Error::Cancelled),
                error: Some(error.to_string()),
            });
        }
    });
    Job { control, events }
}
pub fn scan(
    source: SourceDescriptor,
    options: ScanOptions,
    control: &Control,
    notify: &dyn Fn(Event),
) -> Result<PathBuf> {
    scan_attempt(source, options, control, notify, None)
}
fn scan_attempt(
    source: SourceDescriptor,
    mut options: ScanOptions,
    control: &Control,
    notify: &dyn Fn(Event),
    retained: Option<PathBuf>,
) -> Result<PathBuf> {
    options.validate()?;
    let source = if retained.is_none() {
        sources::verify(&source)?
    } else {
        source
    };
    if retained.is_none() {
        if let Some(partition) = &source.partition {
            partition.validate(source.size)?;
            if (options.offset_bytes != 0 || options.length_bytes.is_some())
                && (options.offset_bytes != partition.offset_bytes
                    || options.length_bytes != Some(partition.length_bytes))
            {
                return fail(
                    "A discovered partition already defines its byte range. Leave the custom range empty.",
                );
            }
            options.offset_bytes = partition.offset_bytes;
            options.length_bytes = Some(partition.length_bytes);
        }
    }
    if matches!(
        source.filesystem.to_ascii_lowercase().as_str(),
        "zfs" | "zfs_member" | "crypto_luks" | "bitlocker"
    ) {
        return fail(
            "ZFS and locked encrypted volumes are outside the supported source scope. Select an unlocked mapper.",
        );
    }
    if source.scope.is_some() && options.mode != Mode::Quick {
        return fail(
            "Content carving cannot be limited to a mounted folder. Select its whole filesystem for Deep/Full scan, or choose Quick for a folder-scoped metadata scan.",
        );
    }
    if options.offset_bytes >= source.size
        || options.length_bytes.is_some_and(|length| {
            options
                .offset_bytes
                .checked_add(length)
                .is_none_or(|end| end > source.size)
        })
    {
        return fail("The custom byte range is outside the source.");
    }
    notify(Event::Stage {
        text: "Checking source and installed engines".into(),
        fraction: None,
    });
    // Validation precedes creating a log, cache, database or output directory.
    safety::validate_storage(&source, &options.workspace, 0)?;
    let versions = adapters::versions();
    let plan = super::plan::resolve(&source, &options, &versions)?;
    let metadata = plan.metadata.clone();
    let carver = plan.carver.clone();
    let input = if let Some(path) = &retained {
        File::open(path)?
    } else {
        sources::open_readonly(&source)?
    };
    let mut store = Store::create(&source, &options, versions)?;
    super::store::write_json(&store.directory.join("plan.json"), &plan)?;
    notify(Event::Session(store.directory.clone()));
    if source.mounted {
        notify(Event::Message("Live source: other applications can still change the mounted filesystem. A clone of a live filesystem may be inconsistent.".into()));
    }
    let result = (|| -> Result<()> {
        let (mut input, mut input_size) = if let Some(path) = retained {
            let destination = store.directory.join("retained-input.img");
            let size = input.metadata()?.len();
            safety::validate_storage(&source, &store.directory, size)?;
            if fs::hard_link(&path, &destination).is_err() {
                let mut out = new_file(&destination)?;
                adapters::copy_range(&input, &mut out, 0, size, control, notify)?;
            }
            store.manifest.scan_input = Some("retained-input.img".into());
            store.manifest.clone = Some("retained-input.img".into());
            store.save()?;
            (File::open(destination)?, size)
        } else {
            prepare(&mut store, input, control, notify)?
        };
        if options.offset_bytes != 0 || options.length_bytes.is_some() {
            let length = options
                .length_bytes
                .unwrap_or(input_size - options.offset_bytes);
            let range = store.directory.join("source-range.img");
            let mut output = new_file(&range)?;
            adapters::copy_range(
                &input,
                &mut output,
                options.offset_bytes,
                length,
                control,
                notify,
            )?;
            input = File::open(&range)?;
            input_size = length;
            store.manifest.scan_input = Some("source-range.img".into());
            store.save()?;
        }
        store.manifest.state = "scanning".into();
        store.save()?;
        diagnostics(&store, &input, control, notify)?;
        for engine in metadata {
            notify(Event::Stage {
                text: format!("Searching filesystem metadata · {engine}"),
                fraction: None,
            });
            let work = store.directory.join("artifacts/partition-table");
            fs::create_dir_all(&work)?;
            let ranges = if engine == "tsk" && store.manifest.source.scope.is_none() {
                adapters::partitions(&input, &work, plan.sector_size, input_size, control)?
            } else {
                vec![(0, input_size)]
            };
            for (offset, length) in ranges {
                control.checkpoint()?;
                let mut context = ScanContext {
                    store: &mut store,
                    input: &input,
                    control,
                    notify,
                    offset,
                    length,
                    sector_size: plan.sector_size,
                };
                if let Err(error) = Adapter(engine.clone()).scan(&mut context) {
                    if matches!(error, Error::Cancelled) {
                        return Err(error);
                    }
                    notify(Event::Message(error.to_string()));
                    store.manifest.errors.push(error.to_string());
                }
            }
        }
        if let Some(engine) = carver {
            control.checkpoint()?;
            notify(Event::Stage {
                text: format!("Searching file contents · {engine}"),
                fraction: None,
            });
            let mut context = ScanContext {
                store: &mut store,
                input: &input,
                control,
                notify,
                offset: 0,
                length: input_size,
                sector_size: plan.sector_size,
            };
            if let Err(error) = Adapter(engine).scan(&mut context) {
                if matches!(error, Error::Cancelled) {
                    return Err(error);
                }
                notify(Event::Message(error.to_string()));
                store.manifest.errors.push(error.to_string());
            }
        }
        if store.manifest.options.consolidate {
            super::consolidation::run(&mut store, &input, control, notify)?;
        }
        Ok(())
    })();
    let cancelled = matches!(result, Err(Error::Cancelled));
    if let Err(error) = &result {
        store.manifest.errors.push(error.to_string());
    }
    store.manifest.complete = result.is_ok() && store.manifest.errors.is_empty();
    store.manifest.state = if cancelled {
        "cancelled"
    } else if store.manifest.complete {
        "complete"
    } else {
        "partial"
    }
    .into();
    store.checkpoint()?;
    store.save()?;
    store.log("scan_finished",&serde_json::json!({"state":store.manifest.state,"candidates":store.count()?,"errors":store.manifest.errors}))?;
    let error = (!store.manifest.errors.is_empty()).then(|| store.manifest.errors.join("\n"));
    notify(Event::Finished {
        directory: store.directory.clone(),
        cancelled,
        error,
    });
    Ok(store.directory)
}

fn new_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?)
}
fn prepare(
    store: &mut Store,
    mut input: File,
    control: &Control,
    notify: &dyn Fn(Event),
) -> Result<(File, u64)> {
    let options = store.manifest.options.clone();
    let size = store.manifest.source.size;
    if store.manifest.source.ewf {
        let work = store.directory.join("artifacts/ewf");
        fs::create_dir(&work)?;
        let target = store.directory.join("ewf.raw");
        let output = new_file(&target)?;
        notify(Event::Stage {
            text: "Reading E01 segments into the workspace".into(),
            fraction: None,
        });
        let mut spec = Spec::new("ewfexport", &work);
        spec.args(&["-u", "-f", "raw", "-t", "-"])
            .arg(&store.manifest.source.path);
        spec.output = Some(output.try_clone()?);
        spec.timeout = options.timeout_seconds;
        adapters::checked(process::run(spec, control, |_| Ok(()))?, "ewfexport")?;
        output.sync_all()?;
        // Older ewf-tools append this status line to raw stdout. Remove only the
        // exact recognized trailer beyond the independently recorded media size.
        if output.metadata()?.len() > size && output.metadata()?.len() <= size + 128 {
            let mut check = output.try_clone()?;
            check.seek(SeekFrom::Start(size))?;
            let mut trailer = String::new();
            check.read_to_string(&mut trailer)?;
            if trailer.trim() == "ewfexport: SUCCESS" {
                output.set_len(size)?;
                output.sync_all()?;
            }
        }
        if output.metadata()?.len() != size {
            return fail("The E01 export length differs from its recorded media size.");
        }
        input = File::open(&target)?;
        store.manifest.scan_input = Some("ewf.raw".into());
        store.save()?;
    }
    if options.clone_first && options.clone_backend == "ddrescue" {
        store.manifest.clone = Some("clone/clone.img".into());
        store.manifest.clone_map = Some("clone/clone.map".into());
        store.manifest.state = "imaging".into();
        store.save()?;
        let result = super::imaging::acquire(
            &store.directory,
            &store.manifest.source,
            &input,
            &options,
            false,
            control,
            notify,
        );
        store.manifest.imaging =
            super::imaging::read_map(&store.directory.join("clone/clone.map"), size)
                .ok()
                .map(|v| v.0);
        store.save()?;
        let state = result?;
        if !state.complete {
            return fail(
                "Imaging has unread regions. The clone and map are retained; use Saved scans → Resume imaging to continue, or Scan again to explicitly scan the partial image.",
            );
        }
        input = File::open(store.directory.join("clone/clone.img"))?;
        store.manifest.scan_input = store.manifest.clone.clone();
        store.save()?;
    } else if options.clone_first {
        let work = store.directory.join("clone");
        fs::create_dir(&work)?;
        let target = work.join("clone.img");
        let map = work.join("clone.badblocks");
        notify(Event::Stage {
            text: format!("Cloning source · {}", options.clone_backend),
            fraction: None,
        });
        let mut spec = Spec::new(&options.clone_backend, &work);
        spec.source(&input)?;
        spec.timeout = options.timeout_seconds;
        spec.args(&["--stage1", "-L", "0", "-Z", "0", "-R"])
            .arg((options.retry_passes + 1).to_string())
            .arg("-o")
            .arg(&map)
            .arg("/proc/self/fd/3")
            .arg(&target);
        // Retain clone/map paths before running so interrupted clones remain discoverable.
        store.manifest.clone = Some("clone/clone.img".into());
        store.manifest.clone_map = Some(format!(
            "clone/{}",
            map.file_name().unwrap().to_string_lossy()
        ));
        store.save()?;
        let outcome = process::run(spec, control, |line| {
            if line.contains('%') {
                notify(Event::Message(line.chars().take(240).collect()));
            }
            Ok(())
        })?;
        adapters::checked(outcome, &options.clone_backend)?;
        if !target.is_file() {
            return fail("The clone backend produced no image.");
        }
        input = File::open(&target)?;
        input.sync_all()?;
        if input.metadata()?.len() != size {
            return fail(
                "The clone is incomplete; its image and map were retained. Reopen or resume it after reviewing the map.",
            );
        }
        store.manifest.scan_input = store.manifest.clone.clone();
        store.save()?;
    }
    Ok((input, size))
}

fn diagnostics(
    store: &Store,
    input: &File,
    control: &Control,
    notify: &dyn Fn(Event),
) -> Result<()> {
    let fsname = store.manifest.source.filesystem.as_str();
    let commands: Vec<(&str, Vec<&str>)> = if fsname.starts_with("ext") {
        vec![
            ("dumpe2fs", vec!["-h", "/proc/self/fd/3"]),
            ("debugfs", vec!["-R", "stats", "/proc/self/fd/3"]),
        ]
    } else if fsname == "btrfs" {
        vec![("btrfs-find-root", vec!["/proc/self/fd/3"])]
    } else {
        vec![]
    };
    for (program, args) in commands {
        if !store.manifest.engines.contains_key(program) {
            continue;
        }
        let dir = store.directory.join("artifacts").join(program);
        fs::create_dir(&dir)?;
        let mut spec = Spec::new(program, &dir);
        spec.source(input)?;
        spec.args(&args);
        spec.timeout = 30;
        spec.output = Some(new_file(&dir.join("report.txt"))?);
        match process::run(spec, control, |_| Ok(()))
            .and_then(|outcome| adapters::checked(outcome, program))
        {
            Ok(()) => {}
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(error) => notify(Event::Message(format!("Optional diagnostic: {error}"))),
        }
    }
    Ok(())
}

pub fn session_source(store: &Store) -> Result<SourceDescriptor> {
    if store.manifest.source.path.exists() {
        sources::verify(&store.manifest.source)
    } else {
        // A retained image/artifact remains usable when the original is disconnected.
        let mut source = store.manifest.source.clone();
        if let Ok(devices) = sources::discover() {
            for device in devices {
                if device.fingerprint == source.fingerprint {
                    source.ancestry.extend(device.ancestry);
                }
            }
        }
        source.ancestry.sort();
        source.ancestry.dedup();
        Ok(source)
    }
}
pub fn input_for(store: &Store) -> Result<File> {
    if let Some(relative) = &store.manifest.scan_input {
        return Ok(File::open(safety::confined_artifact(
            &store.directory,
            relative,
        )?)?);
    }
    if let Some(relative) = &store.manifest.clone {
        return Ok(File::open(safety::confined_artifact(
            &store.directory,
            relative,
        )?)?);
    }
    sources::open_readonly(&sources::verify(&store.manifest.source)?)
}
pub fn validate_recovery(store: &Store, options: &RecoveryOptions) -> Result<()> {
    let source = session_source(store)?;
    let page = store.page(&Query {
        limit: 1,
        ..Default::default()
    })?;
    if page.selected == 0 {
        return fail("Select at least one file for recovery.");
    }
    safety::validate_storage(
        &source,
        &options.destination,
        if options.restore_all_paths {
            page.selected_path_bytes
        } else {
            page.selected_bytes
        },
    )?;
    GuardedDir::existing(&store.directory)?.revalidate(&source, 0)?;
    // Keep all writes out of retained input files and the session's artifact tree.
    if options.destination.starts_with(&store.directory)
        || store.directory.starts_with(&options.destination)
    {
        return fail("Choose a recovery destination outside the saved scan directory.");
    }
    Ok(())
}

pub fn recover(
    directory: &Path,
    options: &RecoveryOptions,
    control: &Control,
    notify: &dyn Fn(Event),
) -> Result<()> {
    let store = Store::open(directory)?;
    validate_recovery(&store, options)?;
    let source = session_source(&store)?;
    let selected = store.page(&Query {
        limit: 1,
        ..Default::default()
    })?;
    let output_count = if options.restore_all_paths {
        selected.selected_paths
    } else {
        selected.selected
    };
    let output = GuardedDir::create(
        &source,
        &options.destination,
        if options.restore_all_paths {
            selected.selected_path_bytes
        } else {
            selected.selected_bytes
        },
    )?;
    let jobdir = store.directory.join("artifacts").join(format!(
        "recovery-{}-{}",
        timestamp(),
        std::process::id()
    ));
    fs::create_dir_all(&jobdir)?;
    let (mut successful, mut failed, mut skipped, mut after) = (0, 0, 0, 0);
    let mut input = None;
    let mut hardlinks: std::collections::BTreeMap<String, (PathBuf, String)> =
        std::collections::BTreeMap::new();
    loop {
        let candidates = store.selected_page(after)?;
        if candidates.is_empty() {
            break;
        }
        for group in candidates {
            after = group.id;
            let mut after_path: Option<String> = None;
            loop {
                let files = if options.restore_all_paths {
                    store.recovery_paths(&group, after_path.as_deref())?
                } else if after_path.is_none() {
                    vec![group.clone()]
                } else {
                    vec![]
                };
                if files.is_empty() {
                    break;
                }
                for candidate in files {
                    after_path = Some(candidate.path.clone());
                    control.checkpoint()?;
                    output.revalidate(&source, candidate.bytes)?;
                    let mut record = RecoveryRecord {
                        candidate_id: candidate.id,
                        original_path: candidate.path.clone(),
                        output_path: None,
                        bytes: 0,
                        sha256: None,
                        verified: false,
                        error: None,
                        timestamp: timestamp(),
                        metadata_warnings: vec![],
                    };
                    let result = (|| -> Result<bool> {
                        if candidate
                            .linux_metadata
                            .as_ref()
                            .is_some_and(|m| m.symbolic)
                        {
                            if !options.preserve_linux_metadata {
                                record.error=Some("Skipped symbolic link: Linux metadata preservation is disabled.".into());
                                return Ok(false);
                            }
                            let metadata = candidate.linux_metadata.as_ref().unwrap();
                            let target = if let Some(target) = &metadata.link_target {
                                target.clone()
                            } else {
                                if input.is_none() {
                                    input = Some(input_for(&store)?);
                                }
                                let temporary =
                                    jobdir.join(format!("link-{}-{}", candidate.id, timestamp()));
                                let mut file = new_file(&temporary)?;
                                let result = Adapter(candidate.backend.clone()).recover(
                                    input.as_ref().unwrap(),
                                    &candidate,
                                    &file,
                                    &jobdir,
                                    super::options::sector(&source, &store.manifest.options),
                                    &store.manifest.options.interpretation.filesystem,
                                    control,
                                );
                                let text = (|| {
                                    result?;
                                    file.seek(SeekFrom::Start(0))?;
                                    let mut text = String::new();
                                    file.take(4097).read_to_string(&mut text)?;
                                    if text.len() > 4096 {
                                        return fail("Symbolic link target is too long.");
                                    }
                                    Ok(text)
                                })();
                                let _ = fs::remove_file(temporary);
                                text?
                            };
                            record.output_path = output.create_link(
                                &candidate.path,
                                options.preserve_tree,
                                options.conflict,
                                Some(&target),
                                None,
                            )?;
                            if let Some(path) = &record.output_path {
                                record.bytes = target.len() as u64;
                                record.sha256 =
                                    Some(format!("{:x}", Sha256::digest(target.as_bytes())));
                                if options.verify_after_write {
                                    record.verified = fs::read_link(path)?.as_os_str()
                                        == std::ffi::OsStr::new(&target);
                                    if !record.verified {
                                        return fail("Symbolic link verification failed.");
                                    }
                                }
                                return Ok(true);
                            }
                            return Ok(false);
                        }
                        let link_key = if options.preserve_linux_metadata {
                            candidate
                                .linux_metadata
                                .as_ref()
                                .and_then(|m| m.hardlink.clone())
                                .map(|key| format!("{}:{key}", candidate.backend))
                        } else {
                            None
                        };
                        if let Some((first, digest)) =
                            link_key.as_ref().and_then(|k| hardlinks.get(k))
                        {
                            let existing = OpenOptions::new()
                                .read(true)
                                .custom_flags(libc::O_NOFOLLOW)
                                .open(first);
                            if let Ok(mut existing) = existing {
                                if existing.metadata()?.len() == candidate.bytes
                                    && hash_file(&mut existing, control)? == *digest
                                {
                                    match output.create_link(
                                        &candidate.path,
                                        options.preserve_tree,
                                        options.conflict,
                                        None,
                                        Some(&existing),
                                    ) {
                                        Ok(path) => {
                                            record.output_path = path;
                                            record.bytes = candidate.bytes;
                                            record.sha256 = Some(digest.clone());
                                            record.verified = options.verify_after_write;
                                            return Ok(record.output_path.is_some());
                                        }
                                        Err(error) => record.metadata_warnings.push(format!(
                                            "Hard link unavailable; writing a copy: {error}"
                                        )),
                                    }
                                }
                            } else {
                                record.metadata_warnings.push("Hard link unavailable after restoring permissions; writing a copy.".into());
                            }
                        }
                        let Some((mut file, path)) = output.create_file(
                            &candidate.path,
                            options.preserve_tree,
                            options.conflict,
                        )?
                        else {
                            return Ok(false);
                        };
                        record.output_path = Some(path); // Partial files remain auditable on failure/cancellation.
                        if let Some(artifact) = &candidate.artifact {
                            let mut original =
                                File::open(safety::confined_artifact(&store.directory, artifact)?)?;
                            record.bytes = super::metadata::copy(
                                &mut original,
                                &mut file,
                                options.preserve_linux_metadata,
                                control,
                            )?;
                        } else {
                            if input.is_none() {
                                input = Some(input_for(&store)?);
                            }
                            Adapter(candidate.backend.clone()).recover(
                                input.as_ref().unwrap(),
                                &candidate,
                                &file,
                                &jobdir,
                                super::options::sector(&source, &store.manifest.options),
                                &store.manifest.options.interpretation.filesystem,
                                control,
                            )?;
                        }
                        if options.preserve_linux_metadata && candidate.artifact.is_none() {
                            if let Err(error) = super::metadata::sparsify(&mut file, control) {
                                if matches!(error, Error::Cancelled) {
                                    return Err(error);
                                }
                                record
                                    .metadata_warnings
                                    .push(format!("Sparse layout unavailable: {error}"));
                            }
                        }
                        file.sync_all()?;
                        record.bytes = file.metadata()?.len();
                        if record.bytes != candidate.bytes {
                            return fail(format!(
                                "Recovered length {} differs from recorded length {}.",
                                record.bytes, candidate.bytes
                            ));
                        }
                        let digest = hash_file(&mut file, control)?;
                        record.sha256 = Some(digest.clone());
                        if candidate
                            .sha256
                            .as_ref()
                            .is_some_and(|expected| expected != &digest)
                        {
                            return fail(
                                "The saved content no longer matches the consolidated group. The group was left unchanged.",
                            );
                        }
                        if options.verify_after_write {
                            let mut reread = File::open(record.output_path.as_ref().unwrap())?;
                            if hash_file(&mut reread, control)? != digest {
                                return fail(
                                    "SHA-256 verification failed after writing the recovered file.",
                                );
                            }
                            record.verified = true;
                        }
                        if options.preserve_timestamps && candidate.modified > 0 {
                            file.set_times(std::fs::FileTimes::new().set_modified(
                                std::time::UNIX_EPOCH
                                    + std::time::Duration::from_secs(candidate.modified as u64),
                            ))?;
                        }
                        if options.preserve_linux_metadata {
                            if let Some(metadata) = &candidate.linux_metadata {
                                record
                                    .metadata_warnings
                                    .extend(super::metadata::apply(&file, metadata));
                            } else {
                                record.metadata_warnings.push(
                                    "No Linux metadata was supplied by this scan/engine.".into(),
                                );
                            }
                            if let Some(key) = link_key {
                                hardlinks.insert(
                                    key,
                                    (record.output_path.as_ref().unwrap().clone(), digest),
                                );
                            }
                        }
                        Ok(true)
                    })();
                    match &result {
                        Ok(true) => successful += 1,
                        Ok(false) => {
                            skipped += 1;
                            if record.error.is_none() {
                                record.error =
                                    Some("Skipped: a destination filename already exists.".into());
                            }
                        }
                        Err(error) => {
                            failed += 1;
                            record.error = Some(error.to_string());
                            if let Some(path) = &record.output_path {
                                if let Ok(meta) = fs::metadata(path) {
                                    record.bytes = meta.len();
                                }
                            }
                        }
                    }
                    for warning in &record.metadata_warnings {
                        notify(Event::Message(format!("{}: {warning}", candidate.path)));
                    }
                    store.record_recovery(&record)?;
                    notify(Event::Stage {
                        text: format!(
                            "Recovering · {successful} saved · {failed} failed · {skipped} skipped"
                        ),
                        fraction: Some(
                            (successful + failed + skipped) as f64 / output_count.max(1) as f64,
                        ),
                    });
                    if let Err(error) = result {
                        notify(Event::Message(format!("{}: {error}", candidate.path)));
                        if matches!(error, Error::Cancelled) || !options.continue_on_error {
                            return Err(error);
                        }
                    }
                }
            }
        }
    }
    notify(Event::Recovered {
        successful,
        failed,
        skipped,
    });
    Ok(())
}
pub fn hash_file(file: &mut File, control: &Control) -> Result<String> {
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        control.checkpoint()?;
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn preview(directory: &Path, id: i64) -> Result<PathBuf> {
    let store = Store::open(directory)?;
    let candidate = store.get(id)?;
    if candidate
        .linux_metadata
        .as_ref()
        .is_some_and(|m| m.symbolic)
    {
        return fail(
            "Symbolic links are never followed for previews. Inspect their saved metadata instead.",
        );
    }
    if candidate.bytes > 8 * 1024 * 1024 {
        return fail(
            "Preview is limited to files up to 8 MiB. Recover this file to inspect it externally.",
        );
    }
    if let Some(artifact) = &candidate.artifact {
        return safety::confined_artifact(directory, artifact);
    }
    let source = session_source(&store)?;
    GuardedDir::existing(directory)?.revalidate(&source, candidate.bytes)?;
    let dir = directory.join("artifacts/previews");
    fs::create_dir_all(&dir)?;
    let extension = Path::new(&candidate.path)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("bin");
    if !extension.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return fail("Invalid preview filename.");
    }
    let path = dir.join(format!("{}-{id}.{extension}", std::process::id()));
    if path.exists() {
        return safety::confined_artifact(
            directory,
            &format!("artifacts/previews/{}-{id}.{extension}", std::process::id()),
        );
    }
    let file = new_file(&path)?;
    let control = Control::default();
    let result = Adapter(candidate.backend.clone()).recover(
        &input_for(&store)?,
        &candidate,
        &file,
        &dir,
        super::options::sector(&source, &store.manifest.options),
        &store.manifest.options.interpretation.filesystem,
        &control,
    );
    if result.is_err() {
        let _ = fs::remove_file(&path);
    }
    result?;
    Ok(path)
}
