use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;

pub struct LiveBench;

impl Source for LiveBench {
    fn name(&self) -> &str {
        "livebench"
    }

    fn fetch(&self, _config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get("livebench") {
            return Ok(self.parse_response(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        // Try to fetch from the primary JSON endpoint
        // Currently not available; LiveBench publishes results via HuggingFace parquet files
        // This endpoint is for future use when a public JSON leaderboard is available
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let json_url = "https://livebench.ai/api/leaderboard.json";
        match client.get(json_url).send() {
            Ok(response) => {
                if response.status().is_success() {
                    if let Ok(data) = response.json::<serde_json::Value>() {
                        // Cache the raw response
                        cache.set("livebench", &data)?;
                        return Ok(self.parse_response(&data, Some(Utc::now()), SourceStatus::Ok));
                    }
                }
            }
            Err(_) => {
                // Primary endpoint not available, fall through to alternative
            }
        }

        // Fall back to HuggingFace parquet endpoint
        // Note: This requires external tooling to convert parquet to JSON.
        // For now, return a descriptive error with instructions.
        let hf_url = "https://huggingface.co/api/datasets/livebench/model_judgment/parquet";

        match client.get(hf_url).send() {
            Ok(response) => {
                if response.status().is_success() {
                    if let Ok(_parquet_info) = response.json::<serde_json::Value>() {
                        // The response contains parquet file URLs, but we need to convert them
                        // This is a placeholder; full implementation would:
                        // 1. Download the parquet file
                        // 2. Use a parquet library (polars, arrow) to read and convert to JSON
                        // 3. Parse the structured results

                        return Ok(SourceResult {
                            source: self.name().into(),
                            fetched_at: None,
                            status: SourceStatus::Error(
                                "LiveBench data is stored in parquet format. \
                                 To use this source, either: \
                                 (1) Implement parquet deserialization with polars/arrow dependencies, \
                                 (2) Use the livebench Python package to export results as JSON, \
                                 (3) Wait for livebench.ai to publish a public JSON API."
                                    .into(),
                            ),
                            scores: vec![],
                        });
                    }
                }
            }
            Err(_) => {}
        }

        // Both endpoints failed
        Ok(SourceResult {
            source: self.name().into(),
            fetched_at: None,
            status: SourceStatus::Unavailable,
            scores: vec![],
        })
    }
}

impl LiveBench {
    fn parse_response(
        &self,
        data: &serde_json::Value,
        fetched_at: Option<chrono::DateTime<Utc>>,
        status: SourceStatus,
    ) -> SourceResult {
        let mut scores = Vec::new();

        // Expected structure (when JSON endpoint is available):
        // {
        //   "leaderboard": [
        //     {
        //       "model": "model-name",
        //       "global_average": 75.5,
        //       "math": 72.3,
        //       "coding": 68.9,
        //       "reasoning": 80.1,
        //       "language": 77.2,
        //       "data_analysis": 65.4,
        //       "instruction_following": 82.1
        //     },
        //     ...
        //   ]
        // }

        if let Some(leaderboard) = data
            .get("leaderboard")
            .or_else(|| data.get("results"))
            .and_then(|v| v.as_array())
        {
            let mut ranked_models: Vec<(
                String,
                f64,
                HashMap<String, f64>,
                Option<String>,
            )> = Vec::new();

            for model_entry in leaderboard {
                let source_model_name = match model_entry.get("model").and_then(|v| v.as_str()) {
                    Some(name) => name.to_string(),
                    None => continue,
                };

                // Get the overall/global score for ranking
                let overall_score = model_entry
                    .get("global_average")
                    .or_else(|| model_entry.get("overall_score"))
                    .or_else(|| model_entry.get("overall"))
                    .and_then(|v| v.as_f64());

                let overall_score = match overall_score {
                    Some(score) => score,
                    None => continue,
                };

                // Extract category scores
                let mut category_scores = HashMap::new();

                for category in &[
                    "math",
                    "coding",
                    "reasoning",
                    "language",
                    "data_analysis",
                    "instruction_following",
                ] {
                    if let Some(score) = model_entry.get(category).and_then(|v| v.as_f64()) {
                        category_scores.insert(category.to_string(), score);
                    }
                }

                // Extract version info if present
                let version = model_entry
                    .get("version")
                    .or_else(|| model_entry.get("model_version"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                ranked_models.push((
                    source_model_name,
                    overall_score,
                    category_scores,
                    version,
                ));
            }

            // Sort by overall score descending
            ranked_models.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            // Build ModelScore entries with ranks
            for (rank, model_data) in ranked_models.iter().enumerate() {
                let rank_u32 = (rank + 1) as u32;
                let source_model_name = &model_data.0;
                let overall_score = model_data.1;
                let category_scores = &model_data.2;
                let version = &model_data.3;

                // Derive canonical model name (lowercase, normalized)
                let canonical_name = normalize_model_name(source_model_name);

                let mut metrics = HashMap::new();

                // Add overall score
                metrics.insert("global_average".into(), MetricValue::Float(overall_score));

                // Add category scores
                for (category, score) in category_scores {
                    metrics.insert(category.clone(), MetricValue::Float(*score));
                }

                // Add version if present
                if let Some(v) = version {
                    metrics.insert("version".into(), MetricValue::Text(v.clone()));
                }

                // Add rank
                metrics.insert("rank".into(), MetricValue::Int(rank_u32 as i64));

                scores.push(ModelScore {
                    model: canonical_name,
                    source_model_name: source_model_name.clone(),
                    metrics,
                    rank: Some(rank_u32),
                });
            }
        }

        SourceResult {
            source: self.name().into(),
            fetched_at,
            status,
            scores,
        }
    }
}

fn normalize_model_name(name: &str) -> String {
    name.to_lowercase()
        .replace(' ', "-")
        .replace('_', "-")
}
