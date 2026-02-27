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
use models::{PondusOutput, QueryInfo};
use output::OutputFormat;
use sources::Source;

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

    let command = cli.command.unwrap_or(Command::Rank { top: None });

    match command {
        Command::Rank { top } => cmd_rank(&config, &cache, &aliases, format, top),
        Command::Check { model } => cmd_check(&config, &cache, &aliases, format, &model),
        Command::Compare { model1, model2 } => {
            cmd_compare(&config, &cache, &aliases, format, &model1, &model2)
        }
        Command::Sources => cmd_sources(&config, &cache, format),
        Command::Refresh => {
            cache.clear()?;
            eprintln!("Cache cleared. Re-fetching all sources...");
            cmd_rank(&config, &cache, &aliases, format, None)
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
) -> Result<()> {
    let mut results = fetch_all(config, cache);
    if let Some(n) = top {
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
