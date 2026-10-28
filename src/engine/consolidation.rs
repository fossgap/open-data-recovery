//! Final, atomic content grouping. Raw findings are never removed or rewritten.
use super::{
    Error, Result,
    adapters::{Adapter, Recoverer},
    fail,
    process::Control,
    safety::{self, GuardedDir},
    store::{Store, native_projection, row_candidate},
    types::{Candidate, Conflict, Consolidation, Event},
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata},
    io::Read,
    os::unix::fs::MetadataExt,
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}
impl Stamp {
    fn of(meta: &Metadata) -> Result<Self> {
        if !meta.is_file() {
            return fail("Consolidation content is not a regular file.");
        }
        Ok(Self {
            device: meta.dev(),
            inode: meta.ino(),
            bytes: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })
    }
}

struct Content {
    relative: String,
    hash: String,
    stamp: Stamp,
    extracted: bool,
}

struct Finalizer<'a> {
    store: &'a Store,
    input: &'a File,
    cache: &'a GuardedDir,
    control: &'a Control,
    buffer: Vec<u8>,
    other: Vec<u8>,
    summary: Consolidation,
}

impl Finalizer<'_> {
    fn open(&self, relative: &str) -> Result<File> {
        Ok(File::open(safety::confined_artifact(
            &self.store.directory,
            relative,
        )?)?)
    }
    fn content(&mut self, candidate: &Candidate) -> Result<Content> {
        let extracted = candidate.artifact.is_none();
        let relative = if let Some(relative) = &candidate.artifact {
            relative.clone()
        } else {
            self.cache.check_space(candidate.bytes)?;
            let extension = Path::new(&candidate.path)
                .extension()
                .and_then(|e| e.to_str())
                .filter(|e| {
                    !e.is_empty() && e.len() <= 20 && e.bytes().all(|b| b.is_ascii_alphanumeric())
                })
                .unwrap_or("bin");
            let name = format!("{}.{}", candidate.id, extension);
            let Some((file, path)) = self.cache.create_file(&name, false, Conflict::Skip)? else {
                return fail("Unexpected existing consolidation cache file.");
            };
            let result = Adapter(candidate.backend.clone())
                .recover(
                    self.input,
                    candidate,
                    &file,
                    &self.cache.path,
                    super::options::sector(
                        &self.store.manifest.source,
                        &self.store.manifest.options,
                    ),
                    &self.store.manifest.options.interpretation.filesystem,
                    self.control,
                )
                .and_then(|()| Ok(file.sync_all()?));
            if let Err(error) = result {
                let _ = fs::remove_file(path);
                return Err(error);
            }
            format!("artifacts/consolidation/{name}")
        };
        let result = (|| {
            let mut file = self.open(&relative)?;
            let stamp = Stamp::of(&file.metadata()?)?;
            if stamp.bytes != candidate.bytes {
                return fail("Extracted length differs from the finding's recorded length.");
            }
            let mut hasher = Sha256::new();
            loop {
                self.control.checkpoint()?;
                let n = file.read(&mut self.buffer)?;
                if n == 0 {
                    break;
                }
                hasher.update(&self.buffer[..n]);
            }
            if stamp != Stamp::of(&file.metadata()?)? {
                return fail("Content changed while being hashed.");
            }
            Ok(Content {
                relative: relative.clone(),
                hash: format!("{:x}", hasher.finalize()),
                stamp,
                extracted,
            })
        })();
        if result.is_err() && extracted {
            let _ = fs::remove_file(self.store.directory.join(relative));
        }
        result
    }
    fn identical(&mut self, existing: &str, stamp: &Stamp, candidate: &Content) -> Result<bool> {
        let (mut a, mut b) = (self.open(existing)?, self.open(&candidate.relative)?);
        if Stamp::of(&a.metadata()?)? != *stamp || Stamp::of(&b.metadata()?)? != candidate.stamp {
            return fail("Consolidation content changed during verification.");
        }
        if stamp.bytes != candidate.stamp.bytes {
            return Ok(false);
        }
        let mut remaining = stamp.bytes;
        let mut equal = true;
        while remaining > 0 {
            self.control.checkpoint()?;
            let n = remaining.min(self.buffer.len() as u64) as usize;
            a.read_exact(&mut self.buffer[..n])?;
            b.read_exact(&mut self.other[..n])?;
            if self.buffer[..n] != self.other[..n] {
                equal = false;
                break;
            }
            remaining -= n as u64;
        }
        if Stamp::of(&a.metadata()?)? != *stamp || Stamp::of(&b.metadata()?)? != candidate.stamp {
            return fail("Consolidation content changed during comparison.");
        }
        Ok(equal)
    }
    fn singleton(
        &mut self,
        candidate: &Candidate,
        content: Option<&Content>,
        error: Option<String>,
    ) -> Result<i64> {
        self.store.connection.execute("INSERT INTO consolidated_groups(id,representative_id,bytes,artifact,sha256,stamp,consolidation_error) VALUES(?1,?1,?2,?3,?4,?5,?6)",
            params![candidate.id,candidate.bytes as i64,
                content.map(|c|c.relative.as_str()).or(candidate.artifact.as_deref()),
                content.map(|c|c.hash.as_str()),content.map(|c|serde_json::to_string(&c.stamp)).transpose()?,error])?;
        self.summary.groups += 1;
        if let Some(content) = content.filter(|c| c.extracted) {
            self.summary.cached_bytes += content.stamp.bytes;
        }
        Ok(candidate.id)
    }
    fn add(&mut self, candidate: Candidate, unique_size: bool) -> Result<()> {
        self.control.checkpoint()?;
        self.cache
            .check_space(if !unique_size && candidate.artifact.is_none() {
                candidate.bytes
            } else {
                0
            })?;
        let symbolic = self
            .store
            .metadata(candidate.id)?
            .is_some_and(|m| m.symbolic);
        let group = if unique_size || symbolic {
            self.singleton(&candidate, None, None)?
        } else {
            match self.content(&candidate) {
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(error) => {
                    // Disk exhaustion/directory replacement aborts the whole
                    // transaction. An unreadable finding stays separate.
                    self.cache.check_space(if candidate.artifact.is_none() {
                        candidate.bytes
                    } else {
                        0
                    })?;
                    self.summary.unverified += 1;
                    self.singleton(
                        &candidate,
                        None,
                        Some(error.to_string().chars().take(2000).collect()),
                    )?
                }
                Ok(content) => {
                    let mut after = 0;
                    let mut found = None;
                    loop {
                        let previous: Option<(i64,String,String)> = self.store.connection.query_row(
                            "SELECT id,artifact,stamp FROM consolidated_groups WHERE bytes=?1 AND sha256=?2 AND id>?3 ORDER BY id LIMIT 1",
                            params![candidate.bytes as i64,content.hash,after],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
                        let Some((id, path, stamp)) = previous else {
                            break;
                        };
                        after = id;
                        if self.identical(&path, &serde_json::from_str(&stamp)?, &content)? {
                            found = Some(id);
                            break;
                        }
                    }
                    if let Some(id) = found {
                        if content.extracted {
                            fs::remove_file(self.store.directory.join(&content.relative))?;
                        }
                        id
                    } else {
                        self.singleton(&candidate, Some(&content), None)?
                    }
                }
            }
        };
        let db = &self.store.connection;
        db.execute(
            "INSERT INTO consolidated_members VALUES(?1,?2)",
            params![candidate.id, group],
        )?;
        db.execute(
            "INSERT OR IGNORE INTO consolidated_paths VALUES(?1,?2,?3)",
            params![group, candidate.path, candidate.id],
        )?;
        let new_path = db.changes();
        // Prefer original TSK metadata, then another metadata engine, then a
        // carved name. Content storage is independent of the displayed name.
        let rank = |backend: &str, carved: bool| (carved, backend != "tsk");
        let (backend,carved): (String,bool) = db.query_row(
            "SELECT c.backend,c.carved FROM candidates c JOIN consolidated_groups g ON c.id=g.representative_id WHERE g.id=?1",[group],|r|Ok((r.get(0)?,r.get(1)?)))?;
        if rank(&candidate.backend, candidate.carved) < rank(&backend, carved) {
            db.execute(
                "UPDATE consolidated_groups SET representative_id=?1 WHERE id=?2",
                params![candidate.id, group],
            )?;
        }
        db.execute(
            "UPDATE consolidated_groups SET findings=findings+1,paths=paths+?1 WHERE id=?2",
            params![new_path as i64, group],
        )?;
        self.summary.findings += 1;
        Ok(())
    }
}

/// Runs only on a new scan, before announcing completion. Publication is a
/// single database commit followed by the atomic manifest update. Interrupted
/// scans retain their complete raw catalogue, without a partially grouped view.
pub fn run(
    store: &mut Store,
    input: &File,
    control: &Control,
    notify: &dyn Fn(Event),
) -> Result<()> {
    let began = Instant::now();
    control.checkpoint()?;
    store.manifest.state = "consolidating".into();
    store.save()?;
    notify(Event::Stage {
        text: "Consolidating identical files…".into(),
        fraction: Some(0.0),
    });
    let root = GuardedDir::existing(&store.directory)?;
    root.revalidate(&store.manifest.source, 0)?;
    let cache_path = store.directory.join("artifacts/consolidation");
    fs::create_dir(&cache_path)?;
    let cache = GuardedDir::existing(&cache_path)?;
    cache.revalidate(&store.manifest.source, 0)?;
    let result = (|| -> Result<Consolidation> {
        store.connection.execute_batch("BEGIN IMMEDIATE;
            CREATE TABLE consolidated_groups(id INTEGER PRIMARY KEY,representative_id INTEGER NOT NULL,bytes INTEGER NOT NULL,artifact TEXT,sha256 TEXT,stamp TEXT,findings INTEGER NOT NULL DEFAULT 0,paths INTEGER NOT NULL DEFAULT 0,consolidation_error TEXT);
            CREATE INDEX consolidated_hash ON consolidated_groups(bytes,sha256,id);
            CREATE INDEX consolidated_representative ON consolidated_groups(representative_id);
            CREATE TABLE consolidated_members(candidate_id INTEGER PRIMARY KEY,group_id INTEGER NOT NULL);
            CREATE INDEX consolidated_members_group ON consolidated_members(group_id,candidate_id);
            CREATE TABLE consolidated_paths(group_id INTEGER NOT NULL,path TEXT NOT NULL,candidate_id INTEGER NOT NULL,PRIMARY KEY(group_id,path)) WITHOUT ROWID;
            CREATE VIEW consolidated_candidates AS SELECT g.id,c.path,c.bytes,c.modified,c.backend,c.backend_version,c.locator,c.offset_bytes,c.length_bytes,COALESCE(g.artifact,c.artifact) AS artifact,c.carved,c.deleted,g.findings,g.paths,g.sha256,g.consolidation_error FROM consolidated_groups g JOIN candidates c ON c.id=g.representative_id;")?;
        let total = store.count()?;
        let mut worker = Finalizer {
            store,
            input,
            cache: &cache,
            control,
            buffer: vec![0; 1024 * 1024],
            other: vec![0; 1024 * 1024],
            summary: Consolidation::default(),
        };
        let mut sizes = store
            .connection
            .prepare("SELECT bytes,COUNT(*) FROM candidates GROUP BY bytes ORDER BY bytes")?;
        let mut buckets = sizes.query([])?;
        let mut updated = Instant::now() - Duration::from_secs(1);
        while let Some(bucket) = buckets.next()? {
            let bytes: i64 = bucket.get(0)?;
            let unique_size = bucket.get::<_, i64>(1)? == 1;
            let (mut after_kind, mut after_id) = (-1_i64, 0_i64);
            loop {
                let mut statement = store.connection.prepare(&format!(
                    "SELECT {} FROM candidates WHERE bytes=?1 AND ((artifact IS NULL),id)>(?2,?3) ORDER BY (artifact IS NULL),id LIMIT 128",native_projection()))?;
                let files = statement
                    .query_map(params![bytes, after_kind, after_id], row_candidate)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if files.is_empty() {
                    break;
                }
                for file in files {
                    after_kind = file.artifact.is_none() as i64;
                    after_id = file.id;
                    worker.add(file, unique_size)?;
                    if updated.elapsed() >= Duration::from_millis(100) {
                        notify(Event::Stage {
                            text: format!(
                                "Consolidating · {} of {total} findings checked",
                                worker.summary.findings
                            ),
                            fraction: Some(worker.summary.findings as f64 / total.max(1) as f64),
                        });
                        updated = Instant::now();
                    }
                }
            }
        }
        // A replaced or edited artifact must never become a published canonical
        // copy. Each content file is checked again before committing the view.
        let mut statement = store
            .connection
            .prepare("SELECT artifact,stamp FROM consolidated_groups WHERE stamp IS NOT NULL")?;
        let mut contents = statement.query([])?;
        while let Some(row) = contents.next()? {
            control.checkpoint()?;
            let path: String = row.get(0)?;
            let stamp: Stamp = serde_json::from_str(&row.get::<_, String>(1)?)?;
            if Stamp::of(&worker.open(&path)?.metadata()?)? != stamp {
                return fail(
                    "An artifact changed during consolidation. Original findings were retained.",
                );
            }
        }
        cache.check_space(0)?;
        File::open(&cache.path)?.sync_all()?;
        File::open(store.directory.join("artifacts"))?.sync_all()?;
        control.checkpoint()?;
        Ok(worker.summary)
    })();
    match result {
        Ok(mut summary) => {
            if let Err(error) = store.connection.execute_batch("COMMIT") {
                let _ = store.connection.execute_batch("ROLLBACK");
                if cache.check_identity().is_ok() {
                    let _ = fs::remove_dir_all(&cache_path);
                }
                return Err(error.into());
            }
            if summary.unverified > 0 {
                store.manifest.errors.push(format!("Consolidation could not verify {} finding(s); they remain separate. Inspect their finding details.",summary.unverified));
            }
            summary.duration_millis = began.elapsed().as_millis().min(u64::MAX as u128) as u64;
            store.publish_consolidation(summary.clone())?;
            store.log("consolidated", &summary)?;
            notify(Event::Candidates(summary.groups));
            notify(Event::Stage {
                text: format!(
                    "Consolidated {} findings into {} {}",
                    summary.findings,
                    summary.groups,
                    if summary.groups == 1 {
                        "group"
                    } else {
                        "groups"
                    }
                ),
                fraction: Some(1.0),
            });
            Ok(())
        }
        Err(error) => {
            let _ = store.connection.execute_batch("ROLLBACK");
            // These are only new temporary/cache files owned by this stage.
            // Existing engine artifacts and the original findings are untouched.
            if cache.check_identity().is_ok() {
                let _ = fs::remove_dir_all(&cache_path);
            }
            notify(Event::Message(
                "Consolidation did not finish; original findings remain available.".into(),
            ));
            Err(error)
        }
    }
}
