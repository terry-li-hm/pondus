use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;

pub struct SweBench;

impl Source for SweBench {
    fn name(&self) -> &str {
        "swebench"
    }

    fn fetch(&self, _config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get("swebench") {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(fetched_at),
                status: SourceStatus::Cached,
                scores: parse_scores(&cached_data),
            });
        }

        // Fetch from GitHub raw JSON
        let url = "https://raw.githubusercontent.com/SWE-bench/swe-bench.github.io/master/data/leaderboards.json";
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;
        let response = client
            .get(url)
            .send()
            .context("Failed to fetch SWE-bench leaderboard data")?;

        if !response.status().is_success() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Error(format!("HTTP {}", response.status())),
                scores: vec![],
            });
        }

        let data: serde_json::Value = response.json().context("Failed to parse SWE-bench JSON")?;

        // Cache the raw response
        cache.set("swebench", &data)?;

        Ok(SourceResult {
            source: self.name().into(),
            fetched_at: Some(Utc::now()),
            status: SourceStatus::Ok,
            scores: parse_scores(&data),
        })
    }
}

fn parse_scores(data: &serde_json::Value) -> Vec<ModelScore> {
    let mut scores = Vec::new();

    // Try multiple JSON structures: {"leaderboards": [...]} or top-level array
    let entries = data
        .get("leaderboards")
        .or_else(|| data.get("results"))
        .and_then(|v| v.as_array())
        .or_else(|| data.as_array());

    let Some(entries) = entries else {
        return scores;
    };

    // For nested leaderboard structure, extract results from each
    for entry in entries {
        if let Some(results) = entry.get("results").and_then(|v| v.as_array()) {
            for result in results {
                if let Some(score) = extract_model_score(result) {
                    scores.push(score);
                }
            }
        } else {
            // Flat structure — each entry IS a result
            if let Some(score) = extract_model_score(entry) {
                scores.push(score);
            }
        }
    }

    // Sort by resolved_rate descending, assign ranks
    scores.sort_by(|a, b| {
        let a_rate = get_float(&a.metrics, "resolved_rate");
        let b_rate = get_float(&b.metrics, "resolved_rate");
        b_rate
            .partial_cmp(&a_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (i, score) in scores.iter_mut().enumerate() {
        score.rank = Some((i + 1) as u32);
    }

    scores
}

fn extract_model_score(result: &serde_json::Value) -> Option<ModelScore> {
    let name = result.get("name").and_then(|v| v.as_str())?;

    let mut metrics = HashMap::new();

    if let Some(rate) = result.get("resolved").and_then(|v| v.as_f64()) {
        metrics.insert("resolved_rate".into(), MetricValue::Float(rate));
    }

    if let Some(count) = result.get("resolved_count").and_then(|v| v.as_i64()) {
        metrics.insert("resolved_count".into(), MetricValue::Int(count));
    }

    if let Some(date) = result.get("date").and_then(|v| v.as_str()) {
        metrics.insert("date".into(), MetricValue::Text(date.to_string()));
    }

    Some(ModelScore {
        model: name.to_lowercase(),
        source_model_name: name.to_string(),
        metrics,
        rank: None,
    })
}

fn get_float(metrics: &HashMap<String, MetricValue>, key: &str) -> f64 {
    match metrics.get(key) {
        Some(MetricValue::Float(f)) => *f,
        _ => 0.0,
    }
}
