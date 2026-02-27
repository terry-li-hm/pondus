use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::process::Command;

pub struct Seal;

impl Source for Seal {
    fn name(&self) -> &str {
        "seal"
    }

    fn fetch(&self, config: &Config, cache: &Cache) -> Result<SourceResult> {
        if let Some((fetched_at, cached_data)) = cache.get("seal") {
            return Ok(self.parse_cached(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        let agent_browser = config.agent_browser_path();

        if let Err(err) = run_agent_browser(agent_browser, &["open", "https://scale.com/leaderboard"]) {
            return Ok(map_command_error(self.name(), "open", err));
        }

        if let Err(err) = run_agent_browser(agent_browser, &["snapshot"]) {
            return Ok(map_command_error(self.name(), "snapshot", err));
        }

        let page_text = match run_agent_browser(agent_browser, &["read_page"]) {
            Ok(text) => text,
            Err(err) => return Ok(map_command_error(self.name(), "read_page", err)),
        };

        let mut parsed = parse_scores_from_text(&page_text);

        if parsed.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(Utc::now()),
                status: SourceStatus::Error("Failed to parse any model scores from SEAL page output".into()),
                scores: vec![],
            });
        }

        parsed.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let cached_rows: Vec<serde_json::Value> = parsed
            .iter()
            .map(|(source_model_name, score)| {
                serde_json::json!({
                    "source_model_name": source_model_name,
                    "score": score,
                })
            })
            .collect();

        let cache_value = serde_json::json!({ "scores": cached_rows });
        cache.set("seal", &cache_value)?;

        Ok(self.parse_cached(&cache_value, Some(Utc::now()), SourceStatus::Ok))
    }
}

impl Seal {
    fn parse_cached(
        &self,
        data: &serde_json::Value,
        fetched_at: Option<chrono::DateTime<Utc>>,
        status: SourceStatus,
    ) -> SourceResult {
        let mut rows: Vec<(String, f64)> = data
            .get("scores")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|entry| {
                        let source_model_name = entry
                            .get("source_model_name")
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned)?;
                        let score = entry.get("score").and_then(|v| v.as_f64())?;
                        Some((source_model_name, score))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        rows.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let scores = rows
            .into_iter()
            .enumerate()
            .map(|(idx, (source_model_name, score))| {
                let rank = (idx + 1) as u32;
                let mut metrics = HashMap::new();
                metrics.insert("overall_score".into(), MetricValue::Float(score));
                metrics.insert("rank".into(), MetricValue::Int(rank as i64));

                ModelScore {
                    model: normalize_model_name(&source_model_name),
                    source_model_name,
                    metrics,
                    rank: Some(rank),
                }
            })
            .collect();

        SourceResult {
            source: self.name().into(),
            fetched_at,
            status,
            scores,
        }
    }
}

fn run_agent_browser(agent_browser_path: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(agent_browser_path)
        .args(args)
        .output()
        .with_context(|| {
            format!(
                "Failed to execute {} {}",
                agent_browser_path,
                args.join(" ")
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if !stderr.is_empty() {
            stderr
        } else if !stdout.is_empty() {
            stdout
        } else {
            format!("Exit status: {}", output.status)
        };

        anyhow::bail!("agent-browser {} failed: {}", args.join(" "), details);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn map_command_error(source: &str, step: &str, err: anyhow::Error) -> SourceResult {
    let unavailable = err
        .root_cause()
        .downcast_ref::<std::io::Error>()
        .map(|io_err| io_err.kind() == std::io::ErrorKind::NotFound)
        .unwrap_or(false);

    if unavailable {
        SourceResult {
            source: source.into(),
            fetched_at: None,
            status: SourceStatus::Unavailable,
            scores: vec![],
        }
    } else {
        SourceResult {
            source: source.into(),
            fetched_at: None,
            status: SourceStatus::Error(format!("SEAL scrape failed at {}: {}", step, err)),
            scores: vec![],
        }
    }
}

fn parse_scores_from_text(text: &str) -> Vec<(String, f64)> {
    let mut best_by_model: HashMap<String, f64> = HashMap::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.len() < 4 || should_skip_line(trimmed) {
            continue;
        }

        if let Some((source_model_name, score)) = parse_line(trimmed) {
            best_by_model
                .entry(source_model_name)
                .and_modify(|existing| {
                    if score > *existing {
                        *existing = score;
                    }
                })
                .or_insert(score);
        }
    }

    best_by_model.into_iter().collect()
}

fn parse_line(line: &str) -> Option<(String, f64)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    let mut numeric_positions = Vec::new();
    for (idx, tok) in tokens.iter().enumerate() {
        if let Some(value) = parse_numeric_token(tok) {
            numeric_positions.push((idx, value));
        }
    }

    if numeric_positions.is_empty() {
        return None;
    }

    // Prefer the right-most percentage-like score (0..100); otherwise use right-most numeric token.
    let score_idx_and_val = numeric_positions
        .iter()
        .rev()
        .find(|(_, v)| (0.0..=100.0).contains(v))
        .or_else(|| numeric_positions.iter().rev().find(|(_, v)| (0.0..=10_000.0).contains(v)))?;

    let score_idx = score_idx_and_val.0;
    let score = score_idx_and_val.1;

    let model_start = if let Some(rank_val) = parse_numeric_token(tokens[0]) {
        if tokens.len() > 2 && rank_val.fract() == 0.0 && (1.0..=200.0).contains(&rank_val) {
            1
        } else {
            0
        }
    } else {
        0
    };

    if score_idx <= model_start {
        return None;
    }

    let model = tokens[model_start..score_idx].join(" ").trim().to_string();
    if !is_likely_model_name(&model) {
        return None;
    }

    Some((model, score))
}

fn parse_numeric_token(token: &str) -> Option<f64> {
    let cleaned = token
        .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '+')
        .trim_end_matches('%')
        .replace(',', "");

    if cleaned.is_empty() || !cleaned.chars().any(|c| c.is_ascii_digit()) {
        return None;
    }

    if cleaned.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }

    cleaned.parse::<f64>().ok()
}

fn should_skip_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    let skip_terms = [
        "leaderboard",
        "benchmark",
        "category",
        "categories",
        "updated",
        "loading",
        "search",
        "filter",
        "overall rank",
        "view",
        "share",
    ];

    skip_terms.iter().any(|term| lower.contains(term))
}

fn is_likely_model_name(name: &str) -> bool {
    if name.len() < 2 || name.len() > 120 {
        return false;
    }

    let lower = name.to_lowercase();
    let banned = ["overall", "composite", "score", "rank", "model"];
    if banned.iter().any(|word| lower == *word) {
        return false;
    }

    name.chars().any(|c| c.is_ascii_alphabetic())
}

fn normalize_model_name(name: &str) -> String {
    name.to_lowercase()
        .replace(' ', "-")
        .replace('_', "-")
}
