# Pondus Design Brainstorm

**Date:** 2026-02-27
**Status:** Complete
**Participants:** Terry, Claude

## What We're Building

An open-source Rust CLI that aggregates AI model benchmark results from multiple trusted sources into structured JSON. The primary consumer is an AI agent (Claude Code), not a human reading terminal output.

**Name:** pondus (Latin: "weight, gravity, judgment" — root of "ponder")
**Registries claimed:** crates.io, PyPI, GitHub (terry-li-hm/pondus)

## Why This Approach

The core insight: Terry doesn't read benchmark dashboards — he asks Claude. So the tool's job is **reliable data fetching**, not display. The opinion/interpretation layer lives in the agent, not the tool. This simplifies the design dramatically:

- JSON output, not pretty tables (agent parses it)
- No composite scoring (agent interprets contextually, informed by user's routing needs)
- No config for weights or preferences (that lives in CLAUDE.md and the agent's judgment)
- The vault artifact is Claude's analysis note, not raw data

## Key Decisions

### Data Sources (8 total)

| Source | Method | Data Available |
|--------|--------|----------------|
| Artificial Analysis | REST API (free tier, 1K req/day, `x-api-key` header) | Benchmark scores, pricing, speed |
| LM Arena | Community JSON (`nakasyou/lmarena-history` GitHub) | ELO scores — fragile, not official |
| SWE-bench | GitHub JSON (`SWE-bench/swe-bench.github.io` → `data/leaderboards.json`) | Resolve rates |
| SWE-rebench | Scrape (scores not in dataset, only tasks on HF) | Cost-per-resolved, resolve rates |
| LiveBench | HuggingFace datasets + GitHub (`LiveBench/LiveBench`) | Per-category scores |
| Aider | GitHub YAML (`Aider-AI/aider/aider/website/_data/*.yml`) | Edit/polyglot/refactor leaderboards |
| Terminal-Bench | HuggingFace (`sabhay/terminal-bench-2-leaderboard`) | Per-agent results |
| SEAL | Scrape (private by design) | Coding leaderboard scores |

**Scrape-only sources (SEAL, SWE-rebench):** Use `agent-browser` locally. Graceful degradation if unavailable — those sources return "unavailable", tool still works.

### Output

- **Default:** JSON to stdout
- **Flags:** `--format table`, `--format markdown` as options (nice-to-have, not core)
- **Primary consumer:** Claude Code reads the JSON, analyses it, writes a vault note in natural language

### Caching

- **TTL:** 24 hours file-based cache (`~/.cache/pondus/`, one JSON per source with timestamp)
- **Bypass:** `--refresh` flag to force re-fetch
- **Rationale:** Same session never hits network twice; next session always gets fresh data

### Model Name Resolution

- **Built-in alias map:** TOML file in the repo mapping canonical names to source-specific variants
- **No fuzzy matching:** Explicit > clever. Unknown models return "unknown" rather than guessing wrong
- **Maintenance:** Community PRs to update the alias map when new models drop

### Subcommands

```
pondus rank [--top N]              # All models ranked across sources
pondus check <model>               # Single model scorecard across all sources
pondus compare <model1> <model2>   # Head-to-head comparison
pondus sources                     # List available sources and their status
pondus refresh                     # Force re-fetch all sources
```

### Usage Triggers

1. **New model drop:** `pondus check <model>` — returns whatever data is available, marks gaps with "pending" or "unavailable"
2. **Weekly/monthly review:** `pondus rank` — full leaderboard, feeds into `/weekly` or model routing updates
3. **Model selection:** `pondus compare` — when deciding between models for a specific use case

### Expectations for New Models

Benchmarks lag behind releases (Arena: 24-48h, AA: ~1 week, SWE-bench: weeks). Pondus is most useful 1-2 weeks after a model drops, not day one. The "check" command honestly shows what's available.

## Architecture Notes

- **Language:** Rust (open-source single-binary distribution via `cargo install pondus`)
- **Sister tools:** lustro (AI news), consilium (multi-model deliberation), hexis (meta-cognitive framework)
- **Config:** `~/.config/pondus/config.toml` — API keys (AA), agent-browser path, cache TTL override
- **Alias map:** `models.toml` shipped in the crate, user can override via config dir

## Open Questions

*None — all resolved during brainstorm.*

## What This Is NOT

- Not a benchmark runner (doesn't evaluate models, just aggregates published results)
- Not a dashboard (no TUI, no live updating, no charts)
- Not opinionated in code (the opinion comes from Claude's interpretation, not a weighted score)

## Next Steps

`/workflows:plan` to create implementation plan.
