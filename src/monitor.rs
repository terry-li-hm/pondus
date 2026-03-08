use anyhow::Result;
use chrono::Local;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::alias::AliasMap;
use crate::cache::Cache;
use crate::config::Config;
use crate::models::MetricValue;

#[derive(Subcommand)]
pub enum MonitorCommand {
    /// Add a model to the watchlist
    Add {
        /// Model name (canonical or alias)
        model: String,
    },
    /// List all watched models
    List,
    /// Remove a model from the watchlist
    Remove {
        /// Model name
        model: String,
    },
    /// Poll sources for new benchmark data for watched models
    Check,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MonitorState {
    pub watched: Vec<WatchedModel>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WatchedModel {
    pub model: String,
    pub added_at: String,
    pub last_seen: HashMap<String, String>,
}

impl MonitorState {
    fn load() -> Result<Self> {
        let path = state_file_path()?;
        if !path.exists() {
            return Ok(MonitorState { watched: vec![] });
        }
        let content = fs::read_to_string(path)?;
        let state = serde_json::from_str(&content)?;
        Ok(state)
    }

    fn save(&self) -> Result<()> {
        let path = state_file_path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

fn state_file_path() -> Result<PathBuf> {
    let mut path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local").join("share"));
    path.push("pondus");
    path.push("monitors.json");
    Ok(path)
}

pub fn handle_command(
    subcommand: MonitorCommand,
    config: &Config,
    cache: &Cache,
    aliases: &AliasMap,
) -> Result<()> {
    match subcommand {
        MonitorCommand::Add { model } => add_model(model, aliases),
        MonitorCommand::List => list_models(),
        MonitorCommand::Remove { model } => remove_model(model, aliases),
        MonitorCommand::Check => check_models(config, cache, aliases),
    }
}

fn add_model(model: String, aliases: &AliasMap) -> Result<()> {
    let canonical = aliases.resolve(&model);
    let mut state = MonitorState::load()?;
    
    if state.watched.iter().any(|w| w.model == canonical) {
        println!("Already watching {}.", canonical);
        return Ok(());
    }

    state.watched.push(WatchedModel {
        model: canonical.clone(),
        added_at: Local::now().format("%Y-%m-%d").to_string(),
        last_seen: HashMap::new(),
    });

    state.save()?;
    println!("Watching {}. Run 'pondus monitor check' to poll now.", canonical);
    Ok(())
}

fn list_models() -> Result<()> {
    let state = MonitorState::load()?;
    if state.watched.is_empty() {
        println!("No models on the watchlist.");
        return Ok(());
    }

    println!("{:<25} {:<15} {}", "MODEL", "ADDED", "SOURCES WITH DATA");
    for w in state.watched {
        let sources = if w.last_seen.is_empty() {
            "no data yet".to_string()
        } else {
            let mut s: Vec<_> = w.last_seen.keys().cloned().collect();
            s.sort();
            s.join(", ")
        };
        println!("{:<25} {:<15} {}", w.model, w.added_at, sources);
    }
    Ok(())
}

fn remove_model(model: String, aliases: &AliasMap) -> Result<()> {
    let canonical = aliases.resolve(&model);
    let mut state = MonitorState::load()?;
    let initial_len = state.watched.len();
    state.watched.retain(|w| w.model != canonical);

    if state.watched.len() == initial_len {
        println!("Model '{}' was not on the watchlist.", model);
    } else {
        state.save()?;
        println!("Removed '{}' from watchlist.", canonical);
    }
    Ok(())
}

fn check_models(config: &Config, cache: &Cache, aliases: &AliasMap) -> Result<()> {
    let mut state = MonitorState::load()?;
    if state.watched.is_empty() {
        println!("No models on the watchlist.");
        return Ok(());
    }

    let results = crate::fetch_all(config, cache);
    let today = Local::now().format("%Y-%m-%d").to_string();
    let mut state_changed = false;

    for w in &mut state.watched {
        let mut new_data = Vec::new();
        let canonical = &w.model;

        for r in &results {
            if let Some(s) = r.scores.iter().find(|s| {
                s.model.to_lowercase() == *canonical || aliases.matches(&s.source_model_name, canonical)
            }) {
                if !w.last_seen.contains_key(&r.source) {
                    w.last_seen.insert(r.source.clone(), today.clone());
                    state_changed = true;
                    
                    let metric_info = if let Some(rank) = s.rank {
                        format!("rank {}/{}", rank, r.scores.len())
                    } else {
                        s.metrics.iter()
                            .next()
                            .map(|(k, v)| format!("{} = {}", k, format_metric(v)))
                            .unwrap_or_else(|| "no metrics".to_string())
                    };
                    new_data.push((r.source.clone(), metric_info));
                }
            }
        }

        if new_data.is_empty() {
            println!("No new benchmark data for {}.", w.model);
        } else {
            println!("Found new data for {}!", w.model);
            for (source, info) in &new_data {
                let msg = format!("pondus: new benchmark data for {}
{}: {}", w.model, source, info);
                println!("  {}: {}", source, info);
                
                // Notify via Telegram Bot API directly (cross-platform, no deltos dependency)
                let token = std::env::var("TELEGRAM_BOT_TOKEN").ok();
                let notified = token.as_deref().map(|tok| {
                    let url = format!("https://api.telegram.org/bot{}/sendMessage", tok);
                    reqwest::blocking::Client::new()
                        .post(&url)
                        .json(&serde_json::json!({
                            "chat_id": 6201770409i64,
                            "text": msg
                        }))
                        .send()
                        .is_ok()
                }).unwrap_or(false);
                if !notified {
                    // Fallback: try deltos (macOS), then just print
                    let sent = Command::new("deltos").arg(&msg).spawn()
                        .and_then(|mut c| c.wait()).is_ok();
                    if !sent {
                        println!("  [notify] {}", msg);
                    }
                }
            }
        }
    }

    if state_changed {
        state.save()?;
    }

    Ok(())
}

fn format_metric(v: &MetricValue) -> String {
    match v {
        MetricValue::Float(f) => format!("{:.2}", f),
        MetricValue::Int(i) => i.to_string(),
        MetricValue::Text(t) => t.clone(),
    }
}
