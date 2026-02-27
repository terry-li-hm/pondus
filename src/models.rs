use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceResult {
    pub source: String,
    pub fetched_at: Option<DateTime<Utc>>,
    pub status: SourceStatus,
    pub scores: Vec<ModelScore>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Ok,
    Cached,
    Unavailable,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelScore {
    pub model: String,
    pub source_model_name: String,
    pub metrics: HashMap<String, MetricValue>,
    pub rank: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetricValue {
    Float(f64),
    Int(i64),
    Text(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PondusOutput {
    pub timestamp: DateTime<Utc>,
    pub query: QueryInfo,
    pub sources: Vec<SourceResult>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueryInfo {
    #[serde(rename = "type")]
    pub query_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<usize>,
}
