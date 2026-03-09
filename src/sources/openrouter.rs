use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus, SourceTag};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

pub struct OpenRouter;
static TAGS: &[SourceTag] = &[SourceTag::General];

impl Source for OpenRouter {
    fn name(&self) -> &str {
        "openrouter"
    }

    fn tags(&self) -> &'static [SourceTag] {
        TAGS
    }

    fn fetch(&self, _config: &Config, cache: &Cache) -> Result<SourceResult> {
        if let Some((fetched_at, cached_data)) = cache.get("openrouter") {
            return Ok(self.parse_cached(&cached_data, Some(fetched_at), SourceStatus::Cached));
        }

        self.fetch_api(cache)
    }
}

impl OpenRouter {
    fn fetch_api(&self, cache: &Cache) -> Result<SourceResult> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to build HTTP client")?;

        let response = client
            .get("https://openrouter.ai/api/v1/models")
            .send()
            .context("Failed to fetch from OpenRouter API")?;

        if !response.status().is_success() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Error(format!(
                    "OpenRouter API returned HTTP {}",
                    response.status()
                )),
                scores: vec![],
            });
        }

        let payload: OpenRouterResponse = response
            .json()
            .context("Failed to parse OpenRouter API response")?;

        let cached_rows: Vec<serde_json::Value> = payload
            .data
            .into_iter()
            .filter_map(|model| {
                let pricing = model.pricing?;
                let prompt_str = pricing.prompt?;
                let completion_str = pricing.completion?;
                let prompt_per_token: f64 = prompt_str.parse().ok()?;
                let completion_per_token: f64 = completion_str.parse().ok()?;
                // Skip free models (price = 0) — they are proxies or special-case entries
                if prompt_per_token == 0.0 && completion_per_token == 0.0 {
                    return None;
                }
                Some(serde_json::json!({
                    "source_model_name": model.id,
                    "prompt_per_1m": prompt_per_token * 1_000_000.0,
                    "completion_per_1m": completion_per_token * 1_000_000.0,
                }))
            })
            .collect();

        if cached_rows.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: Some(Utc::now()),
                status: SourceStatus::Error(
                    "OpenRouter API returned no models with pricing data".into(),
                ),
                scores: vec![],
            });
        }

        let cache_value = serde_json::json!({ "scores": cached_rows });
        cache.set("openrouter", &cache_value)?;

        Ok(self.parse_cached(&cache_value, Some(Utc::now()), SourceStatus::Ok))
    }

    fn parse_cached(
        &self,
        data: &serde_json::Value,
        fetched_at: Option<DateTime<Utc>>,
        status: SourceStatus,
    ) -> SourceResult {
        let scores = data
            .get("scores")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|entry| {
                        let source_model_name = entry
                            .get("source_model_name")
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned)?;
                        let prompt_per_1m = entry
                            .get("prompt_per_1m")
                            .and_then(|v| v.as_f64())?;
                        let completion_per_1m = entry
                            .get("completion_per_1m")
                            .and_then(|v| v.as_f64())?;

                        let mut metrics = HashMap::new();
                        metrics.insert(
                            "prompt_per_1m".into(),
                            MetricValue::Float(prompt_per_1m),
                        );
                        metrics.insert(
                            "completion_per_1m".into(),
                            MetricValue::Float(completion_per_1m),
                        );

                        // Normalise model ID for alias matching:
                        // "openai/gpt-5.2-pro" → "openai/gpt-5.2-pro" (keep as-is,
                        // the alias map has "openai/gpt-5.2" as an alias for "gpt-5.2")
                        let model = source_model_name.to_lowercase().replace([' ', '_'], "-");

                        Some(ModelScore {
                            model,
                            source_model_name,
                            metrics,
                            rank: None, // pricing has no rank ordering
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        SourceResult {
            source: self.name().into(),
            fetched_at,
            status,
            scores,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenRouterResponse {
    data: Vec<OpenRouterModel>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterModel {
    id: String,
    pricing: Option<OpenRouterPricing>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPricing {
    prompt: Option<String>,
    completion: Option<String>,
}
