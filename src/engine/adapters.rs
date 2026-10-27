//! Typed, shell-free adapters. Tools can write only in their own artifact directory.
use super::{
    Error, Result, fail,
    process::{self, Control, Spec},
    store::Store,
    types::*,
};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

pub const METADATA: &[&str] = &[
    "auto",
    "tsk",
    "tsk_recover",
    "ext4magic",
    "extundelete",
    "ntfsundelete",
    "recuperabit",
    "btrfs",
    "testdisk",
    "ext4rescue",
];
pub const CARVERS: &[&str] = &[
    "auto",
    "photorec",
    "scalpel",
    "foremost",
    "digler",
    "magicrescue",
    "recoverjpeg",
    "bulk_extractor",
];
const TOOLS: &[(&str, &[&str])] = &[
    ("fls", &["-V"]),
    ("icat", &["-V"]),
    ("mmls", &["-V"]),
    ("tsk_recover", &["-V"]),
    ("ext4magic", &["-V"]),
    ("extundelete", &["--version"]),
    ("ntfsundelete", &["--version"]),
    ("recuperabit", &["--help"]),
    ("btrfs", &["--version"]),
    ("btrfs-find-root", &["--help"]),
    ("testdisk", &["/version"]),
    ("debugfs", &["-V"]),
    ("dumpe2fs", &["-V"]),
    ("ext4rescue", &["--help"]),
    ("photorec", &["/version"]),
    ("scalpel", &["-V"]),
    ("foremost", &["-V"]),
    ("digler", &["--version"]),
    ("magicrescue", &["-h"]),
    ("recoverjpeg", &["-V"]),
    ("bulk_extractor", &["-V"]),
    ("ddrescue", &["--version"]),
    ("safecopy", &["--help"]),
    ("ewfexport", &["-V"]),
];
pub fn versions() -> BTreeMap<String, String> {
    TOOLS
        .iter()
        .filter_map(|(name, args)| {
            process::probe(name, args).map(|version| ((*name).into(), version))
        })
        .collect()
}
pub fn metadata_engine(
    source: &SourceDescriptor,
    options: &ScanOptions,
    versions: &BTreeMap<String, String>,
) -> Result<String> {
    if !METADATA.contains(&options.metadata_backend.as_str()) {
        return fail("Unknown metadata engine.");
    }
    if options.metadata_backend != "auto" {
        return available(&options.metadata_backend, versions);
    }
    match source.filesystem.to_ascii_lowercase().as_str() {
        "btrfs" => available("btrfs", versions),
        "xfs" | "apfs" | "hfs" | "hfsplus" => {
            fail("This filesystem supports content carving only; choose Deep scan.")
        }
        "zfs_member" | "zfs" | "crypto_luks" | "bitlocker" => fail(
            "Choose an already-unlocked supported filesystem; encrypted and ZFS sources are not supported.",
        ),
        _ => available("tsk", versions),
    }
}
pub fn carving_engine(
    options: &ScanOptions,
    versions: &BTreeMap<String, String>,
) -> Result<String> {
    if !CARVERS.contains(&options.carver.as_str()) {
        return fail("Unknown carving engine.");
    }
    if options.carver != "auto" {
        return available(&options.carver, versions);
    }
    for name in ["photorec", "scalpel", "foremost"] {
        if versions.contains_key(name) {
            return Ok(name.into());
        }
    }
    fail("Install PhotoRec (testdisk), Scalpel or Foremost to run a Deep scan.")
}
fn available(name: &str, versions: &BTreeMap<String, String>) -> Result<String> {
    let executable = if name == "tsk" { "fls" } else { name };
    if !versions.contains_key(executable) {
        return fail(format!(
            "The {executable} backend is not installed. See Help → Backend availability."
        ));
    }
    if name == "tsk" && !versions.contains_key("icat") {
        return fail("Install icat from sleuthkit for metadata recovery.");
    }
    Ok(name.into())
}

pub struct ScanContext<'a> {
    pub store: &'a mut Store,
    pub input: &'a File,
    pub control: &'a Control,
    pub notify: &'a dyn Fn(Event),
    pub offset: u64,
    pub length: u64,
    pub sector_size: u32,
}
pub trait Scanner {
    fn scan(&self, context: &mut ScanContext<'_>) -> Result<()>;
}
pub trait Recoverer {
    fn recover(
        &self,
        input: &File,
        candidate: &Candidate,
        output: &File,
        work: &Path,
        sector: u32,
        control: &Control,
    ) -> Result<()>;
}
pub struct Adapter(pub String);
impl Adapter {
    fn directory(&self, store: &Store, offset: u64) -> Result<PathBuf> {
        let dir = store
            .directory
            .join("artifacts")
            .join(format!("{}-{offset}", self.0));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

pub fn checked(outcome: process::Outcome, tool: &str) -> Result<()> {
    if outcome.status.success() {
        Ok(())
    } else {
        fail(format!(
            "{tool} exited with {}: {}",
            outcome.status,
            outcome.stderr.trim()
        ))
    }
}
pub fn parse_body(
    line: &str,
    offset: u64,
    length: u64,
    version: &str,
) -> Result<Option<Candidate>> {
    // Split from the right: a legal filename can itself contain '|'. NTFS addresses
    // such as 42-128-3 remain strings rather than being truncated to inode numbers.
    let fields: Vec<_> = line.rsplitn(10, '|').collect();
    if fields.len() != 10 {
        return fail("Unrecognized fls body-file output.");
    }
    let Some((_, name)) = fields[9].split_once('|') else {
        return fail("Missing filename in fls output.");
    };
    let mode = fields[7];
    if mode.contains('d') || mode.starts_with('l') || mode.contains("/l") || fields[8] == "0" {
        return Ok(None);
    }
    let deleted = name.ends_with(" (deleted)") || name.contains(" (deleted-realloc)");
    let name = name
        .trim_end_matches(" (deleted)")
        .trim_end_matches(" (deleted-realloc)");
    if name.is_empty() {
        return Ok(None);
    }
    let bytes = fields[4]
        .parse::<u64>()
        .map_err(|_| Error::Message("Invalid file size from fls.".into()))?;
    Ok(Some(Candidate {
        path: name.trim_start_matches('/').into(),
        bytes,
        modified: fields[2].parse().unwrap_or(0),
        backend: "tsk".into(),
        backend_version: version.into(),
        locator: fields[8].into(),
        offset_bytes: offset,
        length_bytes: length,
        deleted,
        ..Default::default()
    }))
}

impl Scanner for Adapter {
    fn scan(&self, c: &mut ScanContext<'_>) -> Result<()> {
        let dir = self.directory(c.store, c.offset)?;
        let options = c.store.manifest.options.clone();
        let version = c
            .store
            .manifest
            .engines
            .get(if self.0 == "tsk" { "fls" } else { &self.0 })
            .cloned()
            .unwrap_or_default();
        let mut spec = Spec::new(if self.0 == "tsk" { "fls" } else { &self.0 }, &dir);
        spec.source(c.input)?;
        spec.timeout = options.timeout_seconds;
        if self.0 == "tsk" {
            spec.args(&["-r", "-p", "-m", "/", "-b"])
                .arg(c.sector_size.to_string())
                .arg("-o")
                .arg((c.offset / c.sector_size as u64).to_string());
            if !options.include_existing {
                spec.arg("-d");
            }
            spec.arg("/proc/self/fd/3");
            let scope = c.store.manifest.source.scope.clone();
            let mut inserted = 0;
            c.store.begin_batch()?;
            let result = process::run(spec, c.control, |line| {
                if let Some(candidate) = parse_body(line, c.offset, c.length, &version)? {
                    if options.accepts(&candidate.path, candidate.bytes)
                        && scope.as_ref().is_none_or(|scope| {
                            Path::new(&format!("/{}", candidate.path)).starts_with(scope)
                        })
                    {
                        c.store.insert(&candidate)?;
                        inserted += 1;
                        if inserted % 250 == 0 {
                            c.store.commit_batch()?;
                            (c.notify)(Event::Candidates(c.store.count()?));
                            c.store.begin_batch()?;
                        }
                    }
                }
                Ok(())
            });
            c.store.commit_batch()?;
            (c.notify)(Event::Candidates(c.store.count()?));
            return checked(result?, "fls");
        }
        if c.offset != 0 {
            return fail(format!(
                "{} requires a partition source or a custom byte range; select that partition before using this engine.",
                self.0
            ));
        }
        let output = dir.join("files");
        // Foremost and Scalpel require a nonexistent/empty output directory; all
        // destinations are generated beneath the approved backend directory.
        let source = "/proc/self/fd/3";
        match self.0.as_str() {
            "tsk_recover" => {
                spec.args(&["-b"]).arg(c.sector_size.to_string());
                if options.include_existing {
                    spec.arg("-e");
                }
                spec.arg(source).arg(&output);
            }
            "ext4magic" => {
                fs::create_dir_all(&output)?;
                spec.arg(source).args(&["-M", "-d"]).arg(&output);
            }
            "extundelete" => {
                spec.arg(source).args(&["--restore-all", "-o"]).arg(&output);
            }
            "ntfsundelete" => {
                fs::create_dir_all(&output)?;
                spec.arg(source)
                    .args(&["--undelete", "--match", "*", "--truncate", "--destination"])
                    .arg(&output);
            }
            "btrfs" => {
                fs::create_dir_all(&output)?;
                spec.args(&["restore", "-i", "-s"]);
                if let Some(root) = options.btrfs_root {
                    spec.arg("-t").arg(root.to_string());
                }
                spec.arg(source).arg(&output);
            }
            "testdisk" => {
                if !c
                    .store
                    .manifest
                    .source
                    .filesystem
                    .eq_ignore_ascii_case("ntfs")
                {
                    return fail(
                        "TestDisk scripted undelete requires an NTFS partition. Use TSK for FAT/exFAT or whole disks.",
                    );
                }
                spec.args(&[
                    "/log",
                    "/cmd",
                    source,
                    "partition_none,advanced,undelete,allundelete",
                ]);
            }
            "ext4rescue" => {
                if options.offset_bytes != 0 || options.length_bytes.is_some() {
                    return fail(
                        "ext4rescue requires a complete filesystem clone and its matching ddrescue map. Select a partition source instead of a custom byte range.",
                    );
                }
                let clone = c.store.manifest.clone.as_ref().ok_or_else(|| {
                    Error::Message("ext4rescue requires a retained ddrescue clone and map.".into())
                })?;
                let map = c
                    .store
                    .manifest
                    .clone_map
                    .as_ref()
                    .ok_or_else(|| Error::Message("The clone has no ddrescue map.".into()))?;
                if options.clone_backend != "ddrescue" {
                    return fail("ext4rescue requires a ddrescue map, not safecopy badblocks.");
                }
                // Old ext4rescue releases mistake modern ddrescue's three-field
                // current-position header for a range. Preserve the original map;
                // supply a separate, validated range-only view to this adapter.
                let original_map = fs::read_to_string(c.store.directory.join(map))?;
                let mut ranges = String::new();
                let mut end = 0_u64;
                for line in original_map.lines() {
                    let fields: Vec<_> = line.split_whitespace().collect();
                    if fields.len() != 3
                        || !fields[0].starts_with("0x")
                        || !fields[1].starts_with("0x")
                    {
                        continue;
                    }
                    let start = u64::from_str_radix(&fields[0][2..], 16)
                        .map_err(|_| Error::Message("Invalid ddrescue map offset.".into()))?;
                    let length = u64::from_str_radix(&fields[1][2..], 16)
                        .map_err(|_| Error::Message("Invalid ddrescue map length.".into()))?;
                    if start != end
                        || length == 0
                        || !["+", "-", "?", "*", "/"].contains(&fields[2])
                    {
                        return fail(
                            "The retained ddrescue map has invalid or noncontiguous ranges.",
                        );
                    }
                    end = start
                        .checked_add(length)
                        .ok_or_else(|| Error::Message("ddrescue map address overflow.".into()))?;
                    ranges.push_str(line);
                    ranges.push('\n');
                }
                if end != c.store.manifest.source.size {
                    return fail("The ddrescue map does not cover the recorded filesystem clone.");
                }
                let compatible_map = dir.join("compatible.map");
                fs::write(&compatible_map, ranges)?;
                spec.arg("--to")
                    .arg(&output)
                    .arg(c.store.directory.join(clone))
                    .arg(compatible_map);
            }
            "photorec" => {
                let mut commands = String::from("partition_none,options,");
                if options.low_memory {
                    commands.push_str("lowmem,");
                }
                commands.push_str(if options.keep_corrupted {
                    "keep_corrupted_file,"
                } else {
                    "keep_corrupted_file_no,"
                });
                commands.push_str("fileopt,everything,enable,wholespace,search");
                // File families do not map one-to-one to suffixes (e.g. docx/zip).
                // Scan all families, then apply the exact extension filter on import.
                spec.arg("/d")
                    .arg(&output)
                    .args(&["/cmd", source, &commands]);
            }
            "scalpel" => {
                let config = dir.join("scalpel.conf");
                let original = ["/etc/scalpel/scalpel.conf", "/etc/scalpel.conf"]
                    .into_iter()
                    .find_map(|p| fs::read_to_string(p).ok())
                    .ok_or_else(|| {
                        Error::Message("Install Scalpel's signature configuration.".into())
                    })?;
                let mut enabled = String::new();
                for line in original.lines() {
                    let candidate = line.trim_start().trim_start_matches('#').trim_start();
                    let fields: Vec<_> = candidate.split_whitespace().collect();
                    if fields.len() >= 4
                        && matches!(fields[1], "y" | "n")
                        && fields[2].parse::<u64>().is_ok()
                    {
                        if fields[0] == "png" {
                            continue;
                        } // Old distro examples use an invalid PNG footer.
                        enabled.push_str(candidate);
                        enabled.push('\n');
                    }
                }
                enabled.push_str("png y 67108864 \\x89\\x50\\x4e\\x47\\x0d\\x0a\\x1a\\x0a \\x49\\x45\\x4e\\x44\\xae\\x42\\x60\\x82\n");
                if enabled.is_empty() {
                    return fail("No supported Scalpel signatures were found.");
                }
                fs::write(&config, enabled)?;
                spec.arg("-c")
                    .arg(&config)
                    .arg("-o")
                    .arg(&output)
                    .arg(source);
            }
            "foremost" => {
                spec.args(&["-t", "all", "-i", source, "-o"]).arg(&output);
            }
            "digler" => {
                spec.args(&["scan", source, "--dump"]).arg(&output);
            }
            "magicrescue" => {
                fs::create_dir_all(&output)?;
                spec.args(&["-r", "/usr/share/magicrescue/recipes", "-d"])
                    .arg(&output)
                    .arg(source);
            }
            "recoverjpeg" => {
                fs::create_dir_all(&output)?;
                spec.arg("-o")
                    .arg(&output)
                    .arg("-s")
                    .arg(options.minimum_size.to_string());
                if let Some(max) = options.maximum_size {
                    spec.arg("-m").arg(max.to_string());
                }
                spec.arg(source);
            }
            "bulk_extractor" => {
                spec.arg("-o").arg(&output).arg(source);
            }
            "recuperabit" => {
                fs::create_dir_all(&output)?;
                let cache = dir.join("reconstruction.save");
                spec.arg(source)
                    .arg("-s")
                    .arg(&cache)
                    .arg("-o")
                    .arg(&output);
                spec.stdin = Some(b"\nrecoverable\nexit\n".to_vec());
                let mut partitions = std::collections::BTreeSet::new();
                let result = process::run(spec, c.control, |line| {
                    // RecuperaBit's recoverable table begins with a numeric partition ID.
                    if let Some(id) = line
                        .split("Partition #")
                        .nth(1)
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse::<u32>().ok())
                    {
                        if partitions.len() < 256 {
                            partitions.insert(id);
                        }
                    }
                    append_output(&dir, line)
                })?;
                checked(result, "recuperabit discovery")?;
                if partitions.is_empty() {
                    return fail(
                        "RecuperaBit reported no reconstructable partition IDs; inspect its saved output.",
                    );
                }
                let mut restore = Spec::new("recuperabit", &dir);
                restore.source(c.input)?;
                restore.timeout = options.timeout_seconds;
                restore
                    .arg(source)
                    .arg("-s")
                    .arg(cache)
                    .arg("-o")
                    .arg(&output);
                let mut commands = String::from("\n");
                for partition in partitions {
                    commands.push_str(&format!("restore {partition} 5\nrestore {partition} -1\n"));
                }
                commands.push_str("exit\n");
                restore.stdin = Some(commands.into_bytes());
                spec = restore;
            }
            _ => return fail("Unsupported backend adapter."),
        }
        let result = process::run(spec, c.control, |line| append_output(&dir, line));
        // Import partial outputs even when an engine exits with an error/cancellation.
        if self.0 == "bulk_extractor" {
            (c.notify)(Event::Message("bulk_extractor feature reports are retained as artifacts; reports are not presented as recovered original files.".into()));
        } else {
            let root = if matches!(self.0.as_str(), "testdisk" | "photorec") {
                &dir
            } else {
                &output
            };
            if root.exists() {
                import_artifacts(
                    c,
                    root,
                    &self.0,
                    &version,
                    CARVERS.contains(&self.0.as_str()),
                )?;
            }
        }
        checked(result?, &self.0)
    }
}

fn append_output(dir: &Path, line: &str) -> Result<()> {
    use std::fs::OpenOptions;
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("engine.log"))?;
    // Per-backend diagnostics are bounded; the structured event log records failures.
    if log.metadata()?.len() < 16 * 1024 * 1024 {
        writeln!(log, "{line}")?;
    }
    Ok(())
}
fn import_artifacts(
    c: &mut ScanContext<'_>,
    root: &Path,
    backend: &str,
    version: &str,
    carved: bool,
) -> Result<()> {
    fn visit(
        c: &mut ScanContext<'_>,
        root: &Path,
        dir: &Path,
        backend: &str,
        version: &str,
        carved: bool,
        depth: usize,
    ) -> Result<()> {
        if depth > 128 {
            return fail("Backend artifact nesting exceeds 128 levels.");
        }
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                visit(c, root, &entry.path(), backend, version, carved, depth + 1)?;
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let path = entry.path();
            let name = path
                .strip_prefix(root)
                .map_err(|_| Error::Message("Artifact path escaped output.".into()))?
                .to_string_lossy()
                .into_owned();
            if matches!(
                entry.file_name().to_str(),
                Some(
                    "engine.log"
                        | "testdisk.log"
                        | "photorec.log"
                        | "report.xml"
                        | "audit.txt"
                        | "scalpel.conf"
                        | "reconstruction.save"
                )
            ) {
                continue;
            }
            let meta = entry.metadata()?;
            if !c.store.manifest.options.accepts(&name, meta.len()) {
                continue;
            }
            if c.store
                .manifest
                .source
                .scope
                .as_ref()
                .is_some_and(|scope| !Path::new(&format!("/{name}")).starts_with(scope))
                && !carved
            {
                continue;
            }
            let artifact = path
                .strip_prefix(&c.store.directory)
                .map_err(|_| Error::Message("Invalid artifact location.".into()))?
                .to_string_lossy()
                .into_owned();
            c.store.insert(&Candidate {
                path: format!("{backend}/{name}"),
                bytes: meta.len(),
                modified: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|t| t.as_secs() as i64)
                    .unwrap_or(0),
                backend: backend.into(),
                backend_version: version.into(),
                locator: artifact.clone(),
                offset_bytes: c.offset,
                length_bytes: c.length,
                artifact: Some(artifact),
                carved,
                deleted: !carved,
                ..Default::default()
            })?;
        }
        Ok(())
    }
    c.store.begin_batch()?;
    let result = visit(c, root, root, backend, version, carved, 0);
    c.store.commit_batch()?;
    result?;
    (c.notify)(Event::Candidates(c.store.count()?));
    Ok(())
}

impl Recoverer for Adapter {
    fn recover(
        &self,
        input: &File,
        candidate: &Candidate,
        output: &File,
        work: &Path,
        sector: u32,
        control: &Control,
    ) -> Result<()> {
        if self.0 != "tsk" {
            return fail(
                "This backend recovers from its retained artifacts; no artifact is available.",
            );
        }
        if candidate.locator.is_empty()
            || !candidate
                .locator
                .chars()
                .all(|c| c.is_ascii_digit() || c == '-')
        {
            return fail("Invalid TSK metadata address in saved scan.");
        }
        if candidate.offset_bytes % sector as u64 != 0 {
            return fail("Candidate partition offset is not sector aligned.");
        }
        let mut spec = Spec::new("icat", work);
        spec.source(input)?;
        spec.output = Some(output.try_clone()?);
        spec.args(&["-r", "-b"])
            .arg(sector.to_string())
            .arg("-o")
            .arg((candidate.offset_bytes / sector as u64).to_string())
            .arg("/proc/self/fd/3")
            .arg(&candidate.locator);
        checked(process::run(spec, control, |_| Ok(()))?, "icat")
    }
}

pub fn partitions(
    input: &File,
    work: &Path,
    sector: u32,
    size: u64,
    control: &Control,
) -> Result<Vec<(u64, u64)>> {
    let mut spec = Spec::new("mmls", work);
    spec.source(input)?;
    spec.arg("-a")
        .arg("-b")
        .arg(sector.to_string())
        .arg("/proc/self/fd/3");
    let mut ranges = vec![];
    let outcome = process::run(spec, control, |line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() >= 5
            && fields[0].ends_with(':')
            && !fields[1].contains("Meta")
            && !fields[1].contains("-------")
        {
            if let (Ok(start), Ok(length)) = (fields[2].parse::<u64>(), fields[4].parse::<u64>()) {
                let start = start.checked_mul(sector as u64);
                let length = length.checked_mul(sector as u64);
                if let (Some(start), Some(length)) = (start, length) {
                    if length > 0 && start.checked_add(length).is_some_and(|end| end <= size) {
                        ranges.push((start, length));
                    }
                }
            }
        }
        Ok(())
    })?;
    if !outcome.status.success() || ranges.is_empty() {
        return Ok(vec![(0, size)]);
    }
    ranges.sort_unstable();
    ranges.dedup();
    Ok(ranges)
}

pub fn copy_range(
    input: &File,
    output: &mut File,
    offset: u64,
    length: u64,
    control: &Control,
    notify: &dyn Fn(Event),
) -> Result<()> {
    let mut input = input.try_clone()?;
    input.seek(SeekFrom::Start(offset))?;
    let mut remaining = length;
    let mut buffer = vec![0; 1024 * 1024];
    let mut last = std::time::Instant::now();
    while remaining > 0 {
        control.checkpoint()?;
        let want = remaining.min(buffer.len() as u64) as usize;
        let count = input.read(&mut buffer[..want])?;
        if count == 0 {
            return fail("Source ended before the requested byte range was copied.");
        }
        output.write_all(&buffer[..count])?;
        remaining -= count as u64;
        if last.elapsed().as_millis() > 200 {
            (notify)(Event::Stage {
                text: "Copying source range".into(),
                fraction: Some((length - remaining) as f64 / length as f64),
            });
            last = std::time::Instant::now();
        }
    }
    output.sync_all()?;
    Ok(())
}
