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
    /// source_name → canonical_name (also used for prefix matching)
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

        // Exact match first
        if let Some(canonical) = self.to_canonical.get(&lower) {
            return canonical.clone();
        }

        // Prefix match: "gpt-5-(high)" → "gpt-5" if next char after prefix is '-' or '('
        // But "gpt-5.2" should NOT match "gpt-5" (dot means different version)
        if let Some(canonical) = self.prefix_match(&lower) {
            return canonical;
        }

        lower
    }

    /// Check if a source-specific model name matches a canonical name.
    pub fn matches(&self, source_name: &str, canonical: &str) -> bool {
        self.resolve(source_name) == canonical.to_lowercase()
    }

    /// Try prefix matching against known canonical names and aliases.
    /// Matches if name starts with a known name followed by '-' or '('.
    /// Returns the longest matching canonical name to avoid short-prefix collisions.
    fn prefix_match(&self, lower_name: &str) -> Option<String> {
        let mut best: Option<(usize, String)> = None;

        for (alias, canonical) in &self.to_canonical {
            if lower_name.len() > alias.len() && lower_name.starts_with(alias.as_str()) {
                let next_char = lower_name.as_bytes()[alias.len()];
                // Only match if followed by separator, not version dot
                if next_char == b'-' || next_char == b'(' || next_char == b' ' {
                    let len = alias.len();
                    if best.as_ref().is_none_or(|(best_len, _)| len > *best_len) {
                        best = Some((len, canonical.clone()));
                    }
                }
            }
        }

        best.map(|(_, canonical)| canonical)
    }
}
