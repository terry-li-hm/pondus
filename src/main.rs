mod alias;
mod cache;
mod config;
mod models;
mod output;
mod sources;

use alias::{AliasMap, MatchKind};
use anyhow::Result;
use cache::Cache;
use chrono::Utc;
use clap::{Parser, Subcommand};
use config::Config;
use models::{
    MetricValue, ModelScore, PondusOutput, QueryInfo, SourceResult, SourceStatus, SourceTag,
};
use output::OutputFormat;
use sources::Source;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

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
        /// Filter to source tags: reasoning, coding, agentic, general
        #[arg(long)]
        tag: Option<String>,
        /// Comma-separated source names (case-insensitive)
        #[arg(long)]
        sources: Option<String>,
        /// Produce a combined leaderboard across sources
        #[arg(long)]
        aggregate: bool,
        /// Minimum number of sources a model must appear in (default: 2 when --aggregate is set)
        #[arg(long)]
        min_sources: Option<usize>,
        /// Show models excluded by --min-sources threshold when aggregating
        #[arg(long)]
        show_excluded: bool,
    },
    /// Check a single model across all sources
    Check {
        /// Model name (canonical or alias)
        model: String,
        #[arg(long)]
        show_matches: bool,
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
        tag: None,
        sources: None,
        aggregate: false,
        min_sources: None,
        show_excluded: false,
    });

    match command {
        Command::Rank {
            top,
            source,
            tag,
            sources,
            aggregate,
            min_sources,
            show_excluded,
        } => cmd_rank(
            &config,
            &cache,
            &aliases,
            format,
            top,
            source.as_deref(),
            tag.as_deref(),
            sources.as_deref(),
            aggregate,
            min_sources,
            show_excluded,
        ),
        Command::Check {
            model,
            show_matches,
        } => cmd_check(&config, &cache, &aliases, format, &model, show_matches),
        Command::Compare { model1, model2 } => {
            cmd_compare(&config, &cache, &aliases, format, &model1, &model2)
        }
        Command::Sources => cmd_sources(&config, &cache, format),
        Command::Refresh => {
            cache.clear()?;
            eprintln!("Cache cleared. Re-fetching all sources...");
            cmd_rank(
                &config, &cache, &aliases, format, None, None, None, None, false, None, false,
            )
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

fn source_tag_map(config: &Config) -> HashMap<String, Vec<SourceTag>> {
    let mut tags_by_source: HashMap<String, Vec<SourceTag>> = get_sources()
        .into_iter()
        .map(|source| (source.name().to_lowercase(), source.tags().to_vec()))
        .collect();

    for (source, tags) in &config.source_tags {
        let parsed_tags: Vec<SourceTag> = tags.iter().filter_map(|t| parse_source_tag(t)).collect();
        tags_by_source.insert(source.to_lowercase(), parsed_tags);
    }

    tags_by_source
}

fn parse_source_tag(tag: &str) -> Option<SourceTag> {
    match tag.trim().to_lowercase().as_str() {
        "reasoning" => Some(SourceTag::Reasoning),
        "coding" => Some(SourceTag::Coding),
        "agentic" => Some(SourceTag::Agentic),
        "general" => Some(SourceTag::General),
        _ => None,
    }
}

fn source_tag_name(tag: &SourceTag) -> &'static str {
    match tag {
        SourceTag::Reasoning => "reasoning",
        SourceTag::Coding => "coding",
        SourceTag::Agentic => "agentic",
        SourceTag::General => "general",
    }
}

fn cmd_rank(
    config: &Config,
    cache: &Cache,
    _aliases: &AliasMap,
    format: OutputFormat,
    top: Option<usize>,
    source_filter: Option<&str>,
    tag_filter: Option<&str>,
    sources_filter: Option<&str>,
    aggregate: bool,
    min_sources: Option<usize>,
    show_excluded: bool,
) -> Result<()> {
    let mut results = fetch_all(config, cache);

    if let Some(tag_name) = tag_filter {
        let requested_tag = parse_source_tag(tag_name).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown tag: '{tag_name}'. Expected one of: reasoning, coding, agentic, general"
            )
        })?;
        let tags_by_source = source_tag_map(config);
        results.retain(|result| {
            tags_by_source
                .get(&result.source.to_lowercase())
                .is_some_and(|tags| tags.contains(&requested_tag))
        });
    }

    let merged_sources = sources_filter.or(source_filter);
    if let Some(source_list) = merged_sources {
        let requested_sources: HashSet<String> = source_list
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(|name| name.to_lowercase())
            .collect();

        if requested_sources.is_empty() {
            anyhow::bail!("--sources/--source requires at least one source name");
        }

        let filtered: Vec<_> = results
            .into_iter()
            .filter(|r| requested_sources.contains(&r.source.to_lowercase()))
            .collect();

        if filtered.is_empty() {
            let available = get_sources()
                .into_iter()
                .map(|s| s.name().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("No matching sources in '{source_list}'. Available sources: {available}");
        }

        results = filtered;
    }

    if aggregate {
        let threshold = min_sources.unwrap_or(2);
        let excluded_for_count = if show_excluded {
            Vec::new()
        } else {
            excluded_models(results.as_slice(), threshold)
        };
        let (mut aggregated, excluded_models) = aggregate_results(results, threshold, show_excluded);
        let excluded_models = if show_excluded {
            excluded_models
        } else {
            excluded_for_count
        };
        if !excluded_models.is_empty() {
            eprintln!(
                "{} models excluded (appeared in fewer than {} sources). Use --show-excluded to list.",
                excluded_models.len(),
                threshold
            );
            if show_excluded {
                for (model, count) in &excluded_models {
                    eprintln!("  {} ({})", model, count);
                }
            }
        }
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
        source_tags: None,
    };

    println!("{}", output::render(&output, format)?);
    Ok(())
}

fn aggregate_results(
    results: Vec<SourceResult>,
    min_sources: usize,
    show_excluded: bool,
) -> (SourceResult, Vec<(String, usize)>) {
    let mut totals: HashMap<String, Vec<f64>> = HashMap::new();

    for source in results {
        let total_in_source = source.scores.len();
        if total_in_source == 0 {
            continue;
        }

        for score in source.scores {
            let Some(rank) = score.rank else {
                continue;
            };

            let entry = totals.entry(score.model).or_default();
            entry.push(percentile(rank, total_in_source));
        }
    }

    let mut excluded: Vec<(String, usize)> = Vec::new();
    let mut rows: Vec<(String, f64, f64, usize)> = totals
        .into_iter()
        .filter_map(|(model, percentiles)| {
            let count = percentiles.len();
            if count < min_sources {
                if show_excluded {
                    excluded.push((model, count));
                }
                None
            } else {
                let avg = percentiles.iter().sum::<f64>() / count as f64;
                let spread = std_dev(&percentiles);
                Some((model, avg, spread, count))
            }
        })
        .collect();

    rows.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    excluded.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let scores = rows
        .into_iter()
        .enumerate()
        .map(|(i, (model, avg_percentile, spread, sources_count))| ModelScore {
            model: model.clone(),
            source_model_name: model,
            metrics: HashMap::from([
                (
                    "avg_percentile".to_string(),
                    MetricValue::Float(avg_percentile),
                ),
                ("spread".to_string(), MetricValue::Float(spread)),
                (
                    "sources_count".to_string(),
                    MetricValue::Int(sources_count as i64),
                ),
            ]),
            rank: Some((i + 1) as u32),
        })
        .collect();

    (
        SourceResult {
            source: "aggregate".to_string(),
            fetched_at: None,
            status: SourceStatus::Ok,
            scores,
        },
        if show_excluded { excluded } else { Vec::new() },
    )
}

fn excluded_models(results: &[SourceResult], min_sources: usize) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();

    for source in results {
        for score in &source.scores {
            if score.rank.is_none() {
                continue;
            }
            *counts.entry(score.model.clone()).or_insert(0) += 1;
        }
    }

    let mut excluded: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(_, count)| *count < min_sources)
        .collect();
    excluded.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    excluded
}

fn percentile(rank: u32, total: usize) -> f64 {
    if total <= 1 {
        1.0
    } else {
        (total as f64 - rank as f64) / (total as f64 - 1.0)
    }
}

fn std_dev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64).sqrt()
}

fn cmd_check(
    config: &Config,
    cache: &Cache,
    aliases: &AliasMap,
    format: OutputFormat,
    model: &str,
    show_matches: bool,
) -> Result<()> {
    let canonical = aliases.resolve(model);
    let results = fetch_all(config, cache);

    let mut match_lines = Vec::new();
    let filtered: Vec<_> = results
        .into_iter()
        .map(|mut r| {
            r.scores.retain(|s| {
                s.model.to_lowercase() == canonical
                    || aliases.matches(&s.source_model_name, &canonical)
            });
            if show_matches {
                for s in &r.scores {
                    match_lines.push(aliases.explain(&r.source, &s.source_model_name, &canonical));
                }
            }
            r
        })
        .collect();

    if show_matches {
        for m in match_lines {
            eprintln!(
                "[{}]   {:?}  ->  {:?}  ({})",
                m.source_name,
                m.source_model_name,
                m.canonical,
                match_kind_str(&m.match_kind)
            );
        }
    }

    let output = PondusOutput {
        timestamp: Utc::now(),
        query: QueryInfo {
            query_type: "check".into(),
            model: Some(canonical),
            models: None,
            top: None,
        },
        sources: filtered,
        source_tags: None,
    };

    println!("{}", output::render(&output, format)?);
    Ok(())
}

fn match_kind_str(kind: &MatchKind) -> &'static str {
    match kind {
        MatchKind::Exact => "exact",
        MatchKind::Alias => "alias",
        MatchKind::Prefix => "prefix",
        MatchKind::NoMatch => "no-match",
    }
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
        source_tags: None,
    };

    println!("{}", output::render(&output, format)?);
    Ok(())
}

fn cmd_sources(config: &Config, cache: &Cache, format: OutputFormat) -> Result<()> {
    let results = fetch_all(config, cache);
    let source_tags = source_tag_map(config)
        .into_iter()
        .map(|(source, tags)| {
            let names = tags
                .iter()
                .map(source_tag_name)
                .map(str::to_string)
                .collect();
            (source, names)
        })
        .collect::<HashMap<_, _>>();

    let output = PondusOutput {
        timestamp: Utc::now(),
        query: QueryInfo {
            query_type: "sources".into(),
            model: None,
            models: None,
            top: None,
        },
        sources: results,
        source_tags: Some(source_tags),
    };

    println!("{}", output::render(&output, format)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::percentile;

    #[test]
    fn percentile_rank_1_of_10_is_1() {
        assert!((percentile(1, 10) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn percentile_rank_10_of_10_is_0() {
        assert!((percentile(10, 10) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn percentile_rank_1_of_1_is_1() {
        assert!((percentile(1, 1) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn percentile_rank_5_of_10_matches_expected_formula() {
        let expected = (10.0 - 5.0) / (10.0 - 1.0);
        assert!((percentile(5, 10) - expected).abs() < f64::EPSILON);
    }
}
