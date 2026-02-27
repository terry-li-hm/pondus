# pondus

Opinionated AI model benchmark aggregator.

[![crates.io](https://img.shields.io/crates/v/pondus.svg)](https://crates.io/crates/pondus)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

## What it does

Aggregates AI model benchmark data from 8 trusted sources into a unified JSON schema. Designed for AI agents (Claude Code, etc.) to consume programmatically. Caches results for 24h to avoid rate limiting.

## Sources

| Source | Type | Data |
|--------|------|------|
| Artificial Analysis | agent-browser scrape | Intelligence index, speed, pricing |
| LM Arena (LMSYS) | Community JSON | ELO ratings from human preferences |
| SWE-bench | GitHub JSON | Code generation resolve rates |
| SWE-rebench | agent-browser scrape | Code generation resolve rates (rebench variant) |
| Aider | GitHub YAML | Polyglot coding benchmark pass rates |
| LiveBench | HuggingFace API | Multi-domain benchmark scores |
| Terminal-Bench | HuggingFace YAML | Terminal/CLI task completion |
| SEAL | agent-browser scrape | Scale AI multi-benchmark evaluations |

> **Note:** Sources marked "agent-browser scrape" require the [`agent-browser`](https://github.com/anthropics/agent-browser) CLI. All other sources work out of the box. LiveBench data depends on the upstream HuggingFace dataset which may lag behind other sources.

## Installation

```
cargo install pondus
```

## Usage

```bash
pondus rank                     # rank all models (default command)
pondus                          # same as `pondus rank`
pondus rank --top 10            # top 10 only
pondus check claude-opus-4.6    # check one model across all sources
pondus compare gpt-5.2 claude-opus-4.6  # head-to-head comparison
pondus sources                  # show source status
pondus refresh                  # clear cache and re-fetch
```

### Global Flags

| Flag | Description |
|------|-------------|
| `--format json|table|markdown` | Output format (default: json) |
| `--refresh` | Bypass cache for this run |

## Configuration

Config location: `~/.config/pondus/config.toml`

```toml
[cache]
ttl_hours = 24

[alias]
path = "models.toml"  # relative to config dir, or absolute path

[sources.artificial_analysis]
api_key = "your-key"  # optional, for AA source

[sources.agent_browser]
path = "agent-browser"  # path to agent-browser CLI
```

## Model Aliases

Different benchmarks use different naming conventions. `models.toml` maps canonical model names to source-specific variants:

```toml
[claude-opus-4_6]
canonical = "claude-opus-4.6"
aliases = [
  "Claude Opus 4.6",
  "claude-opus-4-6",
  "anthropic/claude-opus-4.6",
  "Opus 4.6",
]
```

When you run `pondus check opus-4.6`, pondus resolves the alias to the canonical name and matches across all sources. Prefix matching also works automatically — `gemini-2.5-pro-preview-06-05` matches `gemini-2.5-pro` since the suffix starts with `-`. PRs welcome to add new models.

## Output Format

Default JSON output:

```json
{
  "timestamp": "2026-02-27T10:30:00Z",
  "query": { "query_type": "rank" },
  "sources": [
    {
      "source": "arena",
      "status": "ok",
      "scores": [
        { "model": "gpt-5.2", "rank": 1, "metrics": { "elo": 1350 } }
      ]
    }
  ]
}
```

## Contributing

- **Add a model:** Add an entry to `models.toml` with canonical name and known aliases
- **Add a source:** Implement the `Source` trait in `src/sources/`

PRs welcome.

## License

MIT

## Sister Tools

Part of a family of AI-augmented CLI tools:

- [lustro](https://github.com/terry-li-hm/lustro) — AI news aggregator
- [consilium](https://github.com/terry-li-hm/consilium) — Multi-model deliberation
- [hexis](https://github.com/hexis-framework/hexis) — Meta-cognitive framework
