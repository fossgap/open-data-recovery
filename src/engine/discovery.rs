//! Read-only TestDisk partition searches. Search output is evidence, never a
//! command or a request to rewrite a partition table.
use super::{
    Error, Result, adapters, fail,
    process::{self, Control, Spec},
    safety::GuardedDir,
    sources, store, timestamp,
    types::{DiscoveredPartition, SourceDescriptor, SourceType},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    os::unix::fs::FileExt,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver},
};

const MAX_PARTITIONS: usize = 1024;
const MAX_LOG: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Table {
    #[default]
    Auto,
    Intel,
    Gpt,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Options {
    pub workspace: PathBuf,
    pub table: Table,
    pub deeper: bool,
    pub timeout_seconds: u64,
}
impl Options {
    pub(super) fn command(&self) -> String {
        let table = match self.table {
            Table::Auto => "",
            Table::Intel => "partition_i386,",
            Table::Gpt => "partition_gpt,",
        };
        format!("{table}analyze{}", if self.deeper { ",search" } else { "" })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub directory: PathBuf,
    pub source: SourceDescriptor,
    pub options: Options,
    pub created_at: u64,
    pub testdisk_version: String,
    pub table: String,
    pub complete: bool,
    pub cancelled: bool,
    pub error: Option<String>,
    pub warnings: Vec<String>,
    pub partitions: Vec<DiscoveredPartition>,
}
impl Report {
    pub fn path(&self) -> PathBuf {
        self.directory.join("report.json")
    }
    pub fn target(&self, index: usize) -> Result<SourceDescriptor> {
        if !self.complete {
            return fail("Only a completed partition search can supply scan targets.");
        }
        let partition = self.partitions.get(index).ok_or_else(|| {
            Error::Message("Choose a partition from the discovery report.".into())
        })?;
        sources::partition_source(&self.source, partition)
    }
}
pub struct SearchJob {
    pub control: Control,
    pub result: Receiver<Result<Report>>,
}
pub fn start(source: SourceDescriptor, options: Options) -> SearchJob {
    let control = Control::default();
    let worker = control.clone();
    let (tx, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(discover(source, options, &worker));
    });
    SearchJob { control, result }
}

pub fn eligible(source: &SourceDescriptor) -> bool {
    source.kind != SourceType::Folder
        && source.scope.is_none()
        && source.partition.is_none()
        && !source.ewf
}

pub fn discover(source: SourceDescriptor, options: Options, control: &Control) -> Result<Report> {
    control.checkpoint()?;
    if !eligible(&source) {
        return fail(
            "Partition discovery needs a raw/dd disk image or block device. For E01, select an exported raw image or retained clone; folders and discovered ranges cannot be searched as whole disks.",
        );
    }
    let source = sources::verify(&source)?;
    // Validate before creating logs or directories, as with file scans.
    super::safety::validate_storage(&source, &options.workspace, 0)?;
    let version = process::probe("testdisk", &["/version"])
        .ok_or_else(|| Error::Message("Install TestDisk to search for lost partitions.".into()))?;
    let input = sources::open_readonly(&source)?;
    let mut nonce = [0_u8; 12];
    File::open("/dev/urandom")?.read_exact(&mut nonce)?;
    let id = format!(
        "{}-{}",
        timestamp(),
        nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let directory = options.workspace.join("partition-searches").join(id);
    let guard = GuardedDir::create(&source, &directory, 0)?;
    let mut report = Report {
        schema_version: 1,
        directory,
        source,
        options,
        created_at: timestamp(),
        testdisk_version: version,
        table: String::new(),
        complete: false,
        cancelled: false,
        error: None,
        warnings: vec![],
        partitions: vec![],
    };
    store::write_json(&report.path(), &report)?;
    let result = (|| -> Result<()> {
        let mut spec = Spec::new("testdisk", &report.directory);
        spec.source(&input)?;
        spec.args(&["/log", "/cmd", "/proc/self/fd/3"])
            .arg(report.options.command());
        spec.timeout = report.options.timeout_seconds;
        // stdout is discarded through the bounded reader; TestDisk's own log
        // carries the search evidence. No interactive stdin is supplied.
        adapters::checked(
            process::run(spec, control, |_| Ok(()))?,
            "TestDisk partition discovery",
        )?;
        control.checkpoint()?;
        guard.check_identity()?;
        let file = File::open(super::safety::confined_artifact(
            &report.directory,
            "testdisk.log",
        )?)?;
        if file.metadata()?.len() > MAX_LOG {
            return fail("TestDisk's log exceeds the 32 MiB parsing limit. The log was retained.");
        }
        let mut log = String::new();
        file.take(MAX_LOG + 1).read_to_string(&mut log)?;
        if log.len() as u64 > MAX_LOG {
            return fail("TestDisk's log grew beyond the parsing limit.");
        }
        let (table, mut partitions, warnings) =
            parse_log(&log, report.source.size, report.source.sector_size)?;
        report.table = table;
        report.warnings = warnings;
        for partition in &mut partitions {
            control.checkpoint()?;
            probe_partition(&input, &report.directory, partition, control)?;
            partition.sample_sha256 = sample(&input, partition)?;
        }
        // Do not publish ranges from an input that changed during the search.
        sources::verify(&report.source)?;
        control.checkpoint()?;
        report.partitions = partitions;
        report.complete = true;
        Ok(())
    })();
    if let Err(error) = result {
        report.cancelled = matches!(error, Error::Cancelled);
        report.error = Some(error.to_string());
    }
    guard.check_identity()?;
    store::write_json(&report.path(), &report)?;
    Ok(report)
}

pub fn load(path: &Path) -> Result<Report> {
    let mut report: Report = super::read_json(path)?;
    if report.schema_version != 1
        || !eligible(&report.source)
        || report.partitions.len() > MAX_PARTITIONS
        || !["Intel", "EFI GPT"].contains(&report.table.as_str())
    {
        return fail("Unsupported or invalid partition-discovery report.");
    }
    if !report.complete || report.cancelled || report.error.is_some() {
        return fail(
            "This partition search did not complete. Run a new search; its logs remain available.",
        );
    }
    for partition in &report.partitions {
        sources::partition_source(&report.source, partition)?;
    }
    report.directory = fs::canonicalize(path)?
        .parent()
        .ok_or_else(|| Error::Message("Invalid report path.".into()))?
        .into();
    Ok(report)
}

pub fn sample(input: &File, partition: &DiscoveredPartition) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(partition.offset_bytes.to_le_bytes());
    hash.update(partition.length_bytes.to_le_bytes());
    let count = partition.length_bytes.min(65536) as usize;
    let mut data = vec![0_u8; count];
    input.read_exact_at(&mut data, partition.offset_bytes)?;
    hash.update(&data);
    input.read_exact_at(
        &mut data,
        partition.offset_bytes + partition.length_bytes - count as u64,
    )?;
    hash.update(&data);
    Ok(format!("{:x}", hash.finalize()))
}

fn probe_partition(
    input: &File,
    work: &Path,
    partition: &mut DiscoveredPartition,
    control: &Control,
) -> Result<()> {
    let mut spec = Spec::new("blkid", work);
    spec.source(input)?;
    spec.args(&["--probe", "--output", "export", "--offset"])
        .arg(partition.offset_bytes.to_string())
        .arg("--size")
        .arg(partition.length_bytes.to_string())
        .arg("/proc/self/fd/3");
    spec.timeout = 10;
    let mut fields = BTreeMap::new();
    let result = process::run(spec, control, |line| {
        if let Some((key, val)) = line.split_once('=') {
            if ["TYPE", "LABEL"].contains(&key) {
                fields.insert(key.to_string(), clean(val));
            }
        }
        Ok(())
    });
    match result {
        Ok(outcome) if outcome.status.success() => {
            if let Some(filesystem) = fields.remove("TYPE") {
                partition.filesystem = filesystem;
                partition.verified = true;
                if let Some(label) = fields.remove("LABEL") {
                    partition.label = label;
                }
            }
        }
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        // A backup superblock may reveal a partition whose primary header is
        // unreadable. Keep it as an explicitly unverified candidate.
        _ => {}
    }
    Ok(())
}

fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(512)
        .collect::<String>()
        .trim()
        .into()
}
fn filesystem_hint(description: &str) -> String {
    let lower = description.to_ascii_lowercase();
    let first = lower
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_end_matches(',');
    match first {
        "ext2" | "ext3" | "ext4" | "ntfs" | "exfat" | "btrfs" | "xfs" | "apfs" | "hfs" => {
            first.into()
        }
        "hfs+" => "hfsplus".into(),
        "fat12" | "fat16" | "fat32" => "vfat".into(),
        _ => String::new(),
    }
}

/// Parse only search sections, never the current partition table or shell hints.
/// Intel rows are CHS; GPT rows are LBA. Every inclusive end and byte boundary
/// is checked against the real source size, not TestDisk's rounded CHS size.
pub(super) fn parse_log(
    log: &str,
    source_size: u64,
    expected_sector: u32,
) -> Result<(String, Vec<DiscoveredPartition>, Vec<String>)> {
    let mut table = String::new();
    let mut heads = 0_u64;
    let mut sectors = 0_u64;
    let mut sector_size = 0_u32;
    let mut searching = false;
    let mut saw_search = false;
    let mut results: Vec<DiscoveredPartition> = vec![];
    let mut pending: Option<usize> = None;
    let mut rejected = 0;
    for line in log.lines() {
        if line.len() > 16384 {
            return fail("TestDisk's log contains an oversized line.");
        }
        if let Some(value) = line.strip_prefix("Partition table type: ") {
            table = value.trim().into();
        }
        if line.starts_with("Disk ") || line.starts_with("Analyse Disk ") {
            if let Some((_, geometry)) = line.split_once("CHS ") {
                let values: Vec<_> = geometry.split_whitespace().collect();
                heads = values.get(1).and_then(|v| v.parse().ok()).unwrap_or(0);
                sectors = values
                    .get(2)
                    .and_then(|v| v.trim_end_matches(',').parse().ok())
                    .unwrap_or(0);
            }
            if let Some((_, value)) = line
                .split_once("sector size=")
                .or_else(|| line.split_once("sector_size="))
            {
                sector_size = value
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
        }
        if line.trim() == "search_part()" {
            searching = true;
            saw_search = true;
            pending = None;
            continue;
        }
        if line.trim() == "interface_write()" {
            searching = false;
            pending = None;
            continue;
        }
        if !searching {
            continue;
        }
        if line.starts_with(char::is_whitespace) {
            if let Some((start, length, description, label)) = row(line, &table, heads, sectors) {
                let bytes = start
                    .checked_mul(sector_size as u64)
                    .zip(length.checked_mul(sector_size as u64));
                if let Some((offset_bytes, length_bytes)) = bytes {
                    let partition = DiscoveredPartition {
                        offset_bytes,
                        length_bytes,
                        sector_size,
                        filesystem: filesystem_hint(&description),
                        label,
                        description,
                        verified: false,
                        sample_sha256: String::new(),
                    };
                    if partition.validate(source_size).is_ok() {
                        let index = results.iter().position(|p| {
                            p.offset_bytes == offset_bytes && p.length_bytes == length_bytes
                        });
                        pending = Some(if let Some(index) = index {
                            index
                        } else {
                            if results.len() >= MAX_PARTITIONS {
                                return fail(
                                    "TestDisk found more than 1024 distinct ranges. Review the retained log.",
                                );
                            }
                            results.push(partition);
                            results.len() - 1
                        });
                        continue;
                    }
                }
                rejected += 1;
                pending = None;
            } else if let Some(index) = pending.take() {
                let hint = filesystem_hint(line.trim());
                if !hint.is_empty() {
                    results[index].filesystem = hint;
                }
                results[index].description =
                    clean(&format!("{} · {}", results[index].description, line.trim()));
            }
        } else if !line.trim().is_empty() {
            pending = None;
        }
    }
    if !["Intel", "EFI GPT"].contains(&table.as_str()) {
        return fail(
            "TestDisk did not use an MBR/Intel or GPT partition table. Choose MBR or GPT explicitly for a disk with a missing table.",
        );
    }
    if !saw_search || !log.lines().any(|l| l.trim() == "TestDisk exited normally.") {
        return fail(
            "TestDisk did not finish a recognizable partition search. Review the retained log.",
        );
    }
    if sector_size != expected_sector || ![512, 1024, 2048, 4096].contains(&sector_size) {
        return fail(
            "TestDisk's sector size differs from the selected source. No partition ranges were accepted.",
        );
    }
    if table == "Intel" && (heads == 0 || heads > 255 || sectors == 0 || sectors > 63) {
        return fail("TestDisk's CHS geometry is invalid; partition offsets cannot be verified.");
    }
    results.sort_by_key(|p| (p.offset_bytes, p.length_bytes));
    let mut warnings = vec![];
    if rejected > 0 {
        warnings.push(format!(
            "Rejected {rejected} range(s) outside the source or with invalid geometry."
        ));
    }
    if results
        .windows(2)
        .any(|p| p[0].offset_bytes + p[0].length_bytes > p[1].offset_bytes)
    {
        warnings.push("Some candidates overlap. They may be alternative or stale filesystem boundaries; scan one candidate at a time.".into());
    }
    Ok((table, results, warnings))
}

fn row(line: &str, table: &str, heads: u64, sectors: u64) -> Option<(u64, u64, String, String)> {
    let words: Vec<_> = line.split_whitespace().collect();
    let count = if table == "Intel" {
        7
    } else if table == "EFI GPT" {
        3
    } else {
        return None;
    };
    for begin in 1..words.len() {
        let Some(values) = words.get(begin..begin + count) else {
            break;
        };
        let Ok(numbers) = values
            .iter()
            .map(|v| v.parse::<u64>())
            .collect::<std::result::Result<Vec<_>, _>>()
        else {
            continue;
        };
        let mut prefix = &words[..begin];
        if prefix.first().is_some_and(|s| s.parse::<u32>().is_ok()) {
            prefix = &prefix[1..];
        }
        if prefix
            .first()
            .is_some_and(|s| ["*", "P", "L", "D", "E"].contains(s))
        {
            prefix = &prefix[1..];
        }
        let description = prefix.join(" ");
        if description.is_empty() || description.contains(['=', '[', '"', ':']) {
            continue;
        }
        let (start, end, length) = if table == "Intel" {
            let chs = |c: u64, h: u64, s: u64| -> Option<u64> {
                if heads == 0 || h >= heads || s == 0 || s > sectors {
                    return None;
                }
                c.checked_mul(heads)?
                    .checked_add(h)?
                    .checked_mul(sectors)?
                    .checked_add(s - 1)
            };
            (
                chs(numbers[0], numbers[1], numbers[2])?,
                chs(numbers[3], numbers[4], numbers[5])?,
                numbers[6],
            )
        } else {
            (numbers[0], numbers[1], numbers[2])
        };
        if length == 0 || start.checked_add(length)?.checked_sub(1)? != end {
            return None;
        }
        let trailing = words[begin + count..].join(" ");
        let label = trailing
            .split_once('[')
            .and_then(|(_, s)| s.rsplit_once(']').map(|(s, _)| clean(s)))
            .unwrap_or_default();
        return Some((start, length, clean(&description), label));
    }
    None
}
