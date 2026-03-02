mod alias;
mod cache;
mod config;
mod models;
mod output;
mod sources;

use alias::AliasMap;
use anyhow::Result;
use cache::Cache;
use chrono::Utc;
use clap::{Parser, Subcommand};
use config::Config;
use models::{MetricValue, ModelScore, PondusOutput, QueryInfo, SourceResult, SourceStatus};
use output::OutputFormat;
use sources::Source;
use std::cmp::Ordering;
use std::collections::HashMap;

#[derive(Parser)]
#[command(
    name = "pondus",
    version,
    about = "Opinionated AI model benchmark aggregator"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Output format: json (default), table, markdown
    #[arg(long, default_value = "json", global = true)]
    format: String,

    /// Bypass cache and re-fetch all sources
    #[arg(long, global = true)]
    refresh: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Rank all models across sources
    Rank {
        /// Show top N models
        #[arg(long)]
        top: Option<usize>,
        /// Filter to a single source name (case-insensitive)
        #[arg(long)]
        source: Option<String>,
        /// Produce a combined leaderboard across sources
        #[arg(long)]
        aggregate: bool,
    },
    /// Check a single model across all sources
    Check {
        /// Model name (canonical or alias)
        model: String,
    },
    /// Compare two models head-to-head
    Compare {
        /// First model
        model1: String,
        /// Second model
        model2: String,
    },
    /// List all sources and their status
    Sources,
    /// Force re-fetch all sources (clears cache)
    Refresh,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    let cache = Cache::new(config.cache.ttl_hours);
    let aliases = AliasMap::load(config.alias.path.as_deref())?;
    let format = OutputFormat::from_str(&cli.format)?;

    if cli.refresh {
        cache.clear()?;
    }

    let command = cli.command.unwrap_or(Command::Rank {
        top: None,
        source: None,
        aggregate: false,
    });

    match command {
        Command::Rank {
            top,
            source,
            aggregate,
        } => cmd_rank(
            &config,
            &cache,
            &aliases,
            format,
            top,
            source.as_deref(),
            aggregate,
        ),
        Command::Check { model } => cmd_check(&config, &cache, &aliases, format, &model),
        Command::Compare { model1, model2 } => {
            cmd_compare(&config, &cache, &aliases, format, &model1, &model2)
        }
        Command::Sources => cmd_sources(&config, &cache, format),
        Command::Refresh => {
            cache.clear()?;
            eprintln!("Cache cleared. Re-fetching all sources...");
            cmd_rank(&config, &cache, &aliases, format, None, None, false)
        }
    }
}

fn fetch_all(config: &Config, cache: &Cache) -> Vec<models::SourceResult> {
    let srcs = get_sources();
    srcs.iter()
        .map(|s| match s.fetch(config, cache) {
            Ok(result) => result,
            Err(e) => models::SourceResult {
                source: s.name().into(),
                fetched_at: None,
                status: models::SourceStatus::Error(e.to_string()),
                scores: vec![],
            },
        })
        .collect()
}

fn get_sources() -> Vec<Box<dyn Source>> {
    let real = sources::all_sources();
    if real.is_empty() {
        sources::all_sources_with_mock()
    } else {
        real
    }
}

fn cmd_rank(
    config: &Config,
    cache: &Cache,
    _aliases: &AliasMap,
    format: OutputFormat,
    top: Option<usize>,
    source_filter: Option<&str>,
    aggregate: bool,
) -> Result<()> {
    let mut results = fetch_all(config, cache);

    if let Some(source_name) = source_filter {
        let needle = source_name.to_lowercase();
        let filtered: Vec<_> = results
            .into_iter()
            .filter(|r| r.source.to_lowercase() == needle)
            .collect();

        if filtered.is_empty() {
            let available = get_sources()
                .into_iter()
                .map(|s| s.name().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!(
                "Source not found: '{source_name}'. Available sources: {available}"
            );
        }

        results = filtered;
    }

    if aggregate {
        let mut aggregated = aggregate_results(results);
        if let Some(n) = top {
            aggregated.scores.truncate(n);
        }
        results = vec![aggregated];
    } else if let Some(n) = top {
        for result in &mut results {
            result.scores.truncate(n);
        }
    }

    let output = PondusOutput {
        timestamp: Utc::now(),
        query: QueryInfo {
            query_type: "rank".into(),
            model: None,
            models: None,
            top,
        },
        sources: results,
    };

    println!("{}", output::render(&output, format)?);
    Ok(())
}

fn aggregate_results(results: Vec<SourceResult>) -> SourceResult {
    let mut totals: HashMap<String, (f64, usize)> = HashMap::new();

    for source in results {
        let total_in_source = source.scores.len();
        if total_in_source == 0 {
            continue;
        }

        let total = total_in_source as f64;
        for score in source.scores {
            let Some(rank) = score.rank else {
                continue;
            };

            let percentile = 1.0 - ((rank as f64 - 1.0) / total);
            let entry = totals.entry(score.model).or_insert((0.0, 0));
            entry.0 += percentile;
            entry.1 += 1;
        }
    }

    let mut rows: Vec<(String, f64, usize)> = totals
        .into_iter()
        .filter_map(|(model, (sum, count))| {
            if count == 0 {
                None
            } else {
                Some((model, sum / count as f64, count))
            }
        })
        .collect();

    rows.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    let scores = rows
        .into_iter()
        .enumerate()
        .map(|(i, (model, avg_percentile, sources_count))| ModelScore {
            model: model.clone(),
            source_model_name: model,
            metrics: HashMap::from([
                ("avg_percentile".to_string(), MetricValue::Float(avg_percentile)),
                (
                    "sources_count".to_string(),
                    MetricValue::Int(sources_count as i64),
                ),
            ]),
            rank: Some((i + 1) as u32),
        })
        .collect();

    SourceResult {
        source: "aggregate".to_string(),
        fetched_at: None,
        status: SourceStatus::Ok,
        scores,
    }
}

fn cmd_check(
    config: &Config,
    cache: &Cache,
    aliases: &AliasMap,
    format: OutputFormat,
    model: &str,
) -> Result<()> {
    let canonical = aliases.resolve(model);
    let results = fetch_all(config, cache);

    let filtered: Vec<_> = results
        .into_iter()
        .map(|mut r| {
            r.scores.retain(|s| {
                s.model.to_lowercase() == canonical
                    || aliases.matches(&s.source_model_name, &canonical)
            });
            r
        })
        .collect();

    let output = PondusOutput {
        timestamp: Utc::now(),
        query: QueryInfo {
            query_type: "check".into(),
            model: Some(canonical),
            models: None,
            top: None,
        },
        sources: filtered,
    };

    println!("{}", output::render(&output, format)?);
    Ok(())
}

fn cmd_compare(
    config: &Config,
    cache: &Cache,
    aliases: &AliasMap,
    format: OutputFormat,
    model1: &str,
    model2: &str,
) -> Result<()> {
    let c1 = aliases.resolve(model1);
    let c2 = aliases.resolve(model2);
    let results = fetch_all(config, cache);

    let filtered: Vec<_> = results
        .into_iter()
        .map(|mut r| {
            r.scores.retain(|s| {
                let resolved = aliases.resolve(&s.source_model_name);
                resolved == c1 || resolved == c2
            });
            r
        })
        .collect();

    let output = PondusOutput {
        timestamp: Utc::now(),
        query: QueryInfo {
            query_type: "compare".into(),
            model: None,
            models: Some(vec![c1, c2]),
            top: None,
        },
        sources: filtered,
    };

    println!("{}", output::render(&output, format)?);
    Ok(())
}

fn cmd_sources(config: &Config, cache: &Cache, format: OutputFormat) -> Result<()> {
    let results = fetch_all(config, cache);

    let output = PondusOutput {
        timestamp: Utc::now(),
        query: QueryInfo {
            query_type: "sources".into(),
            model: None,
            models: None,
            top: None,
        },
        sources: results,
    };

    println!("{}", output::render(&output, format)?);
    Ok(())
}
