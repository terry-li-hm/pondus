use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

const BUNDLED_ALIASES: &str = include_str!("../models.toml");

#[derive(Debug, Deserialize)]
struct AliasEntry {
    canonical: String,
    #[serde(default)]
    aliases: Vec<String>,
}

pub struct AliasMap {
    /// source_name → canonical_name
    to_canonical: HashMap<String, String>,
}

impl AliasMap {
    pub fn load(override_path: Option<&str>) -> Result<Self> {
        let mut to_canonical = HashMap::new();

        // Load bundled aliases
        Self::parse_into(BUNDLED_ALIASES, &mut to_canonical)?;

        // Load user override if it exists
        if let Some(path) = override_path {
            let p = PathBuf::from(path);
            if p.exists() {
                let content = std::fs::read_to_string(&p)?;
                Self::parse_into(&content, &mut to_canonical)?;
            }
        } else {
            // Check default user override location
            let default_override = dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("pondus")
                .join("models.toml");
            if default_override.exists() {
                let content = std::fs::read_to_string(&default_override)?;
                Self::parse_into(&content, &mut to_canonical)?;
            }
        }

        Ok(Self { to_canonical })
    }

    fn parse_into(toml_str: &str, map: &mut HashMap<String, String>) -> Result<()> {
        let entries: HashMap<String, AliasEntry> = toml::from_str(toml_str)?;
        for (_, entry) in entries {
            let canonical = entry.canonical.to_lowercase();
            // Map canonical to itself
            map.insert(canonical.clone(), canonical.clone());
            // Map each alias to canonical
            for alias in &entry.aliases {
                map.insert(alias.to_lowercase(), canonical.clone());
            }
        }
        Ok(())
    }

    /// Resolve a user-provided model name to its canonical form.
    /// Returns the input lowercased if no alias match found.
    pub fn resolve(&self, name: &str) -> String {
        let lower = name.to_lowercase();
        self.to_canonical
            .get(&lower)
            .cloned()
            .unwrap_or(lower)
    }

    /// Check if a source-specific model name matches a canonical name.
    pub fn matches(&self, source_name: &str, canonical: &str) -> bool {
        self.resolve(source_name) == canonical.to_lowercase()
    }
}
