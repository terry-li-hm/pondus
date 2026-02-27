use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const CACHE_KEY: &str = "terminal-bench";
const HF_API_URL: &str = "https://huggingface.co/api/datasets/sabhay/terminal-bench-2-leaderboard";
const HF_RAW_BASE: &str = "https://huggingface.co/datasets/sabhay/terminal-bench-2-leaderboard/raw/main";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TerminalBenchResult {
    #[serde(default)]
    config: Option<ResultConfig>,
    #[serde(default)]
    verifier_result: Option<VerifierResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResultConfig {
    #[serde(default)]
    agent: Option<AgentConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AgentConfig {
    model_name: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VerifierResult {
    #[serde(default)]
    rewards: Option<HashMap<String, f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HFDatasetResponse {
    siblings: Option<Vec<HFSibling>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HFSibling {
    rfilename: String,
}

pub struct TerminalBench;

impl Source for TerminalBench {
    fn name(&self) -> &str {
        "terminal-bench"
    }

    fn fetch(&self, _config: &Config, cache: &Cache) -> Result<SourceResult> {
        // Check cache first
        if let Some((fetched_at, cached_data)) = cache.get(CACHE_KEY) {
            if let Ok(scores) = serde_json::from_value::<Vec<ModelScore>>(cached_data.clone()) {
                return Ok(SourceResult {
                    source: self.name().into(),
                    fetched_at: Some(fetched_at),
                    status: SourceStatus::Cached,
                    scores,
                });
            }
        }

        // Fetch the dataset metadata to find all result.json files
        let client = reqwest::blocking::Client::new();
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

        let metadata = response
            .json::<HFDatasetResponse>()
            .context("Failed to parse dataset metadata")?;

        // Extract result.json file paths from siblings
        let result_files: Vec<_> = metadata
            .siblings
            .unwrap_or_default()
            .iter()
            .filter(|s| s.rfilename.ends_with("result.json"))
            .map(|s| s.rfilename.clone())
            .collect();

        if result_files.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Unavailable,
                scores: vec![],
            });
        }

        // Fetch and parse each result.json file
        let mut all_results = Vec::new();
        for file_path in result_files {
            let url = format!("{}/submissions/{}", HF_RAW_BASE, file_path);
            if let Ok(resp) = client.get(&url).send() {
                if resp.status().is_success() {
                    if let Ok(result) = resp.json::<TerminalBenchResult>() {
                        all_results.push(result);
                    }
                }
            }
            // Continue on individual file failures
        }

        if all_results.is_empty() {
            return Ok(SourceResult {
                source: self.name().into(),
                fetched_at: None,
                status: SourceStatus::Unavailable,
                scores: vec![],
            });
        }

        // Parse results into ModelScore entries
        let mut model_scores: HashMap<String, (String, f64, u32)> = HashMap::new();

        for result in all_results {
            let model_name = result
                .config
                .as_ref()
                .and_then(|c| c.agent.as_ref())
                .and_then(|a| a.model_name.as_ref())
                .cloned();

            let _agent_name = result
                .config
                .as_ref()
                .and_then(|c| c.agent.as_ref())
                .and_then(|a| a.name.as_ref())
                .cloned()
                .unwrap_or_else(|| "unknown".to_string());

            let reward = result
                .verifier_result
                .as_ref()
                .and_then(|v| v.rewards.as_ref())
                .and_then(|r| r.get("reward").copied())
                .unwrap_or(0.0);

            if let Some(model_name) = model_name {
                // Track the best score for each model (aggregate across submissions)
                let canonical_name = normalize_model_name(&model_name);
                let entry = model_scores
                    .entry(canonical_name.clone())
                    .or_insert_with(|| (model_name, reward, 1));

                // Keep the maximum reward
                if reward > entry.1 {
                    entry.1 = reward;
                }
                entry.2 += 1;
            }
        }

        // Convert to ModelScore entries with ranks
        let mut scores: Vec<_> = model_scores
            .into_iter()
            .map(|(canonical_name, (source_model_name, reward, count))| {
                let mut metrics = HashMap::new();
                metrics.insert("score".into(), MetricValue::Float(reward));
                metrics.insert("submissions".into(), MetricValue::Int(count as i64));

                ModelScore {
                    model: canonical_name,
                    source_model_name,
                    metrics,
                    rank: None,
                }
            })
            .collect();

        // Sort by score descending
        scores.sort_by(|a, b| {
            let a_score = a
                .metrics
                .get("score")
                .and_then(|m| match m {
                    MetricValue::Float(f) => Some(*f),
                    _ => None,
                })
                .unwrap_or(0.0);
            let b_score = b
                .metrics
                .get("score")
                .and_then(|m| match m {
                    MetricValue::Float(f) => Some(*f),
                    _ => None,
                })
                .unwrap_or(0.0);
            b_score.partial_cmp(&a_score).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Assign ranks
        for (rank, score) in scores.iter_mut().enumerate() {
            let rank_u32 = (rank + 1) as u32;
            score.rank = Some(rank_u32);
            score
                .metrics
                .insert("rank".into(), MetricValue::Int(rank_u32 as i64));
        }

        // Cache the results
        if let Ok(json_value) = serde_json::to_value(&scores) {
            let _ = cache.set(CACHE_KEY, &json_value);
        }

        Ok(SourceResult {
            source: self.name().into(),
            fetched_at: Some(Utc::now()),
            status: SourceStatus::Ok,
            scores,
        })
    }
}

fn normalize_model_name(name: &str) -> String {
    name.to_lowercase()
        .replace(' ', "-")
        .replace('_', "-")
        .replace("gemini-3-pro-preview", "gemini-3-pro-preview")
        .replace("gemini-2-flash", "gemini-2-flash")
        .replace("claude", "claude")
        .replace("gpt", "gpt")
        .replace("llama", "llama")
}
