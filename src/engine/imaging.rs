//! Persistent ddrescue acquisition. Map progress, not image length, establishes completion.
use super::{
    Error, Result, adapters, fail,
    process::{self, Control, Spec},
    safety, sources, store, timestamp,
    types::*,
};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

#[derive(Debug)]
pub struct Region {
    pub start: u64,
    pub length: u64,
    pub status: char,
}
pub fn parse_map(text: &str, size: u64) -> Result<(ImagingState, Vec<Region>)> {
    fn number(s: &str) -> Option<u64> {
        s.strip_prefix("0x")
            .map_or_else(|| s.parse().ok(), |n| u64::from_str_radix(n, 16).ok())
    }
    let mut regions = vec![];
    let mut end = 0u64;
    let mut state = ImagingState::default();
    let mut header = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        if !header {
            if fields.len() < 2
                || number(fields[0]).is_none()
                || fields[1].len() != 1
                || !"?*/-+FG".contains(fields[1])
            {
                return fail("Invalid ddrescue map status line.");
            }
            header = true;
            continue;
        }
        if fields.len() != 3 {
            return fail("Invalid ddrescue map region.");
        }
        let start =
            number(fields[0]).ok_or_else(|| Error::Message("Invalid map offset.".into()))?;
        let length =
            number(fields[1]).ok_or_else(|| Error::Message("Invalid map length.".into()))?;
        if start != end || length == 0 || fields[2].len() != 1 || !"?*/-+".contains(fields[2]) {
            return fail(
                "The map must contain contiguous, non-overlapping regions from byte zero.",
            );
        }
        end = start
            .checked_add(length)
            .filter(|n| *n <= size)
            .ok_or_else(|| Error::Message("Map region exceeds source size.".into()))?;
        let status = fields[2].chars().next().unwrap();
        match status {
            '+' => state.rescued += length,
            '-' => state.bad += length,
            _ => state.pending += length,
        }
        regions.push(Region {
            start,
            length,
            status,
        });
    }
    if !header || end != size {
        return fail("The ddrescue map does not cover the selected source size.");
    }
    state.complete = state.rescued == size;
    Ok((state, regions))
}
pub fn read_map(path: &Path, size: u64) -> Result<(ImagingState, Vec<Region>)> {
    parse_map(&map_text(path)?, size)
}
fn map_text(path: &Path) -> Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > 64 * 1024 * 1024 {
        return fail("The map must be a regular file of at most 64 MiB.");
    }
    let mut text = String::new();
    file.take(64 * 1024 * 1024 + 1).read_to_string(&mut text)?;
    Ok(text)
}
pub fn arguments(o: &ScanOptions, sector: u32) -> Vec<String> {
    let mut args = vec![
        "--sector-size".into(),
        sector.to_string(),
        "--retry-passes".into(),
        o.retry_passes.to_string(),
    ];
    if !o.ddrescue.scrape {
        args.push("--no-scrape".into());
    }
    if o.ddrescue.direct {
        args.push("--idirect".into());
    }
    if o.ddrescue.reverse {
        args.push("--reverse".into());
    }
    for (key, value) in [
        ("--min-read-rate", o.ddrescue.min_read_rate),
        ("--max-read-rate", o.ddrescue.max_read_rate),
        ("--timeout", o.ddrescue.read_timeout),
    ] {
        if value > 0 {
            args.extend([key.into(), value.to_string()]);
        }
    }
    if let Some(value) = o.ddrescue.max_read_errors {
        args.extend(["--max-read-errors".into(), value.to_string()]);
    }
    args
}
pub fn acquire(
    directory: &Path,
    source: &SourceDescriptor,
    input: &File,
    o: &ScanOptions,
    resume: bool,
    control: &Control,
    notify: &dyn Fn(Event),
) -> Result<ImagingState> {
    let work = directory.join("clone");
    if !resume {
        fs::create_dir(&work)?;
    }
    let guard = safety::GuardedDir::existing(&work)?;
    guard.revalidate(source, 0)?;
    let target = work.join("clone.img");
    let map = work.join("clone.map");
    if !resume {
        if let (Some(image), Some(import_map)) = (&o.ddrescue.import_image, &o.ddrescue.import_map)
        {
            let imported_map = map_text(import_map)?;
            let (_, regions) = parse_map(&imported_map, source.size)?;
            let imported = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(image)?;
            let stamp = imported.metadata()?;
            if !stamp.is_file() || stamp.len() > source.size {
                return fail("Imported image must be a regular file no larger than the source.");
            }
            let good: Vec<_> = regions.iter().filter(|r| r.status == '+').collect();
            if good.is_empty() {
                return fail(
                    "The imported map has no rescued regions to validate against the source.",
                );
            }
            if good.iter().any(|r| r.start + r.length > stamp.len()) {
                return fail("The map claims rescued bytes beyond the imported image.");
            }
            // Check both ends of up to 32 successful regions, never unread regions.
            for r in good.iter().step_by((good.len() / 32).max(1)).take(32) {
                for offset in [r.start, r.start + r.length - r.length.min(4096)] {
                    control.checkpoint()?;
                    let mut a = vec![0; r.length.min(4096) as usize];
                    let mut b = a.clone();
                    input.read_exact_at(&mut a, offset)?;
                    imported.read_exact_at(&mut b, offset)?;
                    if a != b {
                        return fail(
                            "The imported image/map samples do not match the selected source.",
                        );
                    }
                }
            }
            guard.revalidate(source, source.size)?;
            let mut out = OpenOptions::new()
                .create_new(true)
                .write(true)
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&target)?;
            adapters::copy_range(&imported, &mut out, 0, stamp.len(), control, notify)?;
            let after = imported.metadata()?;
            if (
                stamp.len(),
                stamp.mtime(),
                stamp.mtime_nsec(),
                stamp.ctime(),
                stamp.ctime_nsec(),
            ) != (
                after.len(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec(),
            ) {
                return fail("The imported image changed while it was being copied.");
            }
            use std::io::Write;
            let mut out = OpenOptions::new()
                .create_new(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&map)?;
            out.write_all(imported_map.as_bytes())?;
            out.sync_all()?;
        }
    }
    if resume {
        read_map(&map, source.size)?;
        for path in [&target, &map] {
            let meta = fs::symlink_metadata(path)?;
            if !meta.is_file() || meta.nlink() != 1 {
                return fail(
                    "Resuming requires private regular clone/map files, not links shared with other scans.",
                );
            }
        }
    }
    let mut spec = Spec::new("ddrescue", &work);
    spec.source(input)?;
    spec.timeout = o.timeout_seconds;
    for arg in arguments(o, super::options::sector(source, o)) {
        spec.arg(arg);
    }
    spec.arg("--size")
        .arg(source.size.to_string())
        .arg("/proc/self/fd/3")
        .arg(&target)
        .arg(&map);
    notify(Event::Stage {
        text: if resume {
            "Resuming ddrescue imaging"
        } else {
            "Imaging source with ddrescue"
        }
        .into(),
        fraction: None,
    });
    let result = process::run(spec, control, |line| {
        if line.contains('%') {
            notify(Event::Message(line.chars().take(240).collect()));
        }
        Ok(())
    });
    guard.check_identity()?;
    if target.exists() {
        File::open(&target)?.sync_all()?;
    }
    let parsed = read_map(&map, source.size);
    if let Ok((state, _)) = &parsed {
        notify(Event::Message(format!(
            "Image: {} rescued bytes · {} pending · {} bad",
            state.rescued, state.pending, state.bad
        )));
    }
    adapters::checked(result?, "ddrescue")?;
    let (state, _) = parsed?;
    if state.complete && fs::metadata(&target)?.len() != source.size {
        return fail("Completed map and image length disagree.");
    }
    Ok(state)
}
pub fn can_resume(m: &Manifest) -> bool {
    m.options.clone_backend == "ddrescue"
        && m.clone.is_some()
        && m.clone_map.is_some()
        && m.candidate_count == 0
        && m.scan_input.as_deref() != m.clone.as_deref()
        && !m.imaging.as_ref().is_some_and(|s| s.complete)
        && matches!(
            m.state.as_str(),
            "partial" | "cancelled" | "imaging" | "preparing"
        )
}
pub fn resume(
    directory: &Path,
    tuning: DdrescueOptions,
    retries: u32,
    control: &Control,
    notify: &dyn Fn(Event),
) -> Result<()> {
    let _lease = super::sessions::Lease::acquire(directory, true)?;
    let mut m: Manifest =
        super::read_json(&safety::confined_artifact(directory, "manifest.json")?)?;
    if !can_resume(&m) {
        return fail(
            "Only an interrupted, unscanned ddrescue acquisition can resume. Existing scan inputs remain immutable.",
        );
    }
    let source = sources::verify(&m.source)?;
    safety::GuardedDir::existing(directory)?.revalidate(&source, 0)?;
    let input = if source.ewf {
        File::open(safety::confined_artifact(directory, "ewf.raw")?)?
    } else {
        sources::open_readonly(&source)?
    };
    m.options.ddrescue = tuning;
    m.options.retry_passes = retries;
    m.options.validate()?;
    m.options.ddrescue.import_image = None;
    m.options.ddrescue.import_map = None;
    m.state = "imaging".into();
    m.updated_at = timestamp();
    store::write_json(&directory.join("manifest.json"), &m)?;
    notify(Event::Session(directory.into()));
    let result = acquire(
        directory, &source, &input, &m.options, true, control, notify,
    );
    m.imaging = read_map(&directory.join("clone/clone.map"), source.size)
        .ok()
        .map(|p| p.0);
    m.state = if matches!(result, Err(Error::Cancelled)) {
        "cancelled"
    } else if m.imaging.as_ref().is_some_and(|s| s.complete) {
        m.scan_input = m.clone.clone();
        "imaged"
    } else {
        "partial"
    }
    .into();
    m.updated_at = timestamp();
    if let Err(e) = &result {
        m.errors.push(e.to_string());
    }
    store::write_json(&directory.join("manifest.json"), &m)?;
    if result.is_ok() {
        notify(Event::Imaged {
            directory: directory.into(),
            progress: m.imaging.clone().unwrap_or_default(),
        });
    }
    result.map(|_| ())
}
