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

        if let Err(err) =
            run_agent_browser(agent_browser, &["open", "https://scale.com/leaderboard"])
        {
            return Ok(map_command_error(self.name(), "open", err));
        }

        // Wait for page to load, then get accessibility tree text
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
                    "Failed to parse any model scores from SEAL page output".into(),
                ),
                scores: vec![],
            });
        }

        parsed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

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

        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

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

/// Parse SEAL leaderboard from agent-browser accessibility snapshot.
///
/// The page shows benchmark cards as link elements containing flattened text:
/// ```text
/// link "MCP Atlas Evaluating ... 1 claude-opus-4-5 62.30±1.76 1 gpt-5.2 60.57±1.62 ..."
/// ```
///
/// Scores use `SCORE±ERROR` format. We extract model-score pairs from each card
/// and average across benchmarks per model.
fn parse_scores_from_text(text: &str) -> Vec<(String, f64)> {
    let mut model_scores: HashMap<String, Vec<f64>> = HashMap::new();

    for line in text.lines() {
        let trimmed = line.trim();

        // Find link elements that contain benchmark cards (they all have "View Full Ranking")
        if !trimmed.contains("View Full Ranking") {
            continue;
        }

        // Extract the quoted link text
        let link_text = if let Some(start) = trimmed.find('"') {
            let rest = &trimmed[start + 1..];
            if let Some(end) = rest.rfind("View Full Ranking") {
                &rest[..end]
            } else {
                continue;
            }
        } else {
            continue;
        };

        // Parse model-score pairs from the link text
        // Pattern: RANK MODEL_NAME [NEW] SCORE±ERROR
        for (model, score) in extract_model_scores(link_text) {
            model_scores.entry(model).or_default().push(score);
        }
    }

    // Average scores across benchmarks per model
    model_scores
        .into_iter()
        .map(|(model, scores)| {
            let avg = scores.iter().sum::<f64>() / scores.len() as f64;
            (model, avg)
        })
        .collect()
}

/// Extract (model_name, score) pairs from a SEAL card's flattened text.
///
/// Forward parser: walks tokens left-to-right. After each `SCORE±ERROR` token,
/// the next small integer is the rank for the next model. Tokens between rank
/// and score (excluding "NEW") form the model name.
fn extract_model_scores(text: &str) -> Vec<(String, f64)> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut results = Vec::new();

    // First, find all score positions (tokens containing ±)
    let score_positions: Vec<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(_, t)| t.contains('±'))
        .map(|(i, _)| i)
        .collect();

    if score_positions.is_empty() {
        return results;
    }

    // For each score, find the rank that precedes it.
    // The rank for the first model is the first small integer before the first ±.
    // For subsequent models, the rank is the first small integer after the previous ±.
    for (si, &score_pos) in score_positions.iter().enumerate() {
        let score_str = tokens[score_pos].split('±').next().unwrap_or("");
        let score: f64 = match score_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Search window for rank: after previous score (or start) up to this score
        let search_start = if si == 0 {
            0
        } else {
            score_positions[si - 1] + 1
        };

        // Find the rank (first small integer in the search window)
        let rank_pos = (search_start..score_pos).find(|&j| {
            tokens[j].parse::<u32>().is_ok_and(|n| n <= 500)
        });

        let name_start = match rank_pos {
            Some(rp) => rp + 1,
            None => search_start,
        };

        // Collect name tokens between rank and score, skipping "NEW"
        let name: String = tokens[name_start..score_pos]
            .iter()
            .filter(|&&t| t != "NEW")
            .copied()
            .collect::<Vec<_>>()
            .join(" ");

        // Strip trailing asterisks (footnote artifacts from accessibility tree)
        let name = name.trim_end_matches('*').trim().to_string();
        if name.len() >= 2 && name.chars().any(|c| c.is_ascii_alphabetic()) {
            results.push((name, score));
        }
    }

    results
}

fn normalize_model_name(name: &str) -> String {
    name.to_lowercase().replace([' ', '_'], "-")
}
