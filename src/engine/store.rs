use super::{
    Result, fail, read_json,
    safety::{self, GuardedDir},
    timestamp,
    types::*,
};
use rusqlite::{Connection, OpenFlags, params};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

const READONLY: OpenFlags = OpenFlags::SQLITE_OPEN_READ_ONLY
    .union(OpenFlags::SQLITE_OPEN_NO_MUTEX)
    .union(OpenFlags::SQLITE_OPEN_NOFOLLOW);

struct ScanLock(File);
impl Drop for ScanLock {
    fn drop(&mut self) {
        // Unlock explicitly: a concurrently spawned child can briefly inherit
        // this descriptor between fork and exec, before CLOEXEC closes it.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

pub struct Store {
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub(super) connection: Connection,
    projection: String,
    table: String,
    // Released by the OS even if a scan crashes. This distinguishes a live
    // finalizer from a saved manifest whose last state was still active.
    _scan_lock: Option<ScanLock>,
}

impl Store {
    pub fn create(
        source: &SourceDescriptor,
        options: &ScanOptions,
        engines: BTreeMap<String, String>,
    ) -> Result<Self> {
        let reserve = source
            .size
            .saturating_mul(options.clone_first as u64 + source.ewf as u64)
            .saturating_add(
                if options.offset_bytes != 0 || options.length_bytes.is_some() {
                    options
                        .length_bytes
                        .unwrap_or(source.size.saturating_sub(options.offset_bytes))
                } else {
                    0
                },
            );
        let root = GuardedDir::create(source, &options.workspace, reserve)?;
        root.revalidate(source, reserve)?;
        let mut random = [0_u8; 12];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        let id = format!(
            "{}-{}",
            timestamp(),
            random
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
        let directory = root.path.join(&id);
        fs::create_dir(&directory)?;
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        fs::create_dir(directory.join("artifacts"))?;
        let connection = Connection::open_with_flags(
            directory.join("tree.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE candidates(id INTEGER PRIMARY KEY, path TEXT NOT NULL, bytes INTEGER NOT NULL, modified INTEGER NOT NULL,
              backend TEXT NOT NULL, backend_version TEXT NOT NULL, locator TEXT NOT NULL, offset_bytes INTEGER NOT NULL,
              length_bytes INTEGER NOT NULL, artifact TEXT, carved INTEGER NOT NULL, deleted INTEGER NOT NULL,
              UNIQUE(backend,locator,offset_bytes,path));
            CREATE INDEX candidate_path ON candidates(path); CREATE INDEX candidate_backend ON candidates(backend);
            PRAGMA user_version=1;")?;
        let scan_lock = if options.consolidate {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(directory.join("tree.sqlite"))?;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            // Create while empty: bounded inserts avoid a drive-sized sort at
            // finalization, and default scans pay no additional indexing cost.
            connection.execute_batch(
                "CREATE INDEX candidate_size ON candidates(bytes,(artifact IS NULL),id)",
            )?;
            Some(ScanLock(file))
        } else {
            None
        };
        let manifest = Manifest {
            schema_version: if options.consolidate { 2 } else { 1 },
            id,
            created_at: timestamp(),
            updated_at: timestamp(),
            state: "preparing".into(),
            source: source.clone(),
            options: options.clone(),
            engines,
            clone: None,
            clone_map: None,
            scan_input: None,
            errors: vec![],
            candidate_count: 0,
            complete: false,
            consolidation: None,
        };
        let mut store = Self {
            directory,
            manifest,
            connection,
            projection: native_projection().into(),
            table: "candidates".into(),
            _scan_lock: scan_lock,
        };
        write_json(&store.directory.join("source.json"), source)?;
        store.save()?;
        store.log(
            "session_created",
            &serde_json::json!({"source":source.path,"mode":options.mode}),
        )?;
        Ok(store)
    }

    pub fn open(directory: &Path) -> Result<Self> {
        GuardedDir::existing(directory)?;
        let manifest: Manifest =
            read_json(&safety::confined_artifact(directory, "manifest.json")?)?;
        if manifest.schema_version > 2 {
            return fail("This saved scan uses a newer session format.");
        }
        let db = safety::confined_artifact(directory, "tree.sqlite")?;
        let connection = Connection::open_with_flags(db, READONLY)?;
        let (mut table, mut projection) = legacy_projection(&connection)?;
        if manifest.consolidation.is_some() {
            // Only a committed index explicitly published by the manifest is
            // visible. An interrupted attempt always reopens the raw findings.
            table = "consolidated_candidates".into();
            projection = consolidated_projection().into();
            connection.prepare(&format!("SELECT {projection} FROM {table} LIMIT 0"))?;
        }
        Ok(Self {
            directory: directory.into(),
            manifest,
            connection,
            projection,
            table,
            _scan_lock: None,
        })
    }
    pub fn save(&mut self) -> Result<()> {
        self.manifest.updated_at = timestamp();
        self.manifest.candidate_count = self.count()?;
        write_json(&self.directory.join("manifest.json"), &self.manifest)
    }
    pub fn insert(&mut self, file: &Candidate) -> Result<()> {
        if file.bytes > i64::MAX as u64
            || file.offset_bytes > i64::MAX as u64
            || file.length_bytes > i64::MAX as u64
        {
            return fail("Candidate exceeds supported byte-address range.");
        }
        self.connection.execute("INSERT OR IGNORE INTO candidates(path,bytes,modified,backend,backend_version,locator,offset_bytes,length_bytes,artifact,carved,deleted) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![file.path,file.bytes as i64,file.modified,file.backend,file.backend_version,file.locator,file.offset_bytes as i64,file.length_bytes as i64,file.artifact,file.carved,file.deleted])?;
        Ok(())
    }
    pub fn begin_batch(&self) -> Result<()> {
        self.connection.execute_batch("BEGIN IMMEDIATE")?;
        Ok(())
    }
    pub fn commit_batch(&self) -> Result<()> {
        self.connection.execute_batch("COMMIT")?;
        Ok(())
    }
    pub fn checkpoint(&self) -> Result<()> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        Ok(())
    }
    pub fn count(&self) -> Result<u64> {
        Ok(self.connection.query_row(
            &format!("SELECT COUNT(*) FROM {}", self.table),
            [],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }
    pub fn get(&self, id: i64) -> Result<Candidate> {
        let mut file = self.connection.query_row(
            &format!("SELECT {} FROM {} WHERE id=?1", self.projection, self.table),
            [id],
            row_candidate,
        )?;
        if file.backend_version.is_empty() {
            file.backend_version = self
                .manifest
                .engines
                .get(&file.backend)
                .cloned()
                .unwrap_or_else(|| "unknown (legacy scan)".into());
        }
        Ok(file)
    }
    pub fn consolidated(&self) -> bool {
        self.manifest.consolidation.is_some()
    }

    pub(super) fn publish_consolidation(&mut self, summary: Consolidation) -> Result<()> {
        self.table = "consolidated_candidates".into();
        self.projection = consolidated_projection().into();
        self.manifest.consolidation = Some(summary);
        self.save()
    }

    /// Bounded provenance page. Group selection remains one copy; every raw
    /// finding, original path and engine address remains available here.
    pub fn findings(&self, group: i64, after: i64) -> Result<Vec<Candidate>> {
        if !self.consolidated() {
            return Ok(vec![self.get(group)?]);
        }
        let mut statement = self.connection.prepare(&format!(
            "SELECT {} FROM candidates WHERE id IN (SELECT candidate_id FROM consolidated_members WHERE group_id=?1 AND candidate_id>?2 ORDER BY candidate_id LIMIT 100) ORDER BY id",
            native_projection()))?;
        Ok(statement
            .query_map(params![group, after], row_candidate)?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Each distinct recorded path, with the group's verified canonical content.
    pub fn recovery_paths(&self, file: &Candidate, after: Option<&str>) -> Result<Vec<Candidate>> {
        if !self.consolidated() {
            return Ok(if after.is_none() {
                vec![file.clone()]
            } else {
                vec![]
            });
        }
        let mut statement = self.connection.prepare(
            "SELECT p.path,c.modified FROM consolidated_paths p JOIN candidates c ON c.id=p.candidate_id WHERE p.group_id=?1 AND (?2 IS NULL OR p.path>?2) ORDER BY p.path LIMIT 100")?;
        Ok(statement
            .query_map(params![file.id, after], |row| {
                let mut candidate = file.clone();
                candidate.path = row.get(0)?;
                candidate.modified = row.get(1)?;
                Ok(candidate)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }
    pub fn page(&self, query: &Query) -> Result<Page> {
        let filter = format!("SELECT {} FROM {}", self.projection, self.table);
        let sql_where = "instr(lower(path),lower(?1)) > 0 AND (?2='' OR (?2='image' AND lower(path) GLOB '*.*' AND lower(substr(path,instr(path,'.')+1)) IS NOT NULL AND (lower(path) LIKE '%.jpg' OR lower(path) LIKE '%.jpeg' OR lower(path) LIKE '%.png' OR lower(path) LIKE '%.gif' OR lower(path) LIKE '%.webp' OR lower(path) LIKE '%.tif' OR lower(path) LIKE '%.bmp')) OR (?2='document' AND (lower(path) LIKE '%.txt' OR lower(path) LIKE '%.md' OR lower(path) LIKE '%.pdf' OR lower(path) LIKE '%.doc%' OR lower(path) LIKE '%.xls%' OR lower(path) LIKE '%.csv')) OR (?2='other' AND lower(path) NOT LIKE '%.jpg' AND lower(path) NOT LIKE '%.jpeg' AND lower(path) NOT LIKE '%.png' AND lower(path) NOT LIKE '%.gif' AND lower(path) NOT LIKE '%.webp' AND lower(path) NOT LIKE '%.tif' AND lower(path) NOT LIKE '%.bmp' AND lower(path) NOT LIKE '%.txt' AND lower(path) NOT LIKE '%.md' AND lower(path) NOT LIKE '%.pdf' AND lower(path) NOT LIKE '%.doc%' AND lower(path) NOT LIKE '%.xls%' AND lower(path) NOT LIKE '%.csv'))";
        let sql_where = if self.consolidated() {
            format!(
                "EXISTS(SELECT 1 FROM consolidated_paths p WHERE p.group_id=result.id AND ({}))",
                sql_where.replace("path", "p.path")
            )
        } else {
            sql_where.into()
        };
        let matched = self.connection.query_row(
            &format!("SELECT COUNT(*) FROM ({filter}) result WHERE {sql_where}"),
            params![query.text, query.kind],
            |row| row.get::<_, i64>(0),
        )? as u64;
        let mut statement = self.connection.prepare(&format!(
            "SELECT * FROM ({filter}) result WHERE {sql_where} ORDER BY path,id LIMIT ?3 OFFSET ?4"
        ))?;
        let mut files = statement
            .query_map(
                params![
                    query.text,
                    query.kind,
                    query.limit.clamp(1, 500) as i64,
                    query.offset as i64
                ],
                row_candidate,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let selection = self.selection_read()?;
        for file in &mut files {
            if let Some(selection) = &selection {
                file.selected = selection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM selected WHERE id=?1)",
                    [file.id],
                    |row| row.get(0),
                )?;
            }
            if file.backend_version.is_empty() {
                file.backend_version = self
                    .manifest
                    .engines
                    .get(&file.backend)
                    .cloned()
                    .unwrap_or_else(|| "unknown (legacy scan)".into());
            }
        }
        let (selected, selected_bytes, selected_paths, selected_path_bytes) = if let Some(
            selection,
        ) = selection
        {
            let paths = if selection_has_paths(&selection)? {
                "paths"
            } else {
                "1"
            };
            selection.query_row(&format!("SELECT COUNT(*),COALESCE(SUM(bytes),0),COALESCE(SUM({paths}),0),COALESCE(SUM(bytes*{paths}),0) FROM selected"),[],
                |r|Ok((r.get::<_,i64>(0)? as u64,r.get::<_,i64>(1)? as u64,r.get::<_,i64>(2)? as u64,r.get::<_,i64>(3)? as u64)))?
        } else {
            (0, 0, 0, 0)
        };
        let total = self.count()?;
        Ok(Page {
            files,
            matched,
            total,
            selected,
            selected_bytes,
            selected_paths,
            selected_path_bytes,
            findings: self
                .manifest
                .consolidation
                .as_ref()
                .map_or(total, |c| c.findings),
            consolidated: self.consolidated(),
        })
    }

    fn selection_read(&self) -> Result<Option<Connection>> {
        let path = self.directory.join("selection.sqlite");
        if !path.exists() {
            return Ok(None);
        }
        safety::confined_artifact(&self.directory, "selection.sqlite")?;
        Ok(Some(Connection::open_with_flags(path, READONLY)?))
    }
    pub fn select(&self, ids: &[i64], selected: bool) -> Result<()> {
        let _selection_lock = if self.manifest.options.consolidate
            && matches!(
                self.manifest.state.as_str(),
                "preparing" | "scanning" | "consolidating"
            ) {
            let file = File::open(safety::confined_artifact(&self.directory, "tree.sqlite")?)?;
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(error.into());
                }
                return fail(
                    "Wait for the scan and final consolidation to finish before selecting files.",
                );
            }
            let current: Manifest = read_json(&safety::confined_artifact(
                &self.directory,
                "manifest.json",
            )?)?;
            if current.consolidation.is_some() && !self.consolidated() {
                return fail(
                    "The scan finished consolidating. Reload its results before selecting files.",
                );
            }
            Some(file)
        } else {
            None
        };
        // Selection is separate from the candidate tree; legacy trees are never migrated or rewritten.
        GuardedDir::existing(&self.directory)?
            .revalidate(&super::jobs::session_source(self)?, 0)?;
        let path = self.directory.join("selection.sqlite");
        let mut selection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        selection.execute_batch(
            "CREATE TABLE IF NOT EXISTS selected(id INTEGER PRIMARY KEY,bytes INTEGER NOT NULL,paths INTEGER NOT NULL DEFAULT 1)",
        )?;
        if !selection_has_paths(&selection)? {
            selection.execute_batch(
                "ALTER TABLE selected ADD COLUMN paths INTEGER NOT NULL DEFAULT 1",
            )?;
        }
        let transaction = selection.transaction()?;
        for &id in ids {
            if selected {
                let file = self.get(id)?;
                transaction.execute(
                    "INSERT OR IGNORE INTO selected(id,bytes,paths) VALUES(?1,?2,?3)",
                    params![id, file.bytes as i64, file.paths as i64],
                )?;
            } else {
                transaction.execute("DELETE FROM selected WHERE id=?1", [id])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }
    pub fn selected_page(&self, after: i64) -> Result<Vec<Candidate>> {
        let Some(selection) = self.selection_read()? else {
            return Ok(vec![]);
        };
        let mut statement =
            selection.prepare("SELECT id FROM selected WHERE id > ?1 ORDER BY id LIMIT 250")?;
        let ids = statement
            .query_map([after], |row| row.get::<_, i64>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter().map(|id| self.get(id)).collect()
    }
    pub fn log(&self, event: &str, data: &impl Serialize) -> Result<()> {
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .custom_flags(libc::O_NOFOLLOW)
            .mode(0o600)
            .open(self.directory.join("events.log"))?;
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({"time":timestamp(),"event":event,"data":data}),
        )?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(())
    }
    pub fn record_recovery(&self, record: &RecoveryRecord) -> Result<()> {
        GuardedDir::existing(&self.directory)?
            .revalidate(&super::jobs::session_source(self)?, 0)?;
        self.log("recovery", record)
    }
}

fn selection_has_paths(connection: &Connection) -> Result<bool> {
    Ok(connection
        .prepare("PRAGMA table_info(selected)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == "paths"))
}

pub fn list(root: &Path) -> Result<Vec<SavedScan>> {
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut scans = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let manifest = safety::confined_artifact(&entry.path(), "manifest.json")
            .and_then(|path| read_json::<Manifest>(&path));
        if let Ok(manifest) = manifest {
            scans.push(SavedScan {
                directory: entry.path(),
                manifest,
            });
        }
    }
    scans.sort_by_key(|scan| std::cmp::Reverse(scan.manifest.updated_at));
    Ok(scans)
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn native_projection() -> &'static str {
    "id,path,bytes,modified,backend,backend_version,locator,offset_bytes,length_bytes,artifact,carved,deleted,1 AS findings,1 AS paths,NULL AS sha256,NULL AS consolidation_error"
}
fn consolidated_projection() -> &'static str {
    "id,path,bytes,modified,backend,backend_version,locator,offset_bytes,length_bytes,artifact,carved,deleted,findings,paths,sha256,consolidation_error"
}
pub(super) fn row_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<Candidate> {
    Ok(Candidate {
        id: row.get(0)?,
        path: row.get(1)?,
        bytes: row.get::<_, i64>(2)?.max(0) as u64,
        modified: row.get(3)?,
        backend: row.get(4)?,
        backend_version: row.get(5)?,
        locator: row.get(6)?,
        offset_bytes: row.get::<_, i64>(7)?.max(0) as u64,
        length_bytes: row.get::<_, i64>(8)?.max(0) as u64,
        artifact: row.get(9)?,
        carved: row.get(10)?,
        deleted: row.get(11)?,
        selected: false,
        findings: row.get::<_, i64>(12)?.max(1) as u64,
        paths: row.get::<_, i64>(13)?.max(1) as u64,
        sha256: row.get(14)?,
        consolidation_error: row.get(15)?,
    })
}
fn legacy_projection(connection: &Connection) -> Result<(String, String)> {
    for table in ["candidates", "files", "entries"] {
        let columns: BTreeSet<String> = connection
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |row| row.get(1))?
            .collect::<std::result::Result<_, _>>()?;
        if !columns.contains("id") || !columns.contains("path") {
            continue;
        }
        let choose = |name: &str, alternate: &str, default: &str| -> String {
            if columns.contains(name) {
                name.into()
            } else if !alternate.is_empty() && columns.contains(alternate) {
                format!(
                    "CAST({alternate} AS {}) AS {name}",
                    if name == "locator" { "TEXT" } else { "INTEGER" }
                )
            } else {
                format!("{default} AS {name}")
            }
        };
        let projection = [
            "id".into(),
            "path".into(),
            choose("bytes", "size", "0"),
            choose("modified", "mtime", "0"),
            choose("backend", "", "'tsk'"),
            choose("backend_version", "", "''"),
            choose("locator", "inode", "''"),
            choose("offset_bytes", "partition_offset", "0"),
            choose("length_bytes", "partition_length", "0"),
            choose("artifact", "", "NULL"),
            choose("carved", "", "0"),
            choose("deleted", "", "1"),
            "1 AS findings".into(),
            "1 AS paths".into(),
            "NULL AS sha256".into(),
            "NULL AS consolidation_error".into(),
        ]
        .join(",");
        return Ok((table.into(), projection));
    }
    fail("This legacy tree has no supported candidate table. It was left unchanged.")
}

pub fn compare_recorded(first: &Store, second: &Store) -> Result<String> {
    if first.manifest.source.fingerprint != second.manifest.source.fingerprint {
        return fail("The saved scans refer to different source fingerprints.");
    }
    if first.manifest.engines != second.manifest.engines {
        return Ok(format!(
            "Recorded engine versions differ. Counts are not directly comparable.\n\nCurrent: {:?}\nSelected: {:?}",
            first.manifest.engines, second.manifest.engines
        ));
    }
    Ok(format!(
        "Source fingerprints and recorded engine versions match.\nCurrent scan: {} candidates\nSelected scan: {} candidates\n\nCurrent settings: {:?}\nSelected settings: {:?}",
        first.count()?,
        second.count()?,
        first.manifest.options.mode,
        second.manifest.options.mode
    ))
}
