use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

const AIDER_URL: &str = "https://raw.githubusercontent.com/Aider-AI/aider/main/aider/website/_data/polyglot_leaderboard.yml";

#[derive(Debug, Serialize, Deserialize)]
struct AiderEntry {
    model: String,
    #[serde(default)]
    pass_rate_1: Option<f64>,
    #[serde(default)]
    total_cost: Option<f64>,
    #[serde(default)]
    percent_cases_well_formed: Option<f64>,
}

pub struct Aider;

impl Source for Aider {
    fn name(&self) -> &str {
        "aider"
    }

    fn fetch(&self, _config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get("aider") {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(fetched_at),
                status: SourceStatus::Cached,
                scores: parse_scores(&cached_data),
            });
        }

        // Fetch YAML from GitHub
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;
        let response = client
            .get(AIDER_URL)
            .send()
            .context("Failed to fetch Aider leaderboard")?;

        if !response.status().is_success() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Error(format!("HTTP {}", response.status())),
                scores: vec![],
            });
        }

        let yaml_text = response.text().context("Failed to read Aider response")?;

        // Parse YAML into entries
        let entries: Vec<AiderEntry> =
            serde_yaml::from_str(&yaml_text).context("Failed to parse Aider YAML")?;

        // Convert to JSON Value for caching
        let data = serde_json::to_value(&entries)?;
        cache.set("aider", &data)?;

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

    let Some(entries) = data.as_array() else {
        return scores;
    };

    for entry in entries {
        let Some(model_name) = entry.get("model").and_then(|v| v.as_str()) else {
            continue;
        };

        let mut metrics = HashMap::new();

        if let Some(rate) = entry.get("pass_rate_1").and_then(|v| v.as_f64()) {
            metrics.insert("pass_rate_1".into(), MetricValue::Float(rate));
        }

        if let Some(cost) = entry.get("total_cost").and_then(|v| v.as_f64()) {
            metrics.insert("cost".into(), MetricValue::Float(cost));
        }

        if let Some(wf) = entry
            .get("percent_cases_well_formed")
            .and_then(|v| v.as_f64())
        {
            metrics.insert("percent_cases_well_formed".into(), MetricValue::Float(wf));
        }

        scores.push(ModelScore {
            model: model_name.to_lowercase(),
            source_model_name: model_name.to_string(),
            metrics,
            rank: None,
        });
    }

    // Sort by pass_rate_1 descending
    scores.sort_by(|a, b| {
        let a_rate = get_float(&a.metrics, "pass_rate_1");
        let b_rate = get_float(&b.metrics, "pass_rate_1");
        b_rate
            .partial_cmp(&a_rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (i, score) in scores.iter_mut().enumerate() {
        score.rank = Some((i + 1) as u32);
    }

    scores
}

fn get_float(metrics: &HashMap<String, MetricValue>, key: &str) -> f64 {
    match metrics.get(key) {
        Some(MetricValue::Float(f)) => *f,
        _ => 0.0,
    }
}
