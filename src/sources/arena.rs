use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

pub struct Arena;

impl Source for Arena {
    fn name(&self) -> &str {
        "arena"
    }

    fn fetch(&self, config: &Config, cache: &Cache) -> Result<SourceResult> {
        if let Some((fetched_at, cached_data)) = cache.get("arena") {
            return Ok(self.parse_cached(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        // Primary: scrape arena.ai/leaderboard via agent-browser
        match self.fetch_scrape(config, cache) {
            Ok(result) if !result.scores.is_empty() => return Ok(result),
            _ => {}
        }

        // Fallback: community JSON mirror (may be stale)
        self.fetch_json(cache)
    }
}

impl Arena {
    fn fetch_scrape(&self, config: &Config, cache: &Cache) -> Result<SourceResult> {
        let agent_browser = config.agent_browser_path();

        if let Err(err) =
            run_agent_browser(agent_browser, &["open", "https://lmarena.ai/leaderboard"])
        {
            return Ok(map_command_error(self.name(), "open", err));
        }

        let _ = run_agent_browser(agent_browser, &["wait", "4000"]);

        let page_text = match run_agent_browser(agent_browser, &["snapshot"]) {
            Ok(text) => text,
            Err(err) => return Ok(map_command_error(self.name(), "snapshot", err)),
        };

        let parsed = parse_scores_from_snapshot(&page_text);

        if parsed.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(Utc::now()),
                status: SourceStatus::Error(
                    "Failed to parse any scores from Arena leaderboard".into(),
                ),
                scores: vec![],
            });
        }

        let cached_rows: Vec<serde_json::Value> = parsed
            .iter()
            .map(|(name, elo)| {
                serde_json::json!({
                    "source_model_name": name,
                    "elo_score": elo,
                })
            })
            .collect();

        let cache_value = serde_json::json!({ "scores": cached_rows });
        cache.set("arena", &cache_value)?;

        Ok(self.parse_cached(&cache_value, Some(Utc::now()), SourceStatus::Ok))
    }

    fn fetch_json(&self, cache: &Cache) -> Result<SourceResult> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;
        let response = client
            .get("https://raw.githubusercontent.com/nakasyou/lmarena-history/main/output/scores.json")
            .send()
            .context("Failed to fetch from Arena GitHub")?;

        if !response.status().is_success() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Error(format!("HTTP {}", response.status())),
                scores: vec![],
            });
        }

        let data = response.json::<serde_json::Value>()?;
        let scores = parse_json_response(&data);

        if scores.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(Utc::now()),
                status: SourceStatus::Error("Failed to parse Arena JSON".into()),
                scores: vec![],
            });
        }

        // Cache in the same format as scrape results
        let cached_rows: Vec<serde_json::Value> = scores
            .iter()
            .map(|(name, elo)| {
                serde_json::json!({
                    "source_model_name": name,
                    "elo_score": elo,
                })
            })
            .collect();

        let cache_value = serde_json::json!({ "scores": cached_rows });
        cache.set("arena", &cache_value)?;

        Ok(self.parse_cached(&cache_value, Some(Utc::now()), SourceStatus::Ok))
    }

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
                        let name = entry
                            .get("source_model_name")
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned)?;
                        let elo = entry.get("elo_score").and_then(|v| v.as_f64())?;
                        Some((name, elo))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let scores = rows
            .into_iter()
            .enumerate()
            .map(|(idx, (source_model_name, elo))| {
                let rank = (idx + 1) as u32;
                let mut metrics = HashMap::new();
                metrics.insert("elo_score".into(), MetricValue::Float(elo));
                metrics.insert("rank".into(), MetricValue::Int(rank as i64));

                ModelScore {
                    model: source_model_name.to_lowercase().replace([' ', '_'], "-"),
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

/// Parse Arena leaderboard from agent-browser accessibility snapshot.
///
/// The first table on the page is the "Text" leaderboard. Rows look like:
/// ```text
/// - row "1 Anthropic claude-opus-4-6-thinking 1503 6,583":
///   - cell "1" [ref=...]
///   - cell "Anthropic claude-opus-4-6-thinking" [ref=...]:
///     - link "claude-opus-4-6-thinking" [ref=...]:
///   - cell "1503" [ref=...]
///   - cell "6,583" [ref=...]
/// ```
///
/// We extract the model name from the link inside cell 1, and ELO from cell 2.
fn parse_scores_from_snapshot(text: &str) -> Vec<(String, f64)> {
    let mut results: HashMap<String, f64> = HashMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    let mut found_first_table = false;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Only parse rows from the first table (Text leaderboard)
        // Stop when we hit a second table or "View all" section
        if found_first_table && trimmed.starts_with("- link \"") && trimmed.contains("View all") {
            break;
        }

        if trimmed.starts_with("- row \"") && trimmed.contains("1503")
            || trimmed.starts_with("- row \"1 ")
        {
            found_first_table = true;
        }

        if found_first_table && trimmed.starts_with("- row \"") {
            let mut cells: Vec<String> = Vec::new();
            let mut model_link_name: Option<String> = None;
            let mut j = i + 1;

            while j < lines.len() {
                let cell_line = lines[j].trim();
                if cell_line.starts_with("- cell \"") {
                    if let Some(val) = extract_cell_value(cell_line) {
                        cells.push(val);
                    }
                } else if cell_line.starts_with("- link \"") && model_link_name.is_none() {
                    // The model name link is inside the second cell
                    if let Some(val) = extract_cell_value(cell_line)
                        && !val.starts_with("http")
                        && !val.is_empty()
                    {
                        model_link_name = Some(val);
                    }
                } else if cell_line.starts_with("- row ") {
                    break;
                }
                j += 1;
            }

            // Cells: 0=Rank, 1=Provider+Model, 2=ELO, 3=Votes
            if cells.len() >= 3 {
                // Prefer the link name (cleaner), fall back to cell text
                let model_name = model_link_name.unwrap_or_else(|| {
                    // Strip provider prefix from cell text
                    let cell = &cells[1];
                    // Common providers: "Anthropic ", "Bytedance ", etc.
                    cell.split_whitespace()
                        .skip_while(|t| {
                            t.chars().next().is_some_and(|c| c.is_uppercase())
                                && !t.contains('-')
                                && !t.contains('.')
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                });

                if let Ok(elo) = cells[2].parse::<f64>()
                    && elo > 500.0
                    && !model_name.is_empty()
                {
                    results.entry(model_name).or_insert(elo);
                }
            }

            i = j;
        } else {
            i += 1;
        }
    }

    results.into_iter().collect()
}

/// Parse the community JSON mirror (fallback).
fn parse_json_response(data: &serde_json::Value) -> Vec<(String, f64)> {
    let obj = match data.as_object() {
        Some(o) => o,
        None => return vec![],
    };

    let text_data = match obj
        .keys()
        .max()
        .and_then(|k| obj.get(k))
        .and_then(|d| d.get("text"))
    {
        Some(t) => t,
        None => return vec![],
    };

    let category = if text_data.get("overall").is_some() {
        "overall"
    } else if text_data.get("full_old").is_some() {
        "full_old"
    } else if let Some(first_category) = text_data.as_object().and_then(|o| o.keys().next()) {
        first_category.as_str()
    } else {
        return vec![];
    };

    text_data
        .get(category)
        .and_then(|c| c.as_object())
        .map(|models| {
            models
                .iter()
                .filter_map(|(name, score)| score.as_f64().map(|s| (name.clone(), s)))
                .collect()
        })
        .unwrap_or_default()
}

fn extract_cell_value(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
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
            status: SourceStatus::Error(format!("Arena scrape failed at {}: {}", step, err)),
            scores: vec![],
        }
    }
}
