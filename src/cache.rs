use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
struct CacheEntry {
    fetched_at: DateTime<Utc>,
    ttl_hours: u64,
    data: serde_json::Value,
}

pub struct Cache {
    dir: PathBuf,
    ttl_hours: u64,
}

impl Cache {
    pub fn new(ttl_hours: u64) -> Self {
        let dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("pondus");
        Self { dir, ttl_hours }
    }

    pub fn get(&self, source: &str) -> Option<(DateTime<Utc>, serde_json::Value)> {
        let path = self.dir.join(format!("{source}.json"));
        let content = fs::read_to_string(&path).ok()?;
        let entry: CacheEntry = serde_json::from_str(&content).ok()?;

        let age = Utc::now() - entry.fetched_at;
        if age.num_hours() < entry.ttl_hours as i64 {
            Some((entry.fetched_at, entry.data))
        } else {
            None
        }
    }

    pub fn set(&self, source: &str, data: &serde_json::Value) -> Result<()> {
        fs::create_dir_all(&self.dir).context("Failed to create cache directory")?;

        let entry = CacheEntry {
            fetched_at: Utc::now(),
            ttl_hours: self.ttl_hours,
            data: data.clone(),
        };

        let json = serde_json::to_string_pretty(&entry)?;

        // Atomic write: temp file → fsync → rename
        let path = self.dir.join(format!("{source}.json"));
        let tmp_path = self.dir.join(format!("{source}.json.tmp"));

        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp_path, &path)?;

        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        if self.dir.exists() {
            for entry in fs::read_dir(&self.dir)? {
                let entry = entry?;
                if entry.path().extension().is_some_and(|e| e == "json") {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
    }
}
