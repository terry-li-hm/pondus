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
    #[serde(skip)]
    pub aa_api_key: Option<String>,
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
        let mut config = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let mut config: Config = toml::from_str(&content)?;
            config.aa_api_key = aa_api_key_from_content(&content);
            config
        } else {
            Config::default()
        };

        if let Ok(env_api_key) = std::env::var("AA_API_KEY")
            && !env_api_key.trim().is_empty()
        {
            config.aa_api_key = Some(env_api_key);
        }

        Ok(config)
    }

    pub fn agent_browser_path(&self) -> &str {
        self.sources
            .get("seal")
            .or_else(|| self.sources.get("swe-rebench"))
            .and_then(|s| s.agent_browser_path.as_deref())
            .unwrap_or("agent-browser")
    }

    pub fn aa_api_key(&self) -> Option<&str> {
        self.aa_api_key.as_deref().or_else(|| {
            self.sources
                .get("artificial-analysis")
                .or_else(|| self.sources.get("artificial_analysis"))
                .and_then(|s| s.api_key.as_deref())
        })
    }
}

fn aa_api_key_from_content(content: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(content).ok()?;
    value
        .get("artificial-analysis")
        .and_then(|v| v.get("api_key"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pondus")
        .join("config.toml")
}
