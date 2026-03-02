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

pub enum MatchKind {
    Exact,
    Alias,
    Prefix,
    NoMatch,
}

pub struct AliasMatch {
    pub source_name: String,
    pub source_model_name: String,
    pub canonical: String,
    pub match_kind: MatchKind,
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

    #[cfg(test)]
    fn from_toml(toml_str: &str) -> Self {
        let mut to_canonical = HashMap::new();
        Self::parse_into(toml_str, &mut to_canonical).unwrap();
        Self { to_canonical }
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
        self.resolve_with_kind(name).0
    }

    fn resolve_with_kind(&self, name: &str) -> (String, MatchKind) {
        let lower = name.to_lowercase();

        // Exact match first
        if let Some(canonical) = self.to_canonical.get(&lower) {
            if canonical == &lower {
                return (canonical.clone(), MatchKind::Exact);
            }
            return (canonical.clone(), MatchKind::Alias);
        }

        // Prefix match: "gpt-5-(high)" → "gpt-5" if next char after prefix is '-' or '('
        // But "gpt-5.2" should NOT match "gpt-5" (dot means different version)
        if let Some(canonical) = self.prefix_match(&lower) {
            return (canonical, MatchKind::Prefix);
        }

        (lower, MatchKind::NoMatch)
    }

    /// Check if a source-specific model name matches a canonical name.
    pub fn matches(&self, source_name: &str, canonical: &str) -> bool {
        self.resolve(source_name) == canonical.to_lowercase()
    }

    pub fn explain(
        &self,
        source_name: &str,
        source_model_name: &str,
        canonical: &str,
    ) -> AliasMatch {
        let (_, match_kind) = self.resolve_with_kind(source_model_name);
        AliasMatch {
            source_name: source_name.to_string(),
            source_model_name: source_model_name.to_string(),
            canonical: canonical.to_string(),
            match_kind,
        }
    }

    /// Try prefix matching against known canonical names and aliases.
    /// Matches if name starts with a known alias followed by a qualifier suffix.
    /// Returns the longest matching canonical name to avoid short-prefix collisions.
    ///
    /// Allowed suffixes:
    ///   `(` or ` ` — parenthetical qualifier, e.g. `claude-opus-4.5 (reasoning)` → ok
    ///   `-` followed by a digit — date/version suffix, e.g. `gpt-5.2-2025-04-16` → ok
    ///   `-` followed by a letter — model variant, e.g. `o3-pro`, `o3-mini` → NOT ok
    ///
    /// The letter-after-hyphen rule prevents short names like `o3` from swallowing
    /// genuinely distinct models (`o3-pro`, `o3-mini`). Add explicit aliases in
    /// models.toml for deployment-name patterns like `gpt-5.2-chat-latest`.
    fn prefix_match(&self, lower_name: &str) -> Option<String> {
        let mut best: Option<(usize, String)> = None;

        for (alias, canonical) in &self.to_canonical {
            if lower_name.len() > alias.len() && lower_name.starts_with(alias.as_str()) {
                let next_char = lower_name.as_bytes()[alias.len()];
                let allowed = match next_char {
                    b'(' | b' ' => true,
                    b'-' => {
                        // Only allow if the char after the hyphen is a digit
                        // (date/version suffix), not a letter (model variant)
                        lower_name
                            .as_bytes()
                            .get(alias.len() + 1)
                            .is_some_and(|c| c.is_ascii_digit())
                    }
                    _ => false,
                };
                if allowed {
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

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TOML: &str = r#"
[gpt-o3]
canonical = "o3"
aliases = ["o3", "openai/o3"]

[gpt-o3-pro]
canonical = "o3-pro"
aliases = ["o3-pro", "o3-pro (high)", "o3-pro-2025-06-10"]

[gpt-o3-mini]
canonical = "o3-mini"
aliases = ["o3-mini", "o3-mini (high)"]

[gpt-5_2]
canonical = "gpt-5.2"
aliases = ["GPT-5.2", "gpt-5.2-chat-latest"]
"#;

    fn map() -> AliasMap {
        AliasMap::from_toml(TEST_TOML)
    }

    // --- Exact and alias matches ---

    #[test]
    fn exact_match() {
        assert_eq!(map().resolve("o3"), "o3");
    }

    #[test]
    fn alias_match() {
        assert_eq!(map().resolve("openai/o3"), "o3");
    }

    // --- Parenthetical suffix: always allowed ---

    #[test]
    fn paren_suffix_resolves() {
        // "o3 (high)" not in aliases → should prefix-match o3 via space boundary
        assert_eq!(map().resolve("o3 (high)"), "o3");
    }

    #[test]
    fn paren_direct_suffix_resolves() {
        // "gpt-5.2(xhigh)" — '(' immediately after alias
        assert_eq!(map().resolve("gpt-5.2(xhigh)"), "gpt-5.2");
    }

    // --- Hyphen + digit: allowed (date/version suffix) ---

    #[test]
    fn hyphen_digit_suffix_resolves() {
        // "o3-2025-04-16" — '-' then digit → ok
        assert_eq!(map().resolve("o3-2025-04-16"), "o3");
    }

    #[test]
    fn hyphen_digit_suffix_on_longer_alias() {
        // "gpt-5.2-chat-latest-20260210" — "gpt-5.2-chat-latest" is in aliases,
        // then "-20260210" is a digit suffix → resolves via alias+prefix
        assert_eq!(map().resolve("gpt-5.2-chat-latest-20260210"), "gpt-5.2");
    }

    // --- Hyphen + letter: NOT allowed (model variant) ---

    #[test]
    fn o3_does_not_match_o3_pro() {
        // o3-pro has its own canonical — must NOT fall through to o3
        assert_eq!(map().resolve("o3-pro"), "o3-pro");
    }

    #[test]
    fn o3_does_not_match_o3_mini() {
        assert_eq!(map().resolve("o3-mini"), "o3-mini");
    }

    #[test]
    fn unknown_hyphen_letter_suffix_is_no_match() {
        // "o3-turbo" not in aliases, and hyphen-letter prefix match is blocked
        // → falls back to NoMatch, returns input lowercased
        assert_eq!(map().resolve("o3-turbo"), "o3-turbo");
    }

    #[test]
    fn gpt52_chat_latest_resolves_via_alias() {
        assert_eq!(map().resolve("gpt-5.2-chat-latest"), "gpt-5.2");
    }

    // --- No-match ---

    #[test]
    fn completely_unknown_returns_itself() {
        assert_eq!(map().resolve("unknown-model-xyz"), "unknown-model-xyz");
    }
}
