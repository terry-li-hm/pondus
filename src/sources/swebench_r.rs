use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::process::Command;

pub struct SweRebench;

impl Source for SweRebench {
    fn name(&self) -> &str {
        "swe-rebench"
    }

    fn fetch(&self, config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get("swe-rebench") {
            return Ok(self.parse_cached(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        let agent_browser = config.agent_browser_path();

        // 1. agent-browser open <url>
        if let Err(err) = run_agent_browser(agent_browser, &["open", "https://swe-rebench.com/"]) {
            return Ok(map_command_error(self.name(), "open", err));
        }

        // 2. Wait for page to load, then get accessibility tree text
        let _ = run_agent_browser(agent_browser, &["wait", "2000"]);

        let page_text = match run_agent_browser(agent_browser, &["snapshot"]) {
            Ok(text) => text,
            Err(err) => return Ok(map_command_error(self.name(), "snapshot", err)),
        };

        let mut parsed = parse_scores_from_text(&page_text);

        if parsed.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(Utc::now()),
                status: SourceStatus::Error(
                    "Failed to parse any model scores from SWE-rebench page output".into(),
                ),
                scores: vec![],
            });
        }

        // Sort by score descending
        parsed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Cache the parsed data
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
        cache.set("swe-rebench", &cache_value)?;

        Ok(self.parse_cached(&cache_value, Some(Utc::now()), SourceStatus::Ok))
    }
}

impl SweRebench {
    fn parse_cached(
        &self,
        data: &serde_json::Value,
        fetched_at: Option<DateTime<Utc>>,
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

        // Ensure sorted by score
        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let scores = rows
            .into_iter()
            .enumerate()
            .map(|(idx, (source_model_name, score))| {
                let rank = (idx + 1) as u32;
                let mut metrics = HashMap::new();
                metrics.insert("resolve_rate".into(), MetricValue::Float(score));
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
            status: SourceStatus::Error(format!("SWE-rebench scrape failed at {}: {}", step, err)),
            scores: vec![],
        }
    }
}

/// Parse SWE-rebench leaderboard from agent-browser accessibility snapshot.
///
/// Table rows look like:
/// ```text
/// - row "1 Claude Code 52.9% 1.06% 70.8% $3.50 2,088,226 92.1%":
///   - cell "1" [ref=...]
///   - cell "Claude Code" [ref=...]
///   - cell "52.9%" [ref=...]       ← resolved rate
///   ...
/// ```
///
/// Columns: Rank, Model, Resolved Rate (%), SEM (±), Pass@5 (%), Cost, Tokens, Cached%.
fn parse_scores_from_text(text: &str) -> Vec<(String, f64)> {
    let mut results: HashMap<String, f64> = HashMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Look for data rows — they start with a rank number and contain "%"
        if trimmed.starts_with("- row \"") && trimmed.contains('%') {
            let mut cells: Vec<String> = Vec::new();
            let mut j = i + 1;
            while j < lines.len() {
                let cell_line = lines[j].trim();
                if cell_line.starts_with("- cell \"") {
                    if let Some(val) = extract_cell_value(cell_line) {
                        cells.push(val);
                    }
                } else if cell_line.starts_with("- row ") {
                    break;
                }
                j += 1;
            }

            // Cells: 0=Rank, 1=Model, 2=Resolved Rate
            if cells.len() >= 3 {
                let model_name = &cells[1];
                // Parse "52.9%" → 52.9
                let score_str = cells[2].trim_end_matches('%');
                if let Ok(score) = score_str.parse::<f64>()
                    && !model_name.is_empty()
                    && model_name.chars().any(|c| c.is_ascii_alphabetic())
                    && model_name != "Model"
                {
                    results.entry(model_name.clone()).or_insert(score);
                }
            }

            i = j;
        } else {
            i += 1;
        }
    }

    results.into_iter().collect()
}

/// Extract the quoted value from a cell line like `- cell "some value" [ref=...]:`
fn extract_cell_value(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

fn normalize_model_name(name: &str) -> String {
    name.to_lowercase().replace([' ', '_'], "-")
}
