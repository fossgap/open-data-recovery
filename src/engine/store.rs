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
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

const READONLY: OpenFlags = OpenFlags::SQLITE_OPEN_READ_ONLY
    .union(OpenFlags::SQLITE_OPEN_NO_MUTEX)
    .union(OpenFlags::SQLITE_OPEN_NOFOLLOW);
pub struct Store {
    pub directory: PathBuf,
    pub manifest: Manifest,
    connection: Connection,
    projection: String,
    table: String,
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
        let manifest = Manifest {
            schema_version: 1,
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
        };
        let mut store = Self {
            directory,
            manifest,
            connection,
            projection: native_projection().into(),
            table: "candidates".into(),
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
        if manifest.schema_version > 1 {
            return fail("This saved scan uses a newer session format.");
        }
        let db = safety::confined_artifact(directory, "tree.sqlite")?;
        let connection = Connection::open_with_flags(db, READONLY)?;
        let (table, projection) = legacy_projection(&connection)?;
        Ok(Self {
            directory: directory.into(),
            manifest,
            connection,
            projection,
            table,
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
    pub fn page(&self, query: &Query) -> Result<Page> {
        let filter = format!("SELECT {} FROM {}", self.projection, self.table);
        let sql_where = "instr(lower(path),lower(?1)) > 0 AND (?2='' OR (?2='image' AND lower(path) GLOB '*.*' AND lower(substr(path,instr(path,'.')+1)) IS NOT NULL AND (lower(path) LIKE '%.jpg' OR lower(path) LIKE '%.jpeg' OR lower(path) LIKE '%.png' OR lower(path) LIKE '%.gif' OR lower(path) LIKE '%.webp' OR lower(path) LIKE '%.tif' OR lower(path) LIKE '%.bmp')) OR (?2='document' AND (lower(path) LIKE '%.txt' OR lower(path) LIKE '%.md' OR lower(path) LIKE '%.pdf' OR lower(path) LIKE '%.doc%' OR lower(path) LIKE '%.xls%' OR lower(path) LIKE '%.csv')) OR (?2='other' AND lower(path) NOT LIKE '%.jpg' AND lower(path) NOT LIKE '%.jpeg' AND lower(path) NOT LIKE '%.png' AND lower(path) NOT LIKE '%.gif' AND lower(path) NOT LIKE '%.webp' AND lower(path) NOT LIKE '%.tif' AND lower(path) NOT LIKE '%.bmp' AND lower(path) NOT LIKE '%.txt' AND lower(path) NOT LIKE '%.md' AND lower(path) NOT LIKE '%.pdf' AND lower(path) NOT LIKE '%.doc%' AND lower(path) NOT LIKE '%.xls%' AND lower(path) NOT LIKE '%.csv'))";
        let matched = self.connection.query_row(
            &format!("SELECT COUNT(*) FROM ({filter}) WHERE {sql_where}"),
            params![query.text, query.kind],
            |row| row.get::<_, i64>(0),
        )? as u64;
        let mut statement = self.connection.prepare(&format!(
            "SELECT * FROM ({filter}) WHERE {sql_where} ORDER BY path,id LIMIT ?3 OFFSET ?4"
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
        let (selected, selected_bytes) = if let Some(selection) = selection {
            selection.query_row(
                "SELECT COUNT(*),COALESCE(SUM(bytes),0) FROM selected",
                [],
                |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
            )?
        } else {
            (0, 0)
        };
        Ok(Page {
            files,
            matched,
            total: self.count()?,
            selected,
            selected_bytes,
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
            "CREATE TABLE IF NOT EXISTS selected(id INTEGER PRIMARY KEY,bytes INTEGER NOT NULL)",
        )?;
        let transaction = selection.transaction()?;
        for &id in ids {
            if selected {
                let file = self.get(id)?;
                transaction.execute(
                    "INSERT OR IGNORE INTO selected VALUES(?1,?2)",
                    params![id, file.bytes as i64],
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

fn native_projection() -> &'static str {
    "id,path,bytes,modified,backend,backend_version,locator,offset_bytes,length_bytes,artifact,carved,deleted"
}
fn row_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<Candidate> {
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
