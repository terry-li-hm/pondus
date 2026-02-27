use crate::models::{MetricValue, PondusOutput, SourceStatus};
use anyhow::Result;
use owo_colors::OwoColorize;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Json,
    Table,
    Markdown,
}

impl OutputFormat {
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "json" => Ok(Self::Json),
            "table" => Ok(Self::Table),
            "markdown" | "md" => Ok(Self::Markdown),
            _ => anyhow::bail!("Unknown format: {s}. Expected: json, table, markdown"),
        }
    }
}

pub fn render(output: &PondusOutput, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Json => render_json(output),
        OutputFormat::Table => render_table(output),
        OutputFormat::Markdown => render_markdown(output),
    }
}

fn render_json(output: &PondusOutput) -> Result<String> {
    Ok(serde_json::to_string_pretty(output)?)
}

fn render_table(output: &PondusOutput) -> Result<String> {
    let mut result = String::new();

    for source in &output.sources {
        let status_str = format_status(&source.status);
        let header = format!("{} [{}]", source.source.bold(), status_str);
        result.push_str(&header);
        result.push('\n');

        if source.scores.is_empty() {
            result.push_str("  No results\n\n");
            continue;
        }

        let all_metrics: Vec<String> = source
            .scores
            .iter()
            .flat_map(|s| s.metrics.keys().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let mut columns: Vec<String> = vec!["Rank".to_string(), "Model".to_string()];
        columns.extend(all_metrics.clone());

        let mut widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();

        let mut rows: Vec<Vec<String>> = Vec::new();
        for score in &source.scores {
            let rank = score
                .rank
                .map(|r| r.to_string())
                .unwrap_or_else(|| "-".to_string());
            let mut row = vec![rank, score.model.clone()];
            for metric in &all_metrics {
                let val = score
                    .metrics
                    .get(metric)
                    .map(format_metric)
                    .unwrap_or_else(|| "-".to_string());
                row.push(val);
            }
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.len());
            }
            rows.push(row);
        }

        let header_row: String = columns
            .iter()
            .enumerate()
            .map(|(i, c)| pad(c, widths[i]))
            .collect::<Vec<_>>()
            .join("  ");
        result.push_str(&header_row);
        result.push('\n');

        let separator: String = widths
            .iter()
            .map(|&w| "-".repeat(w))
            .collect::<Vec<_>>()
            .join("  ");
        result.push_str(&separator);
        result.push('\n');

        for row in rows {
            let line: String = row
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    if i == 0 {
                        cell.cyan().to_string()
                    } else {
                        pad(cell, widths[i])
                    }
                })
                .collect::<Vec<_>>()
                .join("  ");
            result.push_str(&line);
            result.push('\n');
        }

        result.push('\n');
    }

    Ok(result.trim_end().to_string())
}

fn render_markdown(output: &PondusOutput) -> Result<String> {
    let mut result = String::new();

    for source in &output.sources {
        result.push_str(&format!("## {}\n\n", source.source));

        let status_str = match &source.status {
            SourceStatus::Ok => "OK",
            SourceStatus::Cached => "Cached",
            SourceStatus::Unavailable => "Unavailable",
            SourceStatus::Error(e) => &format!("Error: {}", e),
        };
        result.push_str(&format!("Status: {}\n\n", status_str));

        if source.scores.is_empty() {
            result.push_str("No results.\n\n");
            continue;
        }

        let all_metrics: Vec<String> = source
            .scores
            .iter()
            .flat_map(|s| s.metrics.keys().cloned())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let mut columns: Vec<String> = vec!["Rank".to_string(), "Model".to_string()];
        columns.extend(all_metrics.clone());

        let header: String = columns.to_vec().join(" | ");
        result.push_str(&format!("| {} |\n", header));

        let separator: String = columns
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ");
        result.push_str(&format!("| {} |\n", separator));

        for score in &source.scores {
            let rank = score
                .rank
                .map(|r| r.to_string())
                .unwrap_or_else(|| "-".to_string());
            let mut row = vec![rank, score.model.clone()];
            for metric in &all_metrics {
                let val = score
                    .metrics
                    .get(metric)
                    .map(format_metric)
                    .unwrap_or_else(|| "-".to_string());
                row.push(val);
            }
            result.push_str(&format!("| {} |\n", row.join(" | ")));
        }

        result.push('\n');
    }

    Ok(result.trim_end().to_string())
}

fn format_status(status: &SourceStatus) -> String {
    match status {
        SourceStatus::Ok => "OK".green().to_string(),
        SourceStatus::Cached => "Cached".green().to_string(),
        SourceStatus::Unavailable => "Unavailable".yellow().to_string(),
        SourceStatus::Error(e) => format!("Error: {}", e).red().to_string(),
    }
}

fn format_metric(value: &MetricValue) -> String {
    match value {
        MetricValue::Float(f) => format!("{:.2}", f),
        MetricValue::Int(i) => i.to_string(),
        MetricValue::Text(t) => t.clone(),
    }
}

fn pad(s: &str, width: usize) -> String {
    format!("{:width$}", s, width = width)
}
