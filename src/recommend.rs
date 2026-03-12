use crate::alias::AliasMap;
use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::output::OutputFormat;
use crate::sources::Source;
use crate::sources::aa::{AaEffortFilter, classify_effort_level};
use crate::sources::{self};
use anyhow::{Result, anyhow};
use chrono::Utc;
use clap::ValueEnum;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendTask {
    Coding,
    Agentic,
    Intelligence,
    General,
    Cost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortDirection {
    Desc,
    Asc,
}

#[derive(Debug, Clone, Copy)]
struct SourceMetricSpec {
    source: &'static str,
    label: &'static str,
    metric: &'static str,
    sort: SortDirection,
}

#[derive(Debug, Clone, Copy)]
pub struct TaskSpec {
    task: RecommendTask,
    description: &'static str,
    sources: &'static [SourceMetricSpec],
}

const CODING_SOURCES: &[SourceMetricSpec] = &[
    SourceMetricSpec {
        source: "swebench",
        label: "SWE-bench",
        metric: "resolved_rate",
        sort: SortDirection::Desc,
    },
    SourceMetricSpec {
        source: "terminal-bench",
        label: "Terminal-Bench",
        metric: "tasks_completed",
        sort: SortDirection::Desc,
    },
    SourceMetricSpec {
        source: "aider",
        label: "Aider",
        metric: "pass_rate_1",
        sort: SortDirection::Desc,
    },
    SourceMetricSpec {
        source: "swe-rebench",
        label: "SWE-rebench",
        metric: "resolve_rate",
        sort: SortDirection::Desc,
    },
];

const AGENTIC_SOURCES: &[SourceMetricSpec] = &[
    SourceMetricSpec {
        source: "terminal-bench",
        label: "Terminal-Bench",
        metric: "tasks_completed",
        sort: SortDirection::Desc,
    },
    SourceMetricSpec {
        source: "seal",
        label: "SEAL",
        metric: "overall_score",
        sort: SortDirection::Desc,
    },
];

const INTELLIGENCE_SOURCES: &[SourceMetricSpec] = &[SourceMetricSpec {
    source: "artificial-analysis",
    label: "Artificial Analysis",
    metric: "intelligence_index",
    sort: SortDirection::Desc,
}];

const GENERAL_SOURCES: &[SourceMetricSpec] = &[SourceMetricSpec {
    source: "arena",
    label: "Arena",
    metric: "elo_score",
    sort: SortDirection::Desc,
}];

const COST_SOURCES: &[SourceMetricSpec] = &[SourceMetricSpec {
    source: "openrouter",
    label: "OpenRouter",
    metric: "total_cost",
    sort: SortDirection::Asc,
}];

const TASK_SPECS: &[TaskSpec] = &[
    TaskSpec {
        task: RecommendTask::Coding,
        description: "Use coding benchmarks with SWE-bench as the primary signal.",
        sources: CODING_SOURCES,
    },
    TaskSpec {
        task: RecommendTask::Agentic,
        description: "Use agentic execution benchmarks with Terminal-Bench weighted first.",
        sources: AGENTIC_SOURCES,
    },
    TaskSpec {
        task: RecommendTask::Intelligence,
        description: "Use Artificial Analysis intelligence index; max-effort variants are best.",
        sources: INTELLIGENCE_SOURCES,
    },
    TaskSpec {
        task: RecommendTask::General,
        description: "Use Arena human preference ELO for general-purpose model choice.",
        sources: GENERAL_SOURCES,
    },
    TaskSpec {
        task: RecommendTask::Cost,
        description: "Use OpenRouter pricing and rank the cheapest models first.",
        sources: COST_SOURCES,
    },
];

#[derive(Debug, Clone, Serialize)]
struct RankedModel {
    rank: usize,
    model: String,
    metrics: BTreeMap<String, Option<RecommendMetricValue>>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum RecommendMetricValue {
    Float(f64),
    Int(i64),
}

#[derive(Debug, Serialize)]
struct RecommendSourceStatus {
    source: String,
    label: String,
    status: String,
    fetched_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct RecommendOutput {
    timestamp: chrono::DateTime<Utc>,
    task: RecommendTask,
    description: &'static str,
    effort: String,
    top: usize,
    sources: Vec<RecommendSourceStatus>,
    rows: Vec<RankedModel>,
}

#[derive(Debug, Clone, Default)]
struct AggregatedModel {
    model: String,
    metrics: BTreeMap<String, RecommendMetricValue>,
}

pub fn spec_for_task(task: RecommendTask) -> &'static TaskSpec {
    TASK_SPECS
        .iter()
        .find(|spec| spec.task == task)
        .expect("missing recommend task spec")
}

pub fn list_tasks(format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => {
            let tasks = TASK_SPECS
                .iter()
                .map(|spec| TaskSpecJson {
                    task: spec.task,
                    description: spec.description,
                    sources: spec.sources.iter().map(|source| source.source).collect(),
                })
                .collect::<Vec<_>>();
            Ok(serde_json::to_string_pretty(&tasks)?)
        }
        OutputFormat::Table | OutputFormat::Markdown => {
            let mut lines = Vec::with_capacity(TASK_SPECS.len() + 1);
            lines.push("Task          Description".to_string());
            lines.push(
                "------------  --------------------------------------------------------------"
                    .to_string(),
            );
            for spec in TASK_SPECS {
                lines.push(format!(
                    "{:<12}  {}",
                    task_name(spec.task),
                    spec.description
                ));
            }
            Ok(lines.join("\n"))
        }
    }
}

#[derive(Serialize)]
struct TaskSpecJson {
    task: RecommendTask,
    description: &'static str,
    sources: Vec<&'static str>,
}

pub fn run(
    config: &Config,
    cache: &Cache,
    aliases: &AliasMap,
    task: RecommendTask,
    top: usize,
    effort: AaEffortFilter,
    format: OutputFormat,
) -> Result<()> {
    let spec = spec_for_task(task);
    let mut results = fetch_recommend_sources(config, cache, spec.sources)?;

    if spec
        .sources
        .iter()
        .any(|source_spec| source_spec.source == "artificial-analysis")
    {
        apply_aa_effort_filter(&mut results, effort);
    }

    for result in &results {
        eprintln!("[{}] {}", result.source, status_label(&result.status));
    }

    let rows = rank_models(spec, &results, aliases, top);
    let output = RecommendOutput {
        timestamp: Utc::now(),
        task,
        description: spec.description,
        effort: format!("{effort:?}").to_lowercase(),
        top,
        sources: results
            .iter()
            .map(|result| RecommendSourceStatus {
                label: spec
                    .sources
                    .iter()
                    .find(|source_spec| source_spec.source == result.source)
                    .map(|source_spec| source_spec.label.to_string())
                    .unwrap_or_else(|| result.source.clone()),
                source: result.source.clone(),
                status: status_label(&result.status).to_string(),
                fetched_at: result.fetched_at,
            })
            .collect(),
        rows,
    };

    let rendered = match format {
        OutputFormat::Json => serde_json::to_string_pretty(&output)?,
        OutputFormat::Table => render_table(spec, &output),
        OutputFormat::Markdown => render_markdown(spec, &output),
    };

    println!("{rendered}");
    Ok(())
}

fn fetch_recommend_sources(
    config: &Config,
    cache: &Cache,
    specs: &[SourceMetricSpec],
) -> Result<Vec<SourceResult>> {
    let wanted: HashSet<&str> = specs.iter().map(|spec| spec.source).collect();
    let mut source_map: HashMap<String, Box<dyn Source>> = sources::all_sources()
        .into_iter()
        .map(|source| (source.name().to_string(), source))
        .collect();

    let mut results = Vec::with_capacity(specs.len());
    for spec in specs {
        let Some(source) = source_map.remove(spec.source) else {
            return Err(anyhow!(
                "Unknown source in recommend task taxonomy: {}",
                spec.source
            ));
        };

        if !wanted.contains(spec.source) {
            continue;
        }

        let result = match source.fetch(config, cache) {
            Ok(result) => result,
            Err(err) => SourceResult {
                source: spec.source.to_string(),
                fetched_at: None,
                status: SourceStatus::Error(err.to_string()),
                scores: vec![],
            },
        };
        results.push(result);
    }

    Ok(results)
}

fn apply_aa_effort_filter(results: &mut [SourceResult], effort: AaEffortFilter) {
    if effort == AaEffortFilter::All {
        return;
    }

    for result in results {
        if result.source != "artificial-analysis" {
            continue;
        }

        result
            .scores
            .retain(|score| effort.matches(classify_effort_level(&score.source_model_name)));
    }
}

fn rank_models(
    spec: &TaskSpec,
    results: &[SourceResult],
    aliases: &AliasMap,
    top: usize,
) -> Vec<RankedModel> {
    let mut models: HashMap<String, AggregatedModel> = HashMap::new();

    for source_spec in spec.sources {
        let Some(result) = results
            .iter()
            .find(|result| result.source == source_spec.source)
        else {
            continue;
        };

        let mut best_for_source: HashMap<String, RecommendMetricValue> = HashMap::new();
        for score in &result.scores {
            let Some(metric) = extract_metric(score, source_spec.metric) else {
                continue;
            };
            let model = canonical_model_name(aliases, score);
            match best_for_source.get(&model) {
                Some(existing) if !is_better(metric, *existing, source_spec.sort) => {}
                _ => {
                    best_for_source.insert(model, metric);
                }
            }
        }

        for (model, metric) in best_for_source {
            let entry = models
                .entry(model.clone())
                .or_insert_with(|| AggregatedModel {
                    model,
                    metrics: BTreeMap::new(),
                });
            entry.metrics.insert(source_spec.source.to_string(), metric);
        }
    }

    let primary = spec
        .sources
        .first()
        .expect("recommend spec missing primary source");
    let mut ranked: Vec<AggregatedModel> = models
        .into_values()
        .filter(|model| !model.metrics.is_empty())
        .collect();

    ranked.sort_by(|left, right| compare_models(spec, primary, left, right));
    ranked.truncate(top);

    ranked
        .into_iter()
        .enumerate()
        .map(|(index, model)| RankedModel {
            rank: index + 1,
            model: model.model,
            metrics: spec
                .sources
                .iter()
                .map(|source_spec| {
                    (
                        source_spec.source.to_string(),
                        model.metrics.get(source_spec.source).copied(),
                    )
                })
                .collect(),
        })
        .collect()
}

fn compare_models(
    spec: &TaskSpec,
    primary: &SourceMetricSpec,
    left: &AggregatedModel,
    right: &AggregatedModel,
) -> Ordering {
    let left_primary = left.metrics.get(primary.source).copied();
    let right_primary = right.metrics.get(primary.source).copied();

    compare_option_metric(left_primary, right_primary, primary.sort)
        .then_with(|| right.metrics.len().cmp(&left.metrics.len()))
        .then_with(|| {
            for source_spec in spec.sources.iter().skip(1) {
                let ordering = compare_option_metric(
                    left.metrics.get(source_spec.source).copied(),
                    right.metrics.get(source_spec.source).copied(),
                    source_spec.sort,
                );
                if ordering != Ordering::Equal {
                    return ordering;
                }
            }
            Ordering::Equal
        })
        .then_with(|| left.model.cmp(&right.model))
}

fn compare_option_metric(
    left: Option<RecommendMetricValue>,
    right: Option<RecommendMetricValue>,
    direction: SortDirection,
) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => compare_metric(left, right, direction),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_metric(
    left: RecommendMetricValue,
    right: RecommendMetricValue,
    direction: SortDirection,
) -> Ordering {
    let ordering = metric_as_f64(left)
        .partial_cmp(&metric_as_f64(right))
        .unwrap_or(Ordering::Equal);
    match direction {
        SortDirection::Desc => ordering.reverse(),
        SortDirection::Asc => ordering,
    }
}

fn is_better(
    candidate: RecommendMetricValue,
    current: RecommendMetricValue,
    direction: SortDirection,
) -> bool {
    compare_metric(candidate, current, direction) == Ordering::Less
}

fn extract_metric(score: &ModelScore, metric_name: &str) -> Option<RecommendMetricValue> {
    if metric_name == "total_cost" {
        let prompt = metric_f64(score, "prompt_per_1m")?;
        let completion = metric_f64(score, "completion_per_1m")?;
        return Some(RecommendMetricValue::Float(prompt + completion));
    }

    match score.metrics.get(metric_name)? {
        MetricValue::Float(value) => Some(RecommendMetricValue::Float(*value)),
        MetricValue::Int(value) => Some(RecommendMetricValue::Int(*value)),
        MetricValue::Text(_) => None,
    }
}

fn metric_f64(score: &ModelScore, metric_name: &str) -> Option<f64> {
    match score.metrics.get(metric_name)? {
        MetricValue::Float(value) => Some(*value),
        MetricValue::Int(value) => Some(*value as f64),
        MetricValue::Text(_) => None,
    }
}

fn canonical_model_name(aliases: &AliasMap, score: &ModelScore) -> String {
    let by_model = aliases.resolve(&score.model);
    if by_model != score.model.to_lowercase() {
        return by_model;
    }

    let by_source_name = aliases.resolve(&score.source_model_name);
    if by_source_name != score.source_model_name.to_lowercase() {
        return by_source_name;
    }

    score.model.to_lowercase()
}

fn metric_as_f64(metric: RecommendMetricValue) -> f64 {
    match metric {
        RecommendMetricValue::Float(value) => value,
        RecommendMetricValue::Int(value) => value as f64,
    }
}

fn render_table(spec: &TaskSpec, output: &RecommendOutput) -> String {
    let mut lines = Vec::new();
    lines.push(String::new());
    lines.push(format!(
        "Task: {}  (sources: {})",
        task_name(output.task),
        spec.sources
            .iter()
            .map(|source| source.source)
            .collect::<Vec<_>>()
            .join(", ")
    ));
    lines.push(String::new());

    let mut headers = vec!["Rank".to_string(), "Model".to_string()];
    headers.extend(spec.sources.iter().map(|source| source.label.to_string()));
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for row in &output.rows {
        let mut values = vec![row.rank.to_string(), row.model.clone()];
        for source_spec in spec.sources {
            let cell = row
                .metrics
                .get(source_spec.source)
                .and_then(|value| *value)
                .map(|value| format_metric(source_spec.metric, value))
                .unwrap_or_else(|| "—".to_string());
            values.push(cell);
        }
        for (index, value) in values.iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
        rows.push(values);
    }

    lines.push(padded_row(&headers, &widths));
    lines.push(
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  "),
    );
    for row in rows {
        lines.push(padded_row(&row, &widths));
    }
    lines.join("\n")
}

fn render_markdown(spec: &TaskSpec, output: &RecommendOutput) -> String {
    let mut lines = Vec::new();
    lines.push(format!(
        "**Task:** `{}`  \n**Sources:** {}",
        task_name(output.task),
        spec.sources
            .iter()
            .map(|source| source.source)
            .collect::<Vec<_>>()
            .join(", ")
    ));
    lines.push(String::new());

    let mut headers = vec!["Rank".to_string(), "Model".to_string()];
    headers.extend(spec.sources.iter().map(|source| source.label.to_string()));
    lines.push(format!("| {} |", headers.join(" | ")));
    lines.push(format!(
        "| {} |",
        headers
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ")
    ));

    for row in &output.rows {
        let mut values = vec![row.rank.to_string(), row.model.clone()];
        for source_spec in spec.sources {
            let cell = row
                .metrics
                .get(source_spec.source)
                .and_then(|value| *value)
                .map(|value| format_metric(source_spec.metric, value))
                .unwrap_or_else(|| "—".to_string());
            values.push(cell);
        }
        lines.push(format!("| {} |", values.join(" | ")));
    }

    lines.join("\n")
}

fn padded_row(values: &[String], widths: &[usize]) -> String {
    values
        .iter()
        .enumerate()
        .map(|(index, value)| format!("{value:width$}", width = widths[index]))
        .collect::<Vec<_>>()
        .join("  ")
}

fn format_metric(metric_name: &str, metric: RecommendMetricValue) -> String {
    match metric {
        RecommendMetricValue::Float(value) => match metric_name {
            "resolved_rate" | "resolve_rate" | "pass_rate_1" => format!("{value:.1}%"),
            "elo_score" | "intelligence_index" | "overall_score" => format!("{value:.1}"),
            "total_cost" | "prompt_per_1m" | "completion_per_1m" => format!("${value:.2}"),
            _ => format!("{value:.2}"),
        },
        RecommendMetricValue::Int(value) => value.to_string(),
    }
}

fn status_label(status: &SourceStatus) -> &'static str {
    match status {
        SourceStatus::Ok => "OK",
        SourceStatus::Cached => "Cached",
        SourceStatus::Unavailable => "Unavailable",
        SourceStatus::Error(_) => "Error",
    }
}

fn task_name(task: RecommendTask) -> &'static str {
    match task {
        RecommendTask::Coding => "coding",
        RecommendTask::Agentic => "agentic",
        RecommendTask::Intelligence => "intelligence",
        RecommendTask::General => "general",
        RecommendTask::Cost => "cost",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_score(model: &str, metric_name: &str, value: RecommendMetricValue) -> ModelScore {
        let metric = match value {
            RecommendMetricValue::Float(value) => MetricValue::Float(value),
            RecommendMetricValue::Int(value) => MetricValue::Int(value),
        };

        ModelScore {
            model: model.to_string(),
            source_model_name: model.to_string(),
            metrics: HashMap::from([(metric_name.to_string(), metric)]),
            rank: None,
        }
    }

    fn make_source(source: &str, scores: Vec<ModelScore>) -> SourceResult {
        SourceResult {
            source: source.to_string(),
            fetched_at: None,
            status: SourceStatus::Cached,
            scores,
        }
    }

    #[test]
    fn task_taxonomy_matches_expected_sources() {
        assert_eq!(
            spec_for_task(RecommendTask::Coding)
                .sources
                .iter()
                .map(|source| source.source)
                .collect::<Vec<_>>(),
            vec!["swebench", "terminal-bench", "aider", "swe-rebench"]
        );
        assert_eq!(
            spec_for_task(RecommendTask::Agentic)
                .sources
                .iter()
                .map(|source| source.source)
                .collect::<Vec<_>>(),
            vec!["terminal-bench", "seal"]
        );
        assert_eq!(
            spec_for_task(RecommendTask::Intelligence)
                .sources
                .iter()
                .map(|source| source.source)
                .collect::<Vec<_>>(),
            vec!["artificial-analysis"]
        );
        assert_eq!(
            spec_for_task(RecommendTask::General)
                .sources
                .iter()
                .map(|source| source.source)
                .collect::<Vec<_>>(),
            vec!["arena"]
        );
        assert_eq!(
            spec_for_task(RecommendTask::Cost)
                .sources
                .iter()
                .map(|source| source.source)
                .collect::<Vec<_>>(),
            vec!["openrouter"]
        );
    }

    #[test]
    fn ranking_prefers_primary_metric_when_partial_data_exists() {
        let aliases = AliasMap::load(Some("/tmp/pondus-recommend-no-override.toml")).unwrap();
        let spec = spec_for_task(RecommendTask::Coding);
        let results = vec![
            make_source(
                "swebench",
                vec![
                    make_score(
                        "model-a",
                        "resolved_rate",
                        RecommendMetricValue::Float(80.0),
                    ),
                    make_score(
                        "model-b",
                        "resolved_rate",
                        RecommendMetricValue::Float(78.0),
                    ),
                ],
            ),
            make_source(
                "aider",
                vec![
                    make_score("model-c", "pass_rate_1", RecommendMetricValue::Float(95.0)),
                    make_score("model-b", "pass_rate_1", RecommendMetricValue::Float(82.0)),
                ],
            ),
            make_source("swe-rebench", vec![]),
            make_source("terminal-bench", vec![]),
        ];

        let ranked = rank_models(spec, &results, &aliases, 10);
        assert_eq!(
            ranked
                .iter()
                .map(|row| row.model.as_str())
                .collect::<Vec<_>>(),
            vec!["model-a", "model-b", "model-c"]
        );
        assert!(
            ranked[2]
                .metrics
                .get("swebench")
                .is_some_and(Option::is_none)
        );
        assert!(
            ranked[2]
                .metrics
                .get("aider")
                .and_then(|value| *value)
                .is_some()
        );
    }
}
