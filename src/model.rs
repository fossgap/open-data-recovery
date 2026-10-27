//! Pure, deterministic interaction model. This module never opens a source or writes files.
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanMode {
    Quick,
    Deep,
    Full,
}

impl ScanMode {
    pub fn from_index(index: u32) -> Self {
        match index {
            0 => Self::Quick,
            1 => Self::Deep,
            _ => Self::Full,
        }
    }

    pub fn index(self) -> u32 {
        match self {
            Self::Quick => 0,
            Self::Deep => 1,
            Self::Full => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Quick => "Quick scan",
            Self::Deep => "Deep scan",
            Self::Full => "Full scan",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Quick => "Search filesystem metadata. Original names may survive.",
            Self::Deep => "Search file contents. Original names and folders may be lost.",
            Self::Full => "Search filesystem metadata, then carve for additional files.",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Ready,
    Scanning,
    Paused,
    Complete,
    Cancelled,
}

impl Phase {
    pub fn busy(self) -> bool {
        matches!(self, Self::Scanning | Self::Paused)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Disk,
    Partition,
    Unallocated,
    Image,
    Mapper,
    Folder,
}

#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub detail: String,
    pub path: String,
    pub filesystem: &'static str,
    pub capacity: &'static str,
    pub icon: &'static str,
    pub mounted: bool,
    pub carve_only: bool,
    pub kind: SourceKind,
    /// Parent indices are stable within the demo workspace, including saved scans.
    pub parent: Option<usize>,
    pub size_mib: u64,
}

pub fn sources() -> Vec<Source> {
    vec![
        Source {
            name: "Archive".into(),
            detail: "Partition 1 · NTFS".into(),
            path: "/dev/sdb1".into(),
            filesystem: "NTFS",
            capacity: "931.5 GiB",
            icon: "drive-harddisk-usb-symbolic",
            mounted: false,
            carve_only: false,
            kind: SourceKind::Partition,
            parent: Some(6),
            size_mib: 953856,
        },
        Source {
            name: "Linux home".into(),
            detail: "Mounted partition · ext4".into(),
            path: "/dev/nvme0n1p3".into(),
            filesystem: "ext4",
            capacity: "476.0 GiB",
            icon: "drive-harddisk-solidstate-symbolic",
            mounted: true,
            carve_only: false,
            kind: SourceKind::Partition,
            parent: Some(7),
            size_mib: 487424,
        },
        Source {
            name: "Camera photos".into(),
            detail: "Partition 1 · exFAT".into(),
            path: "/dev/mmcblk0p1".into(),
            filesystem: "exFAT",
            capacity: "59.5 GiB",
            icon: "media-flash-symbolic",
            mounted: false,
            carve_only: false,
            kind: SourceKind::Partition,
            parent: Some(8),
            size_mib: 60928,
        },
        Source {
            name: "Evidence image".into(),
            detail: "E01 image · NTFS".into(),
            path: "/demo/evidence.E01".into(),
            filesystem: "NTFS",
            capacity: "232.9 GiB",
            icon: "drive-optical-symbolic",
            mounted: false,
            carve_only: false,
            kind: SourceKind::Image,
            parent: None,
            size_mib: 238490,
        },
        Source {
            name: "Unlocked volume".into(),
            detail: "Unlocked mapper · btrfs".into(),
            path: "/dev/mapper/archive".into(),
            filesystem: "btrfs",
            capacity: "238.5 GiB",
            icon: "drive-harddisk-symbolic",
            mounted: true,
            carve_only: false,
            kind: SourceKind::Mapper,
            parent: None,
            size_mib: 244224,
        },
        Source {
            name: "Mac archive".into(),
            detail: "Raw image · APFS".into(),
            path: "/demo/mac-archive.dd".into(),
            filesystem: "APFS",
            capacity: "465.8 GiB",
            icon: "drive-optical-symbolic",
            mounted: false,
            carve_only: true,
            kind: SourceKind::Image,
            parent: None,
            size_mib: 476979,
        },
        Source {
            name: "Archive SSD".into(),
            detail: "External SSD · GPT".into(),
            path: "/dev/sdb".into(),
            filesystem: "GPT disk",
            capacity: "1.0 TiB",
            icon: "drive-harddisk-usb-symbolic",
            mounted: false,
            carve_only: false,
            kind: SourceKind::Disk,
            parent: None,
            size_mib: 1048576,
        },
        Source {
            name: "System NVMe".into(),
            detail: "Internal SSD · GPT".into(),
            path: "/dev/nvme0n1".into(),
            filesystem: "GPT disk",
            capacity: "512.0 GiB",
            icon: "drive-harddisk-solidstate-symbolic",
            mounted: true,
            carve_only: false,
            kind: SourceKind::Disk,
            parent: None,
            size_mib: 524288,
        },
        Source {
            name: "Camera card".into(),
            detail: "Removable card · MBR".into(),
            path: "/dev/mmcblk0".into(),
            filesystem: "MBR disk",
            capacity: "59.5 GiB",
            icon: "media-flash-symbolic",
            mounted: false,
            carve_only: false,
            kind: SourceKind::Disk,
            parent: None,
            size_mib: 60928,
        },
        Source {
            name: "Unallocated space".into(),
            detail: "After partition 1 · carve only".into(),
            path: "/dev/sdb (unallocated)".into(),
            filesystem: "Unallocated",
            capacity: "92.5 GiB",
            icon: "view-grid-symbolic",
            mounted: false,
            carve_only: true,
            kind: SourceKind::Unallocated,
            parent: Some(6),
            size_mib: 94720,
        },
        Source {
            name: "EFI system".into(),
            detail: "Partition 1 · FAT32".into(),
            path: "/dev/nvme0n1p1".into(),
            filesystem: "FAT32",
            capacity: "512 MiB",
            icon: "drive-harddisk-symbolic",
            mounted: true,
            carve_only: false,
            kind: SourceKind::Partition,
            parent: Some(7),
            size_mib: 512,
        },
        Source {
            name: "Linux system".into(),
            detail: "Partition 2 · ext4".into(),
            path: "/dev/nvme0n1p2".into(),
            filesystem: "ext4",
            capacity: "35.5 GiB",
            icon: "drive-harddisk-symbolic",
            mounted: true,
            carve_only: false,
            kind: SourceKind::Partition,
            parent: Some(7),
            size_mib: 36352,
        },
    ]
}

/// Partition order, independent of source IDs used by sessions and selections.
pub fn disk_children(sources: &[Source], disk: usize) -> Vec<usize> {
    let mut children: Vec<_> = sources
        .iter()
        .enumerate()
        .filter(|(_, source)| source.parent == Some(disk))
        .map(|(id, _)| id)
        .collect();
    children.sort_by_key(|&id| {
        (
            sources[id].kind == SourceKind::Unallocated,
            &sources[id].path,
        )
    });
    children
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Document,
    Other,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: usize,
    pub folder: &'static str,
    pub name: &'static str,
    pub bytes: u64,
    pub kind: Kind,
    pub modified: &'static str,
    pub carved: bool,
    pub preview: &'static str,
}

impl Entry {
    pub fn icon(&self) -> &'static str {
        match self.kind {
            Kind::Image => "image-x-generic-symbolic",
            Kind::Document => "x-office-document-symbolic",
            Kind::Other => "package-x-generic-symbolic",
        }
    }

    pub fn status(&self) -> &'static str {
        if self.carved { "Carved" } else { "Name found" }
    }
}

pub const FOLDERS: [&str; 4] = ["Documents", "Pictures", "Projects", "Lost files"];

pub fn entries(mode: ScanMode) -> Vec<Entry> {
    let fixtures = [
        (
            "Documents",
            "Field notes.txt",
            4280,
            Kind::Document,
            "18 Oct 2026",
            false,
            "FIELD NOTES · Autumn, 2026\n\n18 October — Alpine trail\nFirst light over the ridge, 07:12.\nWe followed the path to the lake.\n\nRemember: spare batteries, a paper\nmap, and a little more time.\n\nBundled sample content.",
        ),
        (
            "Documents",
            "Project proposal.pdf",
            1835008,
            Kind::Document,
            "16 Oct 2026",
            false,
            "PDF document\n\nA production preview will render an isolated,\nsize-limited copy of the selected candidate.\n\nThis fixture does not contain a real PDF.",
        ),
        (
            "Documents",
            "Budget 2026.xlsx",
            48332,
            Kind::Document,
            "14 Oct 2026",
            false,
            "Spreadsheet\n\nPreview unavailable for this sample format.\nYou can still select the file for recovery.",
        ),
        (
            "Documents",
            "Reading list.md",
            1834,
            Kind::Document,
            "12 Oct 2026",
            false,
            "# Reading list\n\n• The craft of observation\n• A field guide to the mountains\n• Notes on making things\n\nSample content for the design review.",
        ),
        (
            "Pictures",
            "Alpine morning.jpg",
            4831920,
            Kind::Image,
            "18 Oct 2026",
            false,
            "Illustrated sample preview; not recovered media.",
        ),
        (
            "Pictures",
            "Coastline.png",
            7248096,
            Kind::Image,
            "09 Oct 2026",
            false,
            "Illustrated sample preview; not recovered media.",
        ),
        (
            "Pictures",
            "Contact sheet.jpg",
            2941824,
            Kind::Image,
            "07 Oct 2026",
            false,
            "Illustrated sample preview; not recovered media.",
        ),
        (
            "Projects",
            "Archive.zip",
            28311552,
            Kind::Other,
            "11 Oct 2026",
            false,
            "Archive preview unavailable.\n\nA recovered file is not necessarily intact.\nVerification will be recorded after recovery.",
        ),
        (
            "Projects",
            "README.md",
            2342,
            Kind::Document,
            "10 Oct 2026",
            false,
            "# A project worth saving\n\nIdeas, drafts, and the work in between.\n\nThis file is part of the bundled demo dataset.",
        ),
        (
            "Projects",
            "Interview.wav",
            41943040,
            Kind::Other,
            "04 Oct 2026",
            false,
            "Audio preview unavailable in the UI prototype.\n\nNo external player is launched.",
        ),
        (
            "Lost files",
            "f0001048.jpg",
            3421440,
            Kind::Image,
            "Unknown",
            true,
            "Carving found a possible image signature.\nThe original name and path are unknown.",
        ),
        (
            "Lost files",
            "f0002086.pdf",
            829440,
            Kind::Document,
            "Unknown",
            true,
            "Carving found a possible PDF signature.\nThe original name and path are unknown.",
        ),
        (
            "Lost files",
            "f0003102.png",
            2076672,
            Kind::Image,
            "Unknown",
            true,
            "Carving found a possible image signature.\nThe original name and path are unknown.",
        ),
        (
            "Lost files",
            "f0004120.zip",
            9633792,
            Kind::Other,
            "Unknown",
            true,
            "Carved archive candidate.\nThe original name and path are unknown.",
        ),
    ];
    fixtures
        .into_iter()
        .enumerate()
        .filter_map(
            |(id, (folder, name, bytes, kind, modified, carved, preview))| {
                let included = match mode {
                    ScanMode::Quick => !carved,
                    ScanMode::Deep => carved,
                    ScanMode::Full => true,
                };
                included.then_some(Entry {
                    id,
                    folder,
                    name,
                    bytes,
                    kind,
                    modified,
                    carved,
                    preview,
                })
            },
        )
        .collect()
}

#[derive(Clone, Debug)]
pub struct Session {
    pub source: usize,
    pub mode: ScanMode,
    pub clone_first: bool,
    pub phase: Phase,
    pub progress: u32,
    pub found: Vec<Entry>,
    pub selected: BTreeSet<usize>,
}

impl Session {
    pub fn ready(source: usize) -> Self {
        Self {
            source,
            mode: ScanMode::Full,
            clone_first: false,
            phase: Phase::Ready,
            progress: 0,
            found: vec![],
            selected: BTreeSet::new(),
        }
    }

    pub fn example(source: usize) -> Self {
        Self {
            phase: Phase::Complete,
            progress: 100,
            found: entries(ScanMode::Full),
            ..Self::ready(source)
        }
    }

    pub fn start(&mut self, mode: ScanMode, clone_first: bool) -> bool {
        if self.phase.busy() {
            return false;
        }
        self.mode = mode;
        self.clone_first = clone_first;
        self.phase = Phase::Scanning;
        self.progress = 0;
        self.found.clear();
        self.selected.clear();
        true
    }

    pub fn tick(&mut self) -> bool {
        if self.phase != Phase::Scanning {
            return false;
        }
        self.progress = (self.progress + 2).min(100);
        let available = entries(self.mode);
        let scan_progress = if self.clone_first {
            self.progress.saturating_sub(20) * 100 / 80
        } else {
            self.progress
        };
        let count = available.len() * scan_progress as usize / 100;
        let changed = self.found.len() != count;
        self.found = available.into_iter().take(count).collect();
        if self.progress == 100 {
            self.phase = Phase::Complete;
        }
        changed
    }

    pub fn pause_or_resume(&mut self) -> bool {
        self.phase = match self.phase {
            Phase::Scanning => Phase::Paused,
            Phase::Paused => Phase::Scanning,
            _ => return false,
        };
        true
    }

    pub fn cancel(&mut self) -> bool {
        if !self.phase.busy() {
            return false;
        }
        self.phase = Phase::Cancelled;
        true
    }

    pub fn select(&mut self, id: usize, selected: bool) {
        if !self.found.iter().any(|entry| entry.id == id) {
            return;
        }
        if selected {
            self.selected.insert(id);
        } else {
            self.selected.remove(&id);
        }
    }

    pub fn selected_bytes(&self) -> u64 {
        self.found
            .iter()
            .filter(|entry| self.selected.contains(&entry.id))
            .map(|entry| entry.bytes)
            .sum()
    }

    pub fn stage(&self) -> &'static str {
        if self.clone_first && self.progress < 20 && self.phase.busy() {
            return if self.phase == Phase::Paused {
                "Clone simulation paused"
            } else {
                "Simulating clone-first"
            };
        }
        match self.phase {
            Phase::Ready => "Ready to scan",
            Phase::Paused => "Scan paused",
            Phase::Cancelled => "Scan cancelled · partial results retained",
            Phase::Complete => "Scan complete",
            Phase::Scanning
                if self.mode == ScanMode::Deep
                    || (self.mode == ScanMode::Full && self.progress > 60) =>
            {
                "Searching file signatures"
            }
            Phase::Scanning => "Reading filesystem metadata",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Images,
    Documents,
    Other,
}

pub fn matches(entry: &Entry, filter: Filter, query: &str) -> bool {
    let kind_matches = match filter {
        Filter::All => true,
        Filter::Images => entry.kind == Kind::Image,
        Filter::Documents => entry.kind == Kind::Document,
        Filter::Other => entry.kind == Kind::Other,
    };
    kind_matches
        && format!("{}/{}", entry.folder, entry.name)
            .to_lowercase()
            .contains(&query.trim().to_lowercase())
}

pub fn size(bytes: u64) -> String {
    for (unit, divisor) in [("GiB", 1_u64 << 30), ("MiB", 1 << 20), ("KiB", 1 << 10)] {
        if bytes >= divisor {
            return format!("{:.1} {unit}", bytes as f64 / divisor as f64);
        }
    }
    format!("{bytes} B")
}

/// Fictional topology for the destination dialog. Never use this as a real disk-safety check.
pub fn demo_destination(index: u32) -> Result<(), &'static str> {
    match index {
        1 => Ok(()),
        2 => Err("This sample destination shares the source device. Choose a separate drive."),
        3 => Err("The destination device cannot be verified. Recovery stays blocked."),
        _ => Err("Choose a separate drive for recovered files."),
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub title: String,
    pub session: Session,
}

impl Snapshot {
    pub fn capture(title: String, session: &Session) -> Self {
        let mut saved = session.clone();
        if saved.phase == Phase::Scanning {
            saved.phase = Phase::Paused;
        }
        Self {
            title,
            session: saved,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_resume_and_cancel_preserve_partial_results() {
        let mut session = Session::ready(0);
        assert!(session.start(ScanMode::Full, false));
        for _ in 0..20 {
            session.tick();
        }
        assert!(!session.found.is_empty());
        let progress = session.progress;
        assert!(session.pause_or_resume());
        assert!(!session.tick());
        assert_eq!(session.progress, progress);
        assert!(!session.start(ScanMode::Quick, false));
        assert!(session.pause_or_resume());
        session.tick();
        let found = session.found.len();
        assert!(session.cancel());
        assert!(!session.tick());
        assert_eq!(session.found.len(), found);
        assert!(!session.pause_or_resume());
    }

    #[test]
    fn clone_finishes_before_candidates_appear_and_each_mode_completes() {
        for mode in [ScanMode::Quick, ScanMode::Deep, ScanMode::Full] {
            let mut session = Session::ready(0);
            session.start(mode, true);
            for _ in 0..10 {
                session.tick();
            }
            assert!(session.found.is_empty());
            for _ in 0..40 {
                session.tick();
            }
            assert_eq!(session.phase, Phase::Complete);
            assert_eq!(session.progress, 100);
            assert_eq!(session.found.len(), entries(mode).len());
            assert!(!session.tick());
        }
    }

    #[test]
    fn restart_clears_stale_selections_and_rejects_unknown_ids() {
        let mut session = Session::example(0);
        session.select(0, true);
        session.select(999, true);
        assert_eq!(session.selected.len(), 1);
        assert_eq!(session.selected_bytes(), 4280);
        assert!(session.start(ScanMode::Deep, false));
        assert!(session.selected.is_empty());
        assert!(session.found.is_empty());
    }

    #[test]
    fn snapshot_is_independent_and_active_attempt_reopens_paused() {
        let mut session = Session::ready(0);
        session.start(ScanMode::Full, false);
        for _ in 0..15 {
            session.tick();
        }
        let snapshot = Snapshot::capture("Example".into(), &session);
        session.cancel();
        session.start(ScanMode::Deep, false);
        assert_eq!(snapshot.session.phase, Phase::Paused);
        assert_eq!(snapshot.session.progress, 30);
        assert!(!snapshot.session.found.is_empty());
        assert_eq!(snapshot.session.mode, ScanMode::Full);
    }

    #[test]
    fn search_combines_path_and_type_without_changing_selection() {
        let mut session = Session::example(0);
        session.select(0, true);
        assert!(matches(
            &session.found[0],
            Filter::Documents,
            " FIELD NOTES "
        ));
        assert!(matches(&session.found[0], Filter::All, "documents/field"));
        assert!(!matches(&session.found[0], Filter::Images, "field"));
        assert_eq!(session.selected.len(), 1);
    }

    #[test]
    fn unchosen_shared_and_unknown_destinations_are_blocked() {
        assert!(demo_destination(0).is_err());
        assert!(demo_destination(1).is_ok());
        assert!(demo_destination(2).is_err());
        assert!(demo_destination(3).is_err());
        assert!(demo_destination(u32::MAX).is_err());
    }
}
