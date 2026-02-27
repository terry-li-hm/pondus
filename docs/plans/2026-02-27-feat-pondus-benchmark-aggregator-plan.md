---
title: "feat: Pondus benchmark aggregator CLI"
type: feat
status: completed
date: 2026-02-27
origin: docs/brainstorms/2026-02-27-pondus-design-brainstorm.md
---

# feat: Pondus Benchmark Aggregator CLI

## Overview

Rust CLI that aggregates AI model benchmark results from 8 trusted sources into structured JSON. Primary consumer is an AI agent (Claude Code), not a human. The tool fetches reliably; interpretation lives in the agent layer.

(see brainstorm: docs/brainstorms/2026-02-27-pondus-design-brainstorm.md)

## Proposed Solution

Follow the established oura-cli/pplx pattern: `clap` derive for CLI, `reqwest::blocking` for HTTP, `serde` for JSON, `anyhow` for errors, `dirs` for XDG paths. Add `toml` for config/alias parsing and a file-based cache layer adapted from lustro's atomic-write pattern.

Each source implements a `Source` trait that normalises heterogeneous data into a common `ModelScore` schema. Scrape-only sources shell out to `agent-browser` via `std::process::Command`.

## Technical Considerations

### Module Structure

```
src/
  main.rs          — Cli/Command structs, subcommand dispatch
  config.rs        — TOML config loading (~/.config/pondus/config.toml)
  cache.rs         — File-based cache (read/write/invalidate, 24h TTL)
  models.rs        — Common types: ModelScore, SourceResult, SourceStatus
  alias.rs         — Model name resolution (models.toml → canonical ↔ per-source)
  output.rs        — JSON (default), table, markdown formatters
  sources/
    mod.rs         — Source trait definition + registry
    aa.rs          — Artificial Analysis (REST API)
    arena.rs       — LM Arena (community JSON from GitHub)
    swebench.rs    — SWE-bench (GitHub JSON)
    swebench_r.rs  — SWE-rebench (agent-browser scrape)
    livebench.rs   — LiveBench (HuggingFace/GitHub)
    aider.rs       — Aider (GitHub YAML)
    tbench.rs      — Terminal-Bench (HuggingFace)
    seal.rs        — SEAL (agent-browser scrape)
```

### Source Trait

```rust
pub trait Source {
    fn name(&self) -> &str;
    fn fetch(&self, config: &Config, cache: &Cache) -> Result<SourceResult>;
}

pub struct SourceResult {
    pub source: String,
    pub fetched_at: DateTime<Utc>,
    pub status: SourceStatus,        // Ok | Unavailable | Cached | Error(String)
    pub scores: Vec<ModelScore>,
}

pub struct ModelScore {
    pub model: String,               // canonical name from alias map
    pub source_model_name: String,   // original name from the source
    pub metrics: HashMap<String, MetricValue>,  // flexible per-source metrics
    pub rank: Option<u32>,
}

pub enum MetricValue {
    Float(f64),
    Int(i64),
    Text(String),
}
```

### JSON Output Schema

```json
{
  "timestamp": "2026-02-27T14:30:00Z",
  "query": { "type": "rank", "top": 10 },
  "sources": [
    {
      "name": "artificial-analysis",
      "status": "ok",
      "fetched_at": "2026-02-27T14:30:00Z",
      "scores": [
        {
          "model": "claude-opus-4.6",
          "source_model_name": "Claude Opus 4.6",
          "metrics": {
            "intelligence_index": 89.2,
            "price_input_per_mtok": 5.0,
            "price_output_per_mtok": 25.0,
            "speed_tps": 42.3
          },
          "rank": 1
        }
      ]
    },
    {
      "name": "seal",
      "status": "unavailable",
      "fetched_at": null,
      "scores": []
    }
  ]
}
```

### Cache Implementation

Adapted from lustro's atomic-write pattern (see learnings: persistent-test-caching-40x-speedup):

```
~/.cache/pondus/
  aa.json            — cached response + timestamp
  arena.json
  swebench.json
  ...
  meta.json          — last full refresh timestamp
```

Each file: `{ "fetched_at": "...", "ttl_hours": 24, "data": <source response> }`

Write atomically: temp file → fsync → rename. Prevents partial writes on crash.

### Config File

```toml
# ~/.config/pondus/config.toml

[sources.artificial-analysis]
api_key = "aa-..."

[sources.seal]
agent_browser_path = "/usr/local/bin/agent-browser"  # optional override

[cache]
ttl_hours = 24  # override default

[alias]
path = ""  # optional override for models.toml location
```

### Alias Map (models.toml)

Shipped in-crate via `include_str!`, user can override via config dir.

```toml
[claude-opus-4.6]
canonical = "claude-opus-4.6"
aliases = [
  "Claude Opus 4.6",
  "claude-opus-4-6",
  "anthropic/claude-opus-4.6",
  "Opus 4.6",
]

[gpt-5.2]
canonical = "gpt-5.2"
aliases = [
  "GPT-5.2",
  "gpt-5.2-pro",
  "openai/gpt-5.2",
]
```

### Scrape Sources (agent-browser)

Shell out via `std::process::Command`:

```rust
fn fetch_via_browser(url: &str, config: &Config) -> Result<String> {
    let browser = config.agent_browser_path
        .as_deref()
        .unwrap_or("agent-browser");

    let output = Command::new(browser)
        .args(["open", url])
        .output()
        .context("agent-browser not found")?;

    if !output.status.success() {
        return Ok(String::new());  // graceful degradation
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

Key gotchas from learnings:
- Always re-snapshot after any agent-browser action (refs shift)
- Use `fill` not `type` for React inputs
- `resize_window` before `read_page`
- Some sites (SEAL) may serve skeleton HTML to non-browsers — agent-browser handles this

### Dependencies

```toml
[dependencies]
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
clap = { version = "4", features = ["derive"] }
dirs = "6"
reqwest = { version = "0.12", features = ["blocking", "json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"       # Aider YAML parsing
toml = "0.8"              # config + alias map
owo-colors = "4"          # table format output

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

## Implementation Phases

### Phase 1: Foundation

Core scaffolding — CLI skeleton, config, cache, source trait, output schema. No real sources yet.

- [ ] `main.rs` — clap derive with all subcommands (rank, check, compare, sources, refresh)
- [ ] `config.rs` — load from `~/.config/pondus/config.toml`, fallback defaults
- [ ] `cache.rs` — read/write/invalidate per-source, atomic writes, TTL check
- [ ] `models.rs` — `ModelScore`, `SourceResult`, `SourceStatus`, `MetricValue` types
- [ ] `alias.rs` — load `models.toml` (bundled + user override), resolve canonical ↔ aliases
- [ ] `output.rs` — JSON serialiser (default), stub table/markdown formatters
- [ ] `sources/mod.rs` — `Source` trait, source registry (returns all sources)
- [ ] One mock source for end-to-end testing

**Done when:** `pondus rank --format json` returns valid JSON from the mock source.

### Phase 2: First Real Source (Artificial Analysis)

Proves the pattern end-to-end with the richest data source.

- [ ] `sources/aa.rs` — REST API client (`x-api-key` header, free tier)
- [ ] Parse AA's model response into `ModelScore` with metrics: intelligence_index, price, speed
- [ ] Cache integration (fetch once, serve from cache within TTL)
- [ ] Alias resolution for AA model names
- [ ] `pondus check claude-opus-4.6` returns real AA data

**Done when:** `pondus rank` and `pondus check <model>` work with live AA data.

### Phase 3: Structured Sources (GitHub + HuggingFace)

Five sources, all structured data — can be built in parallel.

- [ ] `sources/swebench.rs` — fetch `leaderboards.json` from GitHub raw
- [ ] `sources/arena.rs` — fetch `output/scores.json` from `nakasyou/lmarena-history`
- [ ] `sources/aider.rs` — fetch YAML files from `Aider-AI/aider` raw GitHub
- [ ] `sources/livebench.rs` — fetch from LiveBench GitHub/HuggingFace
- [ ] `sources/tbench.rs` — fetch from Terminal-Bench HuggingFace dataset

**Done when:** `pondus rank` aggregates all 6 structured sources. `pondus sources` shows status for each.

### Phase 4: Scrape Sources

Two sources via agent-browser. Graceful degradation if agent-browser unavailable.

- [ ] `sources/seal.rs` — shell out to agent-browser, parse SEAL leaderboard HTML
- [ ] `sources/swebench_r.rs` — shell out to agent-browser, parse SWE-rebench scores page
- [ ] Handle "unavailable" status cleanly in output
- [ ] Test with and without agent-browser installed

**Done when:** All 8 sources work. Missing agent-browser → those sources show "unavailable", tool still works.

### Phase 5: Subcommands + Polish

All subcommands working, output formats complete.

- [ ] `rank` — aggregate all sources, sort by average rank or configurable metric
- [ ] `check <model>` — single model across all sources, alias resolution
- [ ] `compare <model1> <model2>` — side-by-side
- [ ] `sources` — list all sources with status (ok/cached/unavailable/error)
- [ ] `refresh` — clear cache, re-fetch all
- [ ] `--format table` — terminal table with owo-colors
- [ ] `--format markdown` — pipe-friendly markdown table
- [ ] `--refresh` global flag — bypass cache on any command
- [ ] `--top N` flag on rank

**Done when:** All subcommands work. `pondus rank --top 5 --format table` produces a readable terminal table.

### Phase 6: Open-Source Release

- [x] README.md — installation, usage, source list, contributing guide for alias map PRs
- [x] LICENSE (MIT, already set in Cargo.toml)
- [x] GitHub Actions CI (cargo test, cargo clippy, cargo fmt)
- [x] Seed `models.toml` with current frontier models (~25 models)
- [x] `cargo publish` real version (0.2.0 — 0.1.0 was the placeholder)
- [ ] Update PyPI placeholder or remove it (Rust-only distribution) — deferred, low priority

**Done when:** `cargo install pondus` works for anyone. README documents all commands.

## Acceptance Criteria

- [ ] `pondus rank --format json` returns valid JSON with data from all available sources
- [ ] `pondus check <model>` returns scores across sources for a known model, marks gaps
- [ ] `pondus compare <model1> <model2>` shows side-by-side comparison
- [ ] `pondus sources` shows status of each source (ok/cached/unavailable)
- [ ] Cache works: second run within 24h serves from disk, `--refresh` bypasses
- [ ] Scrape sources degrade gracefully when agent-browser is not installed
- [ ] Alias resolution works: `pondus check opus-4.6` matches "Claude Opus 4.6" from AA
- [ ] Binary installs cleanly via `cargo install pondus`

## Dependencies & Risks

- **LM Arena community JSON** is fragile — `nakasyou/lmarena-history` can break if Arena changes internal format. Mitigation: graceful degradation, monitor for breakage.
- **AA free tier** is 1K req/day — more than enough for personal use, may need commercial tier if tool gets popular.
- **Alias map maintenance** — every model release needs a PR. Mitigation: keep format simple (TOML), document contribution process in README.
- **agent-browser dependency** for 2 sources — not installable via cargo. Mitigation: graceful degradation, clear error message pointing to installation.

## Sources & References

- **Origin brainstorm:** [docs/brainstorms/2026-02-27-pondus-design-brainstorm.md](../brainstorms/2026-02-27-pondus-design-brainstorm.md) — all key decisions (8 sources, JSON-first output, 24h cache, alias map, agent-first design)
- **Rust CLI patterns:** oura-cli (`~/code/oura-cli/`), pplx (`~/code/pplx/`) — clap/reqwest/serde/anyhow stack
- **Cache pattern:** lustro (`~/code/lustro/`) — atomic write with tempfile+fsync+rename
- **Benchmark sources reference:** `~/docs/solutions/ai-model-evaluation-sources.md`
- **Publishing gotchas:** `~/docs/solutions/package-registry-namespace-squatting.md`
- **Test caching pattern:** `~/docs/solutions/testing-patterns/persistent-test-caching-40x-speedup-20260126.md`
- **agent-browser gotchas:** `~/docs/solutions/browser-automation/` (fill vs type, refs shift, resize window)
