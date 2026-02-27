use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;

const CACHE_KEY: &str = "terminal-bench";
const HF_API_URL: &str = "https://huggingface.co/api/datasets/sabhay/terminal-bench-2-leaderboard";

pub struct TerminalBench;

impl Source for TerminalBench {
    fn name(&self) -> &str {
        "terminal-bench"
    }

    fn fetch(&self, _config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get(CACHE_KEY) {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(fetched_at),
                status: SourceStatus::Cached,
                scores: parse_scores(&cached_data),
            });
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        // Fetch dataset metadata — siblings list contains all file paths
        let response = client
            .get(HF_API_URL)
            .send()
            .context("Failed to fetch Terminal-Bench dataset metadata")?;

        if !response.status().is_success() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Error(format!("HTTP {}", response.status())),
                scores: vec![],
            });
        }

        let data: serde_json::Value = response.json()
            .context("Failed to parse Terminal-Bench metadata")?;

        // Extract model/agent names from file paths
        // Path pattern: submissions/terminal-bench/2.0/{Agent}__{Model}/{date}/{task}/result.json
        // We count successful result.json files per model as a proxy for completeness
        let scores = extract_from_siblings(&data);

        if scores.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Unavailable,
                scores: vec![],
            });
        }

        // Cache the metadata (not individual results)
        cache.set(CACHE_KEY, &data)?;

        Ok(SourceResult {
            source: self.name().into(),
            fetched_at: Some(Utc::now()),
            status: SourceStatus::Ok,
            scores,
        })
    }
}

fn extract_from_siblings(data: &serde_json::Value) -> Vec<ModelScore> {
    let Some(siblings) = data.get("siblings").and_then(|v| v.as_array()) else {
        return vec![];
    };

    // Count result.json files per agent/model combination
    let mut model_counts: HashMap<String, u32> = HashMap::new();

    for sibling in siblings {
        let Some(filename) = sibling.get("rfilename").and_then(|v| v.as_str()) else {
            continue;
        };

        if !filename.ends_with("result.json") {
            continue;
        }

        // Parse path: submissions/terminal-bench/2.0/{Agent}__{Model}/{date}/{task}/result.json
        let parts: Vec<&str> = filename.split('/').collect();
        if parts.len() >= 4 {
            let agent_model = parts[3]; // e.g. "Ante__Gemini-3-Pro-Preview"
            *model_counts.entry(agent_model.to_string()).or_default() += 1;
        }
    }

    // Convert to ModelScores
    let mut scores: Vec<ModelScore> = model_counts
        .into_iter()
        .map(|(agent_model, count)| {
            // Split "Agent__Model" into parts
            let display_name = agent_model.replace("__", " / ");
            let canonical = agent_model.to_lowercase().replace("__", "/");

            let mut metrics = HashMap::new();
            metrics.insert("tasks_completed".into(), MetricValue::Int(count as i64));

            ModelScore {
                model: canonical,
                source_model_name: display_name,
                metrics,
                rank: None,
            }
        })
        .collect();

    // Sort by tasks_completed descending
    scores.sort_by(|a, b| {
        let a_count = get_int(&a.metrics, "tasks_completed");
        let b_count = get_int(&b.metrics, "tasks_completed");
        b_count.cmp(&a_count)
    });

    for (i, score) in scores.iter_mut().enumerate() {
        score.rank = Some((i + 1) as u32);
    }

    scores
}

fn parse_scores(data: &serde_json::Value) -> Vec<ModelScore> {
    extract_from_siblings(data)
}

fn get_int(metrics: &HashMap<String, MetricValue>, key: &str) -> i64 {
    match metrics.get(key) {
        Some(MetricValue::Int(i)) => *i,
        _ => 0,
    }
}
