use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;

pub struct Arena;

impl Source for Arena {
    fn name(&self) -> &str {
        "arena"
    }

    fn fetch(&self, _config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get("arena") {
            return Ok(self.parse_response(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        // Fetch from GitHub raw content
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

        // Cache the raw response
        cache.set("arena", &data)?;

        Ok(self.parse_response(&data, Some(Utc::now()), SourceStatus::Ok))
    }
}

impl Arena {
    fn parse_response(
        &self,
        data: &serde_json::Value,
        fetched_at: Option<chrono::DateTime<Utc>>,
        status: SourceStatus,
    ) -> SourceResult {
        let mut scores = Vec::new();

        // The JSON structure is: { "20240527": { "text": { "full_old": { ... }, "overall": { ... } } }, ... }
        // We want to use the latest date's "overall" category
        if let Some(obj) = data.as_object() {
            // Get the latest date key (they appear to be date strings in YYYYMMDD format)
            let latest_date = obj
                .keys()
                .max_by(|a, b| a.cmp(b)); // String comparison works for YYYYMMDD format

            if let Some(date_key) = latest_date {
                if let Some(date_data) = obj.get(date_key) {
                    if let Some(text_data) = date_data.get("text") {
                        // Try to use "overall" category, fall back to "full_old" if not available
                        let category = if text_data.get("overall").is_some() {
                            "overall"
                        } else if text_data.get("full_old").is_some() {
                            "full_old"
                        } else {
                            // Try to use the first available category
                            if let Some(first_category) = text_data.as_object().and_then(|o| o.keys().next()) {
                                first_category.as_str()
                            } else {
                                return SourceResult {
                                    source: self.name().into(),
                                    fetched_at,
                                    status: SourceStatus::Error("No valid categories found".into()),
                                    scores: vec![],
                                };
                            }
                        };

                        if let Some(category_data) = text_data.get(category) {
                            if let Some(models_obj) = category_data.as_object() {
                                // Extract model names and ELO scores
                                let mut ranked_models: Vec<(String, f64)> = models_obj
                                    .iter()
                                    .filter_map(|(model_name, score_value)| {
                                        score_value.as_f64().map(|score| (model_name.clone(), score))
                                    })
                                    .collect();

                                // Sort by ELO score descending
                                ranked_models
                                    .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                                // Build ModelScore entries with ranks
                                for (rank, (source_model_name, elo_score)) in ranked_models.iter().enumerate() {
                                    let rank_u32 = (rank + 1) as u32;

                                    // Derive canonical model name (lowercase)
                                    let canonical_name = source_model_name.to_lowercase();

                                    let mut metrics = HashMap::new();
                                    metrics.insert("elo_score".into(), MetricValue::Float(*elo_score));
                                    metrics.insert("rank".into(), MetricValue::Int(rank_u32 as i64));

                                    scores.push(ModelScore {
                                        model: canonical_name,
                                        source_model_name: source_model_name.clone(),
                                        metrics,
                                        rank: Some(rank_u32),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        // If we got no scores, check if it was a parsing failure
        if scores.is_empty() && matches!(status, SourceStatus::Ok | SourceStatus::Cached) {
            return SourceResult {
                source: self.name().into(),
                fetched_at,
                status: SourceStatus::Error("Failed to parse Arena data structure".into()),
                scores: vec![],
            };
        }

        SourceResult {
            source: self.name().into(),
            fetched_at,
            status,
            scores,
        }
    }
}
