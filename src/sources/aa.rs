use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

pub struct ArtificialAnalysis;

impl Source for ArtificialAnalysis {
    fn name(&self) -> &str {
        "artificial-analysis"
    }

    fn fetch(&self, config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get("artificial-analysis") {
            return Ok(self.parse_cached(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        // Try API with key first
        if let Some(api_key) = config.aa_api_key() {
            return self.fetch_api(api_key, cache);
        }

        // Fallback: scrape leaderboard via agent-browser
        self.fetch_scrape(config, cache)
    }
}

impl ArtificialAnalysis {
    fn fetch_api(&self, api_key: &str, cache: &Cache) -> Result<SourceResult> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        let response = client
            .get("https://artificialanalysis.ai/api/v2/data/llms/models")
            .header("x-api-key", api_key)
            .send()
            .context("Failed to fetch from Artificial Analysis API")?;

        if !response.status().is_success() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Error(format!("HTTP {}", response.status())),
                scores: vec![],
            });
        }

        let data = response.json::<serde_json::Value>()?;

        // Parse API response into cache format
        let mut ranked: Vec<(String, f64)> = Vec::new();
        if let Some(models) = data.as_array() {
            for model in models {
                let name = match model.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                let score = match model.get("intelligence_index").and_then(|v| v.as_f64()) {
                    Some(s) => s,
                    None => continue,
                };
                ranked.push((name, score));
            }
        }

        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let cached_rows: Vec<serde_json::Value> = ranked
            .iter()
            .map(|(name, score)| {
                serde_json::json!({
                    "source_model_name": name,
                    "score": score,
                })
            })
            .collect();

        let cache_value = serde_json::json!({ "scores": cached_rows });
        cache.set("artificial-analysis", &cache_value)?;

        Ok(self.parse_cached(&cache_value, Some(Utc::now()), SourceStatus::Ok))
    }

    fn fetch_scrape(&self, config: &Config, cache: &Cache) -> Result<SourceResult> {
        let agent_browser = config.agent_browser_path();

        if let Err(err) = run_agent_browser(
            agent_browser,
            &["open", "https://artificialanalysis.ai/leaderboards/models"],
        ) {
            return Ok(map_command_error(self.name(), "open", err));
        }

        let _ = run_agent_browser(agent_browser, &["wait", "3000"]);

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
                    "Failed to parse any model scores from AA leaderboard page".into(),
                ),
                scores: vec![],
            });
        }

        parsed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let cached_rows: Vec<serde_json::Value> = parsed
            .iter()
            .map(|(name, score)| {
                serde_json::json!({
                    "source_model_name": name,
                    "score": score,
                })
            })
            .collect();

        let cache_value = serde_json::json!({ "scores": cached_rows });
        cache.set("artificial-analysis", &cache_value)?;

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
                            .and_then(|v| v.as_str())?
                            .to_string();
                        let score = entry.get("score").and_then(|v| v.as_f64())?;
                        Some((name, score))
                    })
                    .collect()
            })
            .unwrap_or_default();

        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let scores = rows
            .into_iter()
            .enumerate()
            .map(|(idx, (source_model_name, score))| {
                let rank = (idx + 1) as u32;
                let mut metrics = HashMap::new();
                metrics.insert("intelligence_index".into(), MetricValue::Float(score));
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
            status: SourceStatus::Error(format!("AA scrape failed at {}: {}", step, err)),
            scores: vec![],
        }
    }
}

/// Parse the agent-browser accessibility tree snapshot for AA leaderboard.
///
/// The table rows look like:
/// ```text
/// - row "Gemini 3.1 Pro Preview 1m Google logo Google 57 $4.50 91 35.19 ...":
///   - cell "Gemini 3.1 Pro Preview" [ref=...]:
///   - cell "1m" [ref=...]
///   - cell "Google logo Google" [ref=...]:
///   - cell "57" [ref=...]       ← intelligence index
///   ...
/// ```
///
/// We extract (model_name, intelligence_index) from each row's cells.
fn parse_scores_from_text(text: &str) -> Vec<(String, f64)> {
    let mut results: HashMap<String, f64> = HashMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Look for data row lines (skip header rows)
        if trimmed.starts_with("- row \"") && trimmed.contains("Model Providers") {
            // Collect cells from subsequent lines
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

            // Table columns: 0=Model, 1=Context, 2=Creator, 3=Intelligence Index, 4=Price, 5=Speed, 6=Latency, 7=Links
            if cells.len() >= 4 {
                let model_name = &cells[0];
                if let Ok(score) = cells[3].parse::<f64>()
                    && !model_name.is_empty()
                    && (1.0..=100.0).contains(&score)
                {
                    results
                        .entry(model_name.clone())
                        .and_modify(|existing| {
                            if score > *existing {
                                *existing = score;
                            }
                        })
                        .or_insert(score);
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
