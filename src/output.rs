use crate::models::PondusOutput;
use anyhow::Result;

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
    // TODO: implement rich table with owo-colors
    // For now, fall back to JSON
    render_json(output)
}

fn render_markdown(output: &PondusOutput) -> Result<String> {
    // TODO: implement markdown table
    // For now, fall back to JSON
    render_json(output)
}
