use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub sources: HashMap<String, SourceConfig>,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub alias: AliasConfig,
}

#[derive(Debug, Deserialize, Default)]
pub struct SourceConfig {
    pub api_key: Option<String>,
    pub agent_browser_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_ttl")]
    pub ttl_hours: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            ttl_hours: default_ttl(),
        }
    }
}

fn default_ttl() -> u64 {
    24
}

#[derive(Debug, Deserialize, Default)]
pub struct AliasConfig {
    pub path: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    pub fn agent_browser_path(&self) -> &str {
        self.sources
            .get("seal")
            .or_else(|| self.sources.get("swe-rebench"))
            .and_then(|s| s.agent_browser_path.as_deref())
            .unwrap_or("agent-browser")
    }

    pub fn aa_api_key(&self) -> Option<&str> {
        self.sources
            .get("artificial-analysis")
            .and_then(|s| s.api_key.as_deref())
    }
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pondus")
        .join("config.toml")
}
