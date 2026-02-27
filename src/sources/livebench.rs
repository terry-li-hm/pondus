use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;

pub struct LiveBench;

const BATCH_SIZE: usize = 100;
const HF_ROWS_URL: &str = "https://datasets-server.huggingface.co/rows";

impl Source for LiveBench {
    fn name(&self) -> &str {
        "livebench"
    }

    fn fetch(&self, _config: &Config, cache: &Cache) -> Result<SourceResult> {
        if let Some((fetched_at, cached_data)) = cache.get("livebench") {
            return Ok(self.parse_cached(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        // Fetch all rows from HuggingFace datasets-server API
        let mut all_scores: HashMap<String, Vec<f64>> = HashMap::new();
        let mut offset = 0;

        loop {
            let url = format!(
                "{}?dataset=livebench/model_judgment&config=default&split=leaderboard&offset={}&length={}",
                HF_ROWS_URL, offset, BATCH_SIZE
            );

            let response = match client.get(&url).send() {
                Ok(r) => r,
                Err(_) => break,
            };

            let data: serde_json::Value = match response.json() {
                Ok(d) => d,
                Err(_) => break,
            };

            let rows = match data.get("rows").and_then(|v| v.as_array()) {
                Some(r) => r,
                None => break,
            };

            if rows.is_empty() {
                break;
            }

            for row in rows {
                let row = match row.get("row") {
                    Some(r) => r,
                    None => continue,
                };

                let model = match row.get("model").and_then(|v| v.as_str()) {
                    Some(m) => m.to_string(),
                    None => continue,
                };

                let score = match row.get("score").and_then(|v| v.as_f64()) {
                    Some(s) => s,
                    None => continue,
                };

                all_scores.entry(model).or_default().push(score);
            }

            let total = data
                .get("num_rows_total")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            offset += BATCH_SIZE;
            if offset >= total {
                break;
            }
        }

        if all_scores.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(Utc::now()),
                status: SourceStatus::Error(
                    "Failed to fetch LiveBench data from HuggingFace datasets API".into(),
                ),
                scores: vec![],
            });
        }

        // Aggregate: average score per model, scale to 0-100
        let mut model_avgs: Vec<(String, f64)> = all_scores
            .into_iter()
            .map(|(model, scores)| {
                let avg = scores.iter().sum::<f64>() / scores.len() as f64 * 100.0;
                (model, avg)
            })
            .collect();

        model_avgs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Cache the aggregated data
        let cached_rows: Vec<serde_json::Value> = model_avgs
            .iter()
            .map(|(model, score)| {
                serde_json::json!({
                    "source_model_name": model,
                    "global_average": score,
                })
            })
            .collect();

        let cache_value = serde_json::json!({ "scores": cached_rows });
        cache.set("livebench", &cache_value)?;

        Ok(self.parse_cached(&cache_value, Some(Utc::now()), SourceStatus::Ok))
    }
}

impl LiveBench {
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
                        let name = entry
                            .get("source_model_name")
                            .and_then(|v| v.as_str())?
                            .to_string();
                        let score = entry.get("global_average").and_then(|v| v.as_f64())?;
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
                metrics.insert("global_average".into(), MetricValue::Float(score));
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
