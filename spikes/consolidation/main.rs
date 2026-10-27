//! Disposable benchmark only; this binary is not linked into the application.
//! Reuses the production sandboxed child-process runner for realistic icat cost.
#![allow(dead_code)]

#[path = "../../src/engine/process.rs"]
mod process;

use rusqlite::{Connection, OpenFlags, params};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("Cancelled")]
    Cancelled,
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Message(message.into()))
}

// The only sibling function referenced by the unchanged production runner.
mod safety {
    pub fn free_bytes(path: &std::path::Path) -> super::Result<u64> {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|e| super::Error::Message(e.to_string()))?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(stat.f_bavail.saturating_mul(stat.f_frsize))
    }
}

struct Candidate {
    id: i64,
    bytes: u64,
    backend: String,
    locator: String,
    offset: u64,
    artifact: Option<String>,
}
struct Group {
    representative: i64,
    bytes: u64,
    hash: Option<String>,
    content: Option<PathBuf>,
    members: Vec<i64>,
}

fn digest(path: &Path) -> Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn identical(a: &Path, b: &Path, bytes: &mut u64) -> Result<bool> {
    let (mut a, mut b) = (File::open(a)?, File::open(b)?);
    if a.metadata()?.len() != b.metadata()?.len() {
        return Ok(false);
    }
    let (mut ab, mut bb) = (vec![0; 1024 * 1024], vec![0; 1024 * 1024]);
    let mut remaining = a.metadata()?.len();
    while remaining > 0 {
        let n = remaining.min(ab.len() as u64) as usize;
        a.read_exact(&mut ab[..n])?;
        b.read_exact(&mut bb[..n])?;
        *bytes += 2 * n as u64;
        if ab[..n] != bb[..n] {
            return Ok(false);
        }
        remaining -= n as u64;
    }
    Ok(true)
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() < 3 || args.len() > 4 {
        return fail(
            "Usage: odr-consolidation-spike FINISHED_SESSION NEW_OUTPUT_DIRECTORY [all|reuse]",
        );
    }
    let reuse_cache = args.get(3).is_some_and(|arg| arg == "reuse");
    if args
        .get(3)
        .is_some_and(|arg| arg != "all" && arg != "reuse")
    {
        return fail("Unknown cache policy");
    }
    let began = Instant::now();
    let session = fs::canonicalize(&args[1])?;
    let output = PathBuf::from(&args[2]);
    // Refuse existing destinations, real block devices, and unfinished scans.
    let manifest: serde_json::Value =
        serde_json::from_reader(File::open(session.join("manifest.json"))?)?;
    if manifest["complete"] != true || !manifest["scan_input"].is_null() {
        return fail("Spike requires a completed scan of an unnormalized regular test image");
    }
    let source = PathBuf::from(manifest["source"]["path"].as_str().unwrap());
    if !source.is_file() {
        return fail("Spike accepts regular image files only");
    }
    let input = File::open(&source)?; // O_RDONLY, inherited by production sandbox runner.
    let sector = manifest["source"]["sector_size"].as_u64().unwrap();
    let db_uri = format!(
        "file:{}?mode=ro&immutable=1",
        session.join("tree.sqlite").display()
    );
    let db = Connection::open_with_flags(
        db_uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let mut query = db.prepare(
        "SELECT id,bytes,backend,locator,offset_bytes,artifact FROM candidates ORDER BY id",
    )?;
    let candidates: Vec<Candidate> = query
        .query_map([], |r| {
            Ok(Candidate {
                id: r.get(0)?,
                bytes: r.get::<_, i64>(1)?.max(0) as u64,
                backend: r.get(2)?,
                locator: r.get(3)?,
                offset: r.get::<_, i64>(4)?.max(0) as u64,
                artifact: r.get(5)?,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    fs::create_dir(&output)?;
    let cache = output.join("cache");
    fs::create_dir(&cache)?;
    let mut sizes: BTreeMap<u64, Vec<&Candidate>> = BTreeMap::new();
    let tsk_ids: HashSet<i64> = candidates
        .iter()
        .filter(|c| c.backend == "tsk")
        .map(|c| c.id)
        .collect();
    for candidate in &candidates {
        sizes.entry(candidate.bytes).or_default().push(candidate);
    }
    let mut groups: Vec<Group> = vec![];
    let (mut skipped, mut extracted, mut cache_bytes, mut hashed_bytes, mut compared_bytes) =
        (0, 0, 0_u64, 0_u64, 0_u64);
    let (mut extract_seconds, mut hash_seconds, mut compare_seconds) = (0.0, 0.0, 0.0);
    let (mut retained_cache, mut peak_cache) = (0_u64, 0_u64);
    let (mut retained_allocated, mut peak_allocated) = (0_u64, 0_u64);
    let control = process::Control::default();
    for bucket in sizes.values() {
        if bucket.len() == 1 {
            let c = bucket[0];
            groups.push(Group {
                representative: c.id,
                bytes: c.bytes,
                hash: None,
                content: None,
                members: vec![c.id],
            });
            skipped += 1;
            continue;
        }
        let mut hashes: HashMap<String, Vec<usize>> = HashMap::new();
        let mut ordered = bucket.clone();
        if reuse_cache {
            // Prefer existing content. Presentation can still prefer a TSK path.
            ordered.sort_by_key(|c| (c.artifact.is_none(), c.id));
        }
        for c in ordered {
            let content = if let Some(artifact) = &c.artifact {
                let content = fs::canonicalize(session.join(artifact))?;
                if !content.starts_with(&session) || !content.is_file() {
                    return fail("Artifact escaped the fixture session");
                }
                content
            } else {
                if c.backend != "tsk"
                    || c.offset % sector != 0
                    || c.locator.is_empty()
                    || !c.locator.chars().all(|x| x.is_ascii_digit() || x == '-')
                {
                    return fail("Unexpected lazy candidate");
                }
                let started = Instant::now();
                let path = cache.join(format!("{}.bin", c.id));
                let file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .read(true)
                    .open(&path)?;
                // Same command and supervisor as Adapter::recover in the application.
                let mut spec = process::Spec::new("icat", &cache);
                spec.source(&input)?;
                spec.output = Some(file.try_clone()?);
                spec.args(&["-r", "-b"])
                    .arg(sector.to_string())
                    .arg("-o")
                    .arg((c.offset / sector).to_string())
                    .arg("/proc/self/fd/3")
                    .arg(&c.locator);
                let outcome = process::run(spec, &control, |_| Ok(()))?;
                if !outcome.status.success() {
                    return fail(format!("icat failed: {}", outcome.stderr));
                }
                file.sync_all()?;
                extracted += 1;
                cache_bytes += file.metadata()?.len();
                retained_cache += file.metadata()?.len();
                retained_allocated += file.metadata()?.blocks() * 512;
                peak_cache = peak_cache.max(retained_cache);
                peak_allocated = peak_allocated.max(retained_allocated);
                extract_seconds += started.elapsed().as_secs_f64();
                path
            };
            if fs::metadata(&content)?.len() != c.bytes {
                return fail(format!("Recovered length mismatch for {}", c.id));
            }
            let started = Instant::now();
            let hash = digest(&content)?;
            hash_seconds += started.elapsed().as_secs_f64();
            hashed_bytes += c.bytes;
            let mut matched = None;
            if let Some(indices) = hashes.get(&hash) {
                for &i in indices {
                    let started = Instant::now();
                    let equal = identical(
                        groups[i].content.as_ref().unwrap(),
                        &content,
                        &mut compared_bytes,
                    )?;
                    compare_seconds += started.elapsed().as_secs_f64();
                    if equal {
                        matched = Some(i);
                        break;
                    }
                }
            }
            if let Some(i) = matched {
                groups[i].members.push(c.id);
                if reuse_cache && c.artifact.is_none() {
                    let already_has_tsk = tsk_ids.contains(&groups[i].representative);
                    if !already_has_tsk {
                        groups[i].representative = c.id;
                    }
                    let meta = fs::metadata(&content)?;
                    retained_cache -= meta.len();
                    retained_allocated -= meta.blocks() * 512;
                    fs::remove_file(&content)?;
                }
            } else {
                hashes.entry(hash.clone()).or_default().push(groups.len());
                groups.push(Group {
                    representative: c.id,
                    bytes: c.bytes,
                    hash: Some(hash),
                    content: Some(content),
                    members: vec![c.id],
                });
            }
        }
    }
    let started = Instant::now();
    let pending = output.join("groups.pending.sqlite");
    let mut result = Connection::open(&pending)?;
    result.execute_batch("PRAGMA synchronous=FULL;
        CREATE TABLE groups(id INTEGER PRIMARY KEY,representative INTEGER NOT NULL,bytes INTEGER NOT NULL,sha256 TEXT,content TEXT);
        CREATE TABLE members(candidate_id INTEGER PRIMARY KEY,group_id INTEGER NOT NULL);
        CREATE INDEX group_members ON members(group_id);")?;
    let transaction = result.transaction()?;
    for (index, group) in groups.iter().enumerate() {
        transaction.execute(
            "INSERT INTO groups VALUES(?1,?2,?3,?4,?5)",
            params![
                index as i64,
                group.representative,
                group.bytes as i64,
                group.hash,
                group
                    .content
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
            ],
        )?;
        for id in &group.members {
            transaction.execute(
                "INSERT INTO members VALUES(?1,?2)",
                params![id, index as i64],
            )?;
        }
    }
    transaction.commit()?;
    result.close().map_err(|(_, e)| Error::Sql(e))?;
    fs::rename(pending, output.join("groups.sqlite"))?;
    File::open(&output)?.sync_all()?;
    let persist_seconds = started.elapsed().as_secs_f64();
    println!(
        "{}",
        json!({
            "seconds": began.elapsed().as_secs_f64(), "candidates": candidates.len(),
            "groups": groups.len(), "duplicate_findings": candidates.len() - groups.len(),
            "unique_size_skipped": skipped, "extracted_files": extracted, "cache_bytes": cache_bytes,
        "cache_retained_bytes": retained_cache, "cache_peak_bytes": peak_cache,
        "cache_retained_allocated_bytes": retained_allocated, "cache_peak_allocated_bytes": peak_allocated,
            "hashed_bytes": hashed_bytes, "compared_bytes": compared_bytes,
            "extraction_seconds": extract_seconds, "hash_seconds": hash_seconds,
            "comparison_seconds": compare_seconds, "persist_seconds": persist_seconds,
        "policy": if reuse_cache { "reuse-confirmed-identical-content; preserve-original-artifacts" }
            else { "retain-all-extracted-candidates; preserve-original-artifacts" }
        })
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
