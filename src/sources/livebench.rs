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
        if let Ok(data) = client
            .get(json_url)
            .send()
            .and_then(|r| r.json::<serde_json::Value>())
        {
            cache.set("livebench", &data)?;
            return Ok(self.parse_response(&data, Some(Utc::now()), SourceStatus::Ok));
        }

        // Fall back to HuggingFace parquet endpoint
        let hf_url = "https://huggingface.co/api/datasets/livebench/model_judgment/parquet";
        if let Ok(true) = client.get(hf_url).send().map(|r| r.status().is_success()) {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Error(
                    "LiveBench data is stored in parquet format. \
                     Parquet deserialization not yet implemented."
                        .into(),
                ),
                scores: vec![],
            });
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
            let mut ranked: Vec<(String, f64)> = Vec::new();

            for entry in leaderboard {
                let Some(name) = entry.get("model").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(score) = entry
                    .get("global_average")
                    .or_else(|| entry.get("overall_score"))
                    .or_else(|| entry.get("overall"))
                    .and_then(|v| v.as_f64())
                else {
                    continue;
                };
                ranked.push((name.to_string(), score));
            }

            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            for (rank, (source_model_name, overall_score)) in ranked.iter().enumerate() {
                let rank_u32 = (rank + 1) as u32;
                let mut metrics = HashMap::new();
                metrics.insert("global_average".into(), MetricValue::Float(*overall_score));
                metrics.insert("rank".into(), MetricValue::Int(rank_u32 as i64));

                scores.push(ModelScore {
                    model: normalize_model_name(source_model_name),
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
    name.to_lowercase().replace([' ', '_'], "-")
}
