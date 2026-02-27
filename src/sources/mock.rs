use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;

pub struct MockSource;

impl Source for MockSource {
    fn name(&self) -> &str {
        "mock"
    }

    fn fetch(&self, _config: &Config, _cache: &Cache) -> Result<SourceResult> {
        let scores = vec![
            ModelScore {
                model: "claude-opus-4.6".into(),
                source_model_name: "Claude Opus 4.6".into(),
                metrics: HashMap::from([
                    ("score".into(), MetricValue::Float(92.5)),
                    ("rank".into(), MetricValue::Int(1)),
                ]),
                rank: Some(1),
            },
            ModelScore {
                model: "gpt-5.2".into(),
                source_model_name: "GPT-5.2".into(),
                metrics: HashMap::from([
                    ("score".into(), MetricValue::Float(89.1)),
                    ("rank".into(), MetricValue::Int(2)),
                ]),
                rank: Some(2),
            },
            ModelScore {
                model: "gemini-3.1-pro".into(),
                source_model_name: "Gemini 3.1 Pro".into(),
                metrics: HashMap::from([
                    ("score".into(), MetricValue::Float(87.3)),
                    ("rank".into(), MetricValue::Int(3)),
                ]),
                rank: Some(3),
            },
        ];

        Ok(SourceResult {
            source: self.name().into(),
            fetched_at: Some(Utc::now()),
            status: SourceStatus::Ok,
            scores,
        })
    }
}
