use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;

pub struct ArtificialAnalysis;

impl Source for ArtificialAnalysis {
    fn name(&self) -> &str {
        "artificial-analysis"
    }

    fn fetch(&self, config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check if API key is configured
        let api_key = match config.aa_api_key() {
            Some(key) => key,
            None => {
                return Ok(SourceResult {
                    source: self.name().into(),
                    fetched_at: None,
                    status: SourceStatus::Unavailable,
                    scores: vec![],
                });
            }
        };

        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get("artificial-analysis") {
            return Ok(self.parse_response(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        // Fetch from API
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;
        let response = client
            .get("https://artificialanalysis.ai/api/v2/data/llms/models")
            .header("x-api-key", api_key)
            .send()
            .context("Failed to fetch from Artificial Analysis API")?;

        let status = response.status();
        if !status.is_success() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Error(format!("HTTP {}", status)),
                scores: vec![],
            });
        }

        let data = response.json::<serde_json::Value>()?;

        // Cache the raw response
        cache.set("artificial-analysis", &data)?;

        Ok(self.parse_response(&data, Some(Utc::now()), SourceStatus::Ok))
    }
}

impl ArtificialAnalysis {
    fn parse_response(
        &self,
        data: &serde_json::Value,
        fetched_at: Option<chrono::DateTime<Utc>>,
        status: SourceStatus,
    ) -> SourceResult {
        let mut scores = Vec::new();

        // Extract array from response
        if let Some(models) = data.as_array() {
            let mut ranked_models: Vec<_> = models
                .iter()
                .filter_map(|model| {
                    let source_model_name = model.get("name")?.as_str()?.to_string();
                    let intelligence_index = model.get("intelligence_index")?.as_f64()?;
                    let input_cost = model.get("input_cost_per_1m_tokens")?.as_f64();
                    let output_cost = model.get("output_cost_per_1m_tokens")?.as_f64();
                    let speed = model.get("tokens_per_second")?.as_f64();

                    Some((
                        source_model_name,
                        intelligence_index,
                        input_cost,
                        output_cost,
                        speed,
                    ))
                })
                .collect();

            // Sort by intelligence_index descending
            ranked_models
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            // Build ModelScore entries with ranks
            for (rank, (source_model_name, intelligence_index, input_cost, output_cost, speed)) in
                ranked_models.iter().enumerate()
            {
                let rank_u32 = (rank + 1) as u32;

                // Derive canonical model name (lowercase, normalized)
                let canonical_name = source_model_name.to_lowercase().replace(' ', "-");

                let mut metrics = HashMap::new();
                metrics.insert(
                    "intelligence_index".into(),
                    MetricValue::Float(*intelligence_index),
                );
                if let Some(input) = input_cost {
                    metrics.insert(
                        "input_cost_per_1m_tokens".into(),
                        MetricValue::Float(*input),
                    );
                }
                if let Some(output) = output_cost {
                    metrics.insert(
                        "output_cost_per_1m_tokens".into(),
                        MetricValue::Float(*output),
                    );
                }
                if let Some(tps) = speed {
                    metrics.insert("tokens_per_second".into(), MetricValue::Float(*tps));
                }
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
