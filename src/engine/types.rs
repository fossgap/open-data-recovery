use super::{Result, fail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SourceType {
    Block,
    Image,
    Folder,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    pub filesystem_uuid: String,
    pub partition_uuid: String,
    pub partition_offset: u64,
    pub drive_ids: Vec<String>,
    pub sample_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceDescriptor {
    pub path: PathBuf,
    pub name: String,
    pub kind: SourceType,
    pub filesystem: String,
    pub size: u64,
    pub sector_size: u32,
    pub mounted: bool,
    pub parent_path: Option<PathBuf>,
    pub mountpoints: Vec<PathBuf>,
    pub scope: Option<String>,
    pub fingerprint: Fingerprint,
    /// Leaf device identities include loop backing storage and mapper/RAID ancestors.
    pub ancestry: Vec<String>,
    pub ewf: bool,
    #[serde(default)]
    pub segments: Vec<PathBuf>,
    /// Virtual partition on the original source. Its fingerprint and ancestry
    /// still identify the whole input; scans retain this bounded range.
    #[serde(default)]
    pub partition: Option<DiscoveredPartition>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveredPartition {
    pub offset_bytes: u64,
    pub length_bytes: u64,
    pub sector_size: u32,
    pub filesystem: String,
    pub label: String,
    pub description: String,
    pub verified: bool,
    pub sample_sha256: String,
}
impl DiscoveredPartition {
    pub fn validate(&self, source_size: u64) -> Result<()> {
        if ![512, 1024, 2048, 4096].contains(&self.sector_size)
            || self.length_bytes == 0
            || self.offset_bytes % self.sector_size as u64 != 0
            || self.length_bytes % self.sector_size as u64 != 0
            || self
                .offset_bytes
                .checked_add(self.length_bytes)
                .is_none_or(|end| end > source_size)
        {
            return fail("The discovered partition has an invalid or out-of-source byte range.");
        }
        Ok(())
    }
}
impl SourceDescriptor {
    pub fn same_target(&self, other: &Self) -> bool {
        self.path == other.path
            && self.scope == other.scope
            && self
                .partition
                .as_ref()
                .map(|p| (p.offset_bytes, p.length_bytes, p.sector_size))
                == other
                    .partition
                    .as_ref()
                    .map(|p| (p.offset_bytes, p.length_bytes, p.sector_size))
    }
    pub fn target_size(&self) -> u64 {
        self.partition
            .as_ref()
            .map_or(self.size, |p| p.length_bytes)
    }
    pub fn target_offset(&self) -> u64 {
        self.partition
            .as_ref()
            .map_or(self.fingerprint.partition_offset, |p| p.offset_bytes)
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Mode {
    Quick,
    Deep,
    #[default]
    Full,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Conflict {
    #[default]
    Rename,
    Skip,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanOptions {
    pub mode: Mode,
    pub workspace: PathBuf,
    pub extensions: Vec<String>,
    pub include_existing: bool,
    pub consolidate: bool,
    pub clone_first: bool,
    pub clone_backend: String,
    pub retry_passes: u32,
    pub metadata_backend: String,
    pub carver: String,
    pub keep_corrupted: bool,
    pub low_memory: bool,
    pub offset_bytes: u64,
    pub length_bytes: Option<u64>,
    pub minimum_size: u64,
    pub maximum_size: Option<u64>,
    pub timeout_seconds: u64,
    pub btrfs_root: Option<u64>,
}
impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            mode: Mode::Full,
            workspace: default_workspace(),
            extensions: vec![],
            include_existing: false,
            consolidate: false,
            clone_first: false,
            clone_backend: "ddrescue".into(),
            retry_passes: 0,
            metadata_backend: "auto".into(),
            carver: "auto".into(),
            keep_corrupted: false,
            low_memory: true,
            offset_bytes: 0,
            length_bytes: None,
            minimum_size: 0,
            maximum_size: None,
            timeout_seconds: 0,
            btrfs_root: None,
        }
    }
}
impl ScanOptions {
    pub fn validate(&mut self) -> Result<()> {
        self.extensions = normalize_extensions(&self.extensions.join(","))?;
        if self.retry_passes > 20 {
            return fail("Retry passes must be between 0 and 20.");
        }
        if self.maximum_size.is_some_and(|max| max < self.minimum_size) {
            return fail("Maximum size must be greater than minimum size.");
        }
        if self.length_bytes == Some(0) {
            return fail("A byte range must have a nonzero length.");
        }
        if self.workspace.as_os_str().is_empty() {
            return fail("Choose a workspace on a separate drive.");
        }
        Ok(())
    }
    pub fn accepts(&self, name: &str, bytes: u64) -> bool {
        let extension = std::path::Path::new(name)
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        (self.extensions.is_empty() || self.extensions.contains(&extension))
            && bytes >= self.minimum_size
            && self.maximum_size.is_none_or(|max| bytes <= max)
    }
}
pub fn normalize_extensions(text: &str) -> Result<Vec<String>> {
    let mut extensions = Vec::new();
    for entry in text
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        let entry = entry
            .trim_start_matches("*.")
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if entry.len() > 20 || entry.is_empty() || !entry.bytes().all(|c| c.is_ascii_alphanumeric())
        {
            return fail(
                "Extensions must be names such as jpg, png, pdf; no commands or paths are accepted.",
            );
        }
        extensions.push(entry);
    }
    extensions.sort();
    extensions.dedup();
    Ok(extensions)
}
pub fn default_workspace() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| PathBuf::from(v).is_absolute())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("open-data-recovery/snapshots")
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct RecoveryOptions {
    pub destination: PathBuf,
    pub preserve_tree: bool,
    pub conflict: Conflict,
    pub preserve_timestamps: bool,
    pub verify_after_write: bool,
    pub continue_on_error: bool,
    /// Restore every distinct recorded path instead of one copy per content group.
    pub restore_all_paths: bool,
}
impl Default for RecoveryOptions {
    fn default() -> Self {
        Self {
            destination: PathBuf::new(),
            preserve_tree: true,
            conflict: Conflict::Rename,
            preserve_timestamps: true,
            verify_after_write: true,
            continue_on_error: true,
            restore_all_paths: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Candidate {
    pub id: i64,
    pub path: String,
    pub bytes: u64,
    pub modified: i64,
    pub backend: String,
    pub backend_version: String,
    pub locator: String,
    pub offset_bytes: u64,
    pub length_bytes: u64,
    pub artifact: Option<String>,
    pub carved: bool,
    pub selected: bool,
    pub deleted: bool,
    #[serde(default = "one")]
    pub findings: u64,
    #[serde(default = "one")]
    pub paths: u64,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub consolidation_error: Option<String>,
}

fn one() -> u64 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Consolidation {
    pub findings: u64,
    pub groups: u64,
    pub unverified: u64,
    pub cached_bytes: u64,
    pub duration_millis: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub state: String,
    pub source: SourceDescriptor,
    pub options: ScanOptions,
    pub engines: BTreeMap<String, String>,
    pub clone: Option<String>,
    pub clone_map: Option<String>,
    pub scan_input: Option<String>,
    pub errors: Vec<String>,
    pub candidate_count: u64,
    pub complete: bool,
    /// Present only after the complete grouping transaction has been committed.
    #[serde(default)]
    pub consolidation: Option<Consolidation>,
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    pub text: String,
    pub kind: String,
    pub offset: usize,
    pub limit: usize,
}
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub files: Vec<Candidate>,
    pub matched: u64,
    pub total: u64,
    pub selected: u64,
    pub selected_bytes: u64,
    pub selected_paths: u64,
    pub selected_path_bytes: u64,
    pub findings: u64,
    pub consolidated: bool,
}
#[derive(Clone, Debug)]
pub struct SavedScan {
    pub directory: PathBuf,
    pub manifest: Manifest,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryRecord {
    pub candidate_id: i64,
    pub original_path: String,
    pub output_path: Option<PathBuf>,
    pub bytes: u64,
    pub sha256: Option<String>,
    pub verified: bool,
    pub error: Option<String>,
    pub timestamp: u64,
}

#[derive(Clone, Debug)]
pub enum Event {
    Message(String),
    Stage {
        text: String,
        fraction: Option<f64>,
    },
    Session(PathBuf),
    Candidates(u64),
    Finished {
        directory: PathBuf,
        cancelled: bool,
        error: Option<String>,
    },
    Recovered {
        successful: u64,
        failed: u64,
        skipped: u64,
    },
}
