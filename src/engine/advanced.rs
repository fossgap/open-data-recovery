use super::{
    Result,
    adapters::{self, ScanContext},
    fail, options,
    process::{self, Control, Spec},
    types::*,
};
use std::{fs, path::Path};

pub fn ext_arguments(
    spec: &mut Spec,
    engine: &str,
    o: &ScanOptions,
    directory: &Path,
) -> Result<()> {
    let e = &o.ext;
    if let Some(journal) = &e.journal {
        let input = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(journal)?;
        if !input.metadata()?.is_file() {
            return fail("Choose a regular saved journal file.");
        }
        let path = directory.join("saved-journal.bin");
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        std::io::copy(&mut &input, &mut out)?;
        out.sync_all()?;
        spec.arg("-j").arg(path);
    }
    if engine == "extundelete" {
        for (flag, value) in [
            ("--after", e.after),
            ("--before", e.before),
            ("-b", e.superblock),
            ("-B", e.block_size.map(u64::from)),
        ] {
            if let Some(value) = value {
                spec.arg(flag).arg(value.to_string());
            }
        }
        match e.target_kind.as_str() {
            "file" => {
                spec.arg("--restore-file").arg(&e.target);
            }
            "directory" => {
                spec.arg("--restore-directory").arg(&e.target);
            }
            "inode" => {
                spec.arg("--restore-inode").arg(&e.target);
            }
            _ => {
                spec.arg("--restore-all");
            }
        }
    } else {
        let targeted = e.target_kind != "all"
            || e.after.is_some()
            || e.before.is_some()
            || e.transaction.is_some();
        spec.arg(if targeted {
            if o.include_existing { "-R" } else { "-r" }
        } else {
            "-M"
        });
        match e.target_kind.as_str() {
            "file" | "directory" => {
                spec.arg("-f").arg(&e.target);
            }
            "inode" => {
                spec.arg("-I").arg(&e.target);
            }
            _ => {}
        }
        for (flag, value) in [("-a", e.after), ("-b", e.before), ("-t", e.transaction)] {
            if let Some(value) = value {
                spec.arg(flag).arg(value.to_string());
            }
        }
    }
    Ok(())
}
use std::os::unix::fs::OpenOptionsExt;
pub fn btrfs_arguments(spec: &mut Spec, o: &ScanOptions) {
    spec.arg("restore");
    if o.btrfs.ignore_errors {
        spec.arg("-i");
    }
    if o.btrfs.snapshots {
        spec.arg("-s");
    }
    if o.capture_metadata {
        spec.args(&["-m", "-x", "-S"]);
    }
    for (flag, value) in [
        ("-t", o.btrfs_root),
        ("-r", o.btrfs.subvolume),
        ("-f", o.btrfs.subvolume_root),
    ] {
        if let Some(value) = value {
            spec.arg(flag).arg(value.to_string());
        }
    }
    spec.arg("-u").arg(o.btrfs.superblock.to_string());
    if let Some(regex) = options::btrfs_regex(&o.btrfs.path) {
        spec.arg("--path-regex").arg(regex);
    }
}
pub fn ntfs_record(line: &str) -> Option<(u64, String)> {
    // ntfsundelete's C-locale table has six whitespace-separated metadata columns.
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() < 7 || !fields[2].ends_with('%') {
        return None;
    }
    let inode = fields[0].parse().ok()?;
    // Locate the filename after Size without collapsing spaces inside the name.
    let mut rest = line.trim_start();
    for _ in 0..6 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        rest = &rest[end..];
    }
    Some((inode, rest.trim_start().to_string()))
}
fn selected_inode(id: u64, selection: &str) -> bool {
    selection.is_empty()
        || selection.split(',').any(|part| {
            if let Some((a, b)) = part.split_once('-') {
                a.parse::<u64>().is_ok_and(|a| id >= a) && b.parse::<u64>().is_ok_and(|b| id <= b)
            } else {
                part.parse::<u64>() == Ok(id)
            }
        })
}
pub fn ntfs_extract(c: &mut ScanContext<'_>, dir: &Path, output: &Path) -> Result<()> {
    let o = c.store.manifest.options.clone();
    fs::create_dir_all(output)?;
    let mut scan = Spec::new("ntfsundelete", dir);
    scan.source(c.input)?;
    scan.timeout = o.timeout_seconds;
    scan.args(&["--scan", "--percentage"])
        .arg(o.ntfs.minimum_percent.to_string());
    if !o.ntfs.pattern.is_empty() {
        scan.arg("--match").arg(&o.ntfs.pattern);
        if o.ntfs.case_sensitive {
            scan.arg("--case");
        }
    }
    if o.minimum_size > 0 || o.maximum_size.is_some() {
        scan.arg("--size").arg(format!(
            "{}-{}",
            o.minimum_size,
            o.maximum_size.unwrap_or(i64::MAX as u64)
        ));
    }
    scan.arg("/proc/self/fd/3");
    let mut ids = vec![];
    let mut count = 0;
    let mut reported = None;
    let listing = process::run(scan, c.control, |line| {
        adapters::append_output(dir, line)?;
        if let Some(value) = line
            .trim()
            .strip_prefix("Files with potentially recoverable content:")
        {
            reported = value.trim().parse::<u64>().ok();
        }
        if let Some((id, name)) = ntfs_record(line) {
            count += 1;
            let ext = Path::new(&name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if selected_inode(id, &o.ntfs.inodes)
                && (o.extensions.is_empty() || o.extensions.contains(&ext))
            {
                ids.push(id);
            }
        }
        Ok(())
    })?;
    if !(listing.status.code() == Some(1)
        && reported == Some(0)
        && listing.stderr.trim().is_empty())
    {
        adapters::checked(listing, "ntfsundelete listing")?;
    }
    if reported.is_some_and(|n| n > 0) && count == 0 {
        return fail("Unrecognized NTFS listing; no extraction was attempted.");
    }
    // Never fall back to undelete-all on an unrecognized table.
    if count == 0 {
        (c.notify)(Event::Message("NTFS listing contained no matching recoverable records; listing retained in engine.log.".into()));
    }
    ids.sort_unstable();
    ids.dedup();
    for chunk in ids.chunks(128) {
        let mut spec = Spec::new("ntfsundelete", dir);
        spec.source(c.input)?;
        spec.timeout = o.timeout_seconds;
        spec.args(&["--undelete", "--truncate", "--inodes"])
            .arg(
                chunk
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            )
            .arg("--destination")
            .arg(output)
            .arg("/proc/self/fd/3");
        adapters::checked(
            process::run(spec, c.control, |line| adapters::append_output(dir, line))?,
            "ntfsundelete extraction",
        )?;
    }
    Ok(())
}
pub fn inspect_btrfs(source: &SourceDescriptor, o: &ScanOptions, roots: bool) -> Result<String> {
    let source = super::sources::verify(source)?;
    let versions = adapters::versions();
    super::plan::resolve(&source, o, &versions)?;
    if options::filesystem(&source, o) != "btrfs" {
        return fail("Select a Btrfs filesystem first.");
    }
    if source.partition.is_some() || o.offset_bytes != 0 || o.length_bytes.is_some() {
        return fail(
            "Inspect roots on a partition device or filesystem image; a custom range is retained only when scanning.",
        );
    }
    let input = super::sources::open_readonly(&source)?;
    // A persisted inspection session keeps engine logs under the approved workspace.
    let mut inspection = o.clone();
    inspection.clone_first = false;
    inspection.consolidate = false;
    let mut store = super::store::Store::create(&source, &inspection, versions)?;
    let dir = store.directory.join("artifacts/btrfs-inspection");
    fs::create_dir(&dir)?;
    let mut spec = Spec::new("btrfs", &dir);
    spec.source(&input)?;
    spec.timeout = if o.timeout_seconds == 0 {
        60
    } else {
        o.timeout_seconds
    };
    if roots {
        spec.args(&["restore", "-l", "-u"])
            .arg(o.btrfs.superblock.to_string());
        if let Some(root) = o.btrfs_root {
            spec.arg("-t").arg(root.to_string());
        }
    } else {
        let mut preview = o.clone();
        preview.capture_metadata = false;
        btrfs_arguments(&mut spec, &preview);
        fs::create_dir_all(dir.join("files"))?;
        spec.args(&["-D", "-vv"]);
    }
    spec.arg("/proc/self/fd/3");
    if !roots {
        spec.arg(dir.join("files"));
    }
    let mut report = String::new();
    adapters::checked(
        process::run(spec, &Control::default(), |line| {
            adapters::append_output(&dir, line)?;
            if report.len() < 128 * 1024 {
                report.push_str(line);
                report.push('\n');
            }
            Ok(())
        })?,
        "Btrfs inspection",
    )?;
    store.manifest.state = "inspection".into();
    store.manifest.complete = true;
    store.save()?;
    Ok(format!("{report}\nReport saved in {}", dir.display()))
}
