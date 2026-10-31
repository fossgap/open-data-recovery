pub mod adapters;
pub mod advanced;
#[cfg(test)]
mod advanced_tests;
pub mod consolidation;
#[cfg(test)]
mod consolidation_tests;
pub mod discovery;
#[cfg(test)]
mod discovery_tests;
pub mod imaging;
pub mod jobs;
pub mod metadata;
pub mod options;
pub mod plan;
pub mod process;
pub mod restore;
pub mod safety;
pub mod sessions;
#[cfg(test)]
mod sessions_tests;
pub mod sources;
#[cfg(test)]
mod storage_tests;
pub mod store;
#[cfg(test)]
mod tests;
pub mod types;

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("Session database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("Session metadata: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Operation cancelled; partial results have been retained.")]
    Cancelled,
}
pub type Result<T> = std::result::Result<T, Error>;
pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Message(message.into()))
}
pub fn timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > 4 * 1024 * 1024 {
        return fail("Session metadata exceeds the 4 MiB limit.");
    }
    let mut data = Vec::new();
    file.take(4 * 1024 * 1024 + 1).read_to_end(&mut data)?;
    Ok(serde_json::from_slice(&data)?)
}
