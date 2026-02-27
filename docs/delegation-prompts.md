# Pondus Delegation Prompts

Ready-to-run prompts for delegating source implementations. Each prompt is self-contained with all context needed.

Run with: `/delegate opencode` or `/delegate gemini` — paste the relevant prompt.

After all sources are done:
1. Add `pub mod <name>;` to `src/sources/mod.rs`
2. Add `Box::new(<name>::<Struct>)` to `all_sources()`
3. `cargo build` to verify

---

## Shared Context (included in each prompt below)

```
PROJECT: ~/code/pondus (Rust CLI, edition 2024)
BRANCH: feat/core-implementation
DEPS: anyhow 1, chrono 0.4 (serde), reqwest 0.12 (blocking, json), serde 1 (derive), serde_json 1, serde_yaml 0.9
```

---

## Phase 2: Artificial Analysis (aa.rs)

```
TASK: Create src/sources/aa.rs for the pondus project at ~/code/pondus/

This implements a Source trait for fetching AI model benchmark data from Artificial Analysis's REST API.

API ENDPOINT: GET https://artificialanalysis.ai/api/v2/data/llms/models
AUTH: x-api-key header. Read from config via config.aa_api_key(). If no key configured, return SourceStatus::Unavailable.
RESPONSE: JSON array of model objects. Key fields to extract as metrics:
  - intelligence_index (float) — composite benchmark score
  - input_cost_per_1m_tokens (float) — price
  - output_cost_per_1m_tokens (float) — price
  - tokens_per_second (float) — speed

CACHE: Check cache.get("artificial-analysis") first. If cache hit, return with SourceStatus::Cached.
After successful fetch, call cache.set("artificial-analysis", &raw_json).

SOURCE TRAIT to implement:
```rust
pub trait Source {
    fn name(&self) -> &str;  // return "artificial-analysis"
    fn fetch(&self, config: &Config, cache: &Cache) -> Result<SourceResult>;
}
```

TYPES (from src/models.rs):
```rust
pub struct SourceResult {
    pub source: String,
    pub fetched_at: Option<DateTime<Utc>>,
    pub status: SourceStatus,
    pub scores: Vec<ModelScore>,
}
pub enum SourceStatus { Ok, Cached, Unavailable, Error(String) }
pub struct ModelScore {
    pub model: String,                              // lowercase canonical
    pub source_model_name: String,                  // original from API
    pub metrics: HashMap<String, MetricValue>,
    pub rank: Option<u32>,                          // None if source doesn't rank
}
pub enum MetricValue { Float(f64), Int(i64), Text(String) }
```

PATTERN TO FOLLOW (from src/sources/mock.rs — same imports, same structure):
- Use reqwest::blocking::Client for HTTP
- Use anyhow::{Context, Result} for errors
- Store model name lowercase in `model` field, original in `source_model_name`
- Wrap each metric as MetricValue::Float or MetricValue::Int
- Sort by intelligence_index descending, assign rank based on position

IMPORTS:
```rust
use crate::cache::Cache;
use crate::config::Config;
use crate::models::{MetricValue, ModelScore, SourceResult, SourceStatus};
use crate::sources::Source;
use anyhow::{Context, Result};
use chrono::Utc;
use std::collections::HashMap;
```

Create the file at: ~/code/pondus/src/sources/aa.rs
Do NOT modify any other files — I'll wire it up in mod.rs myself.
```

---

## Phase 3a: SWE-bench (swebench.rs)

```
TASK: Create src/sources/swebench.rs for the pondus project at ~/code/pondus/

DATA SOURCE: GitHub raw JSON file
URL: https://raw.githubusercontent.com/SWE-bench/swe-bench.github.io/refs/heads/main/data/leaderboards.json
NO AUTH REQUIRED.

RESPONSE FORMAT: JSON object with leaderboard entries. Each entry has model name and resolve rate (percentage of tasks resolved). Extract:
  - resolved_rate (float) — percentage
  - resolved_count (int) — if available

CACHE KEY: "swebench"
Check cache.get("swebench") first. If hit, return SourceStatus::Cached.
After fetch, cache.set("swebench", &raw_json).

SOURCE TRAIT: name() returns "swebench"

TYPES AND IMPORTS: Same as the AA prompt above.

PATTERN: Use reqwest::blocking::get(url) — no auth needed. Parse JSON, create ModelScore per entry. Sort by resolved_rate descending, assign rank. Model name goes lowercase in `model`, original in `source_model_name`.

NOTE: The actual JSON structure may need exploration. If the response format isn't immediately clear, deserialize as serde_json::Value first, inspect the structure, then extract fields. Use anyhow context for error messages.

Create: ~/code/pondus/src/sources/swebench.rs
Do NOT modify other files.
```

---

## Phase 3b: LM Arena (arena.rs)

```
TASK: Create src/sources/arena.rs for the pondus project at ~/code/pondus/

DATA SOURCE: Community-maintained JSON from GitHub
URL: https://raw.githubusercontent.com/nakasyou/lmarena-history/main/output/scores.json
NO AUTH REQUIRED. NOTE: This is a community scraper, not official — format may change.

RESPONSE FORMAT: JSON with model ELO scores. Key fields:
  - model name
  - elo_score or rating (float)

CACHE KEY: "arena"

SOURCE TRAIT: name() returns "arena"

TYPES AND IMPORTS: Same as AA prompt.

PATTERN: reqwest::blocking::get(url). Deserialize as serde_json::Value first since the format is community-maintained and may vary. Extract model names and ELO scores. Sort by ELO descending, assign rank.

IMPORTANT: This source is fragile. Wrap the entire parse in a match/if-let so parsing failures return SourceStatus::Error with a descriptive message, not a panic.

Create: ~/code/pondus/src/sources/arena.rs
Do NOT modify other files.
```

---

## Phase 3c: Aider (aider.rs)

```
TASK: Create src/sources/aider.rs for the pondus project at ~/code/pondus/

DATA SOURCE: YAML files from Aider's GitHub repo
PRIMARY URL: https://raw.githubusercontent.com/Aider-AI/aider/main/aider/website/_data/polyglot_leaderboard.yml
SECONDARY (optional, can add later): edit_leaderboard.yml, refactor_leaderboard.yml
NO AUTH REQUIRED.

RESPONSE FORMAT: YAML array of entries. Each entry has model name and benchmark scores. Key fields to extract:
  - pass_rate_1 or percent_cases_well_formed (float) — primary score
  - cost (float) — if available

DEPENDENCY: serde_yaml 0.9 (already in Cargo.toml)

CACHE KEY: "aider"

SOURCE TRAIT: name() returns "aider"

TYPES AND IMPORTS: Same as AA prompt, plus:
```rust
// For YAML parsing — serde_yaml is already a dependency
```

PATTERN: reqwest::blocking::get(url). Deserialize YAML with serde_yaml::from_str. Each entry becomes a ModelScore. Sort by primary score descending, assign rank.

Create: ~/code/pondus/src/sources/aider.rs
Do NOT modify other files.
```

---

## Phase 3d: LiveBench (livebench.rs)

```
TASK: Create src/sources/livebench.rs for the pondus project at ~/code/pondus/

DATA SOURCE: LiveBench results from GitHub
URL: https://raw.githubusercontent.com/LiveBench/LiveBench/main/docs/blog/leaderboard.json
(If this URL doesn't work, try the HuggingFace API: https://huggingface.co/api/datasets/livebench/results)
NO AUTH REQUIRED.

RESPONSE FORMAT: JSON with model scores across categories (math, coding, reasoning, language, data_analysis, instruction_following). Extract:
  - global_average or overall score (float)
  - Per-category scores as separate metrics

CACHE KEY: "livebench"

SOURCE TRAIT: name() returns "livebench"

TYPES AND IMPORTS: Same as AA prompt.

PATTERN: reqwest::blocking::get(url). The exact JSON structure may need exploration — deserialize as serde_json::Value first, then extract. Create ModelScore per model with category scores as metrics. Sort by overall/global score descending.

NOTE: LiveBench updates monthly with new questions. The data format may include version info — include it as a metric if present.

Create: ~/code/pondus/src/sources/livebench.rs
Do NOT modify other files.
```

---

## Phase 3e: Terminal-Bench (tbench.rs)

```
TASK: Create src/sources/tbench.rs for the pondus project at ~/code/pondus/

DATA SOURCE: Terminal-Bench results from HuggingFace
URL: https://huggingface.co/api/datasets/sabhay/terminal-bench-2-leaderboard/parquet/default/train/0.parquet
ALTERNATIVE: Try the dataset API first: https://huggingface.co/api/datasets/sabhay/terminal-bench-2-leaderboard
If parquet is too complex, fall back to scraping https://www.tbench.ai/leaderboard via the JSON API if one exists.
NO AUTH REQUIRED.

RESPONSE FORMAT: Dataset entries with model/agent names and task completion scores. Extract:
  - score or pass_rate (float)
  - tasks_completed (int) — if available

CACHE KEY: "terminal-bench"

SOURCE TRAIT: name() returns "terminal-bench"

TYPES AND IMPORTS: Same as AA prompt.

PATTERN: This source may be the trickiest due to HuggingFace dataset formats. Try the simplest approach first:
1. Try fetching a JSON endpoint from HF API
2. If that fails, try fetching the leaderboard page JSON from tbench.ai
3. If all else fails, return SourceStatus::Unavailable with a message

Sort by score descending, assign rank.

Create: ~/code/pondus/src/sources/tbench.rs
Do NOT modify other files.
```

---

## Wiring Up (after all delegates complete)

After collecting all source files, update `src/sources/mod.rs`:

```rust
pub mod mock;
pub mod aa;
pub mod arena;
pub mod swebench;
pub mod aider;
pub mod livebench;
pub mod tbench;

// In all_sources():
pub fn all_sources() -> Vec<Box<dyn Source>> {
    vec![
        Box::new(aa::ArtificialAnalysis),
        Box::new(arena::Arena),
        Box::new(swebench::SweBench),
        Box::new(aider::Aider),
        Box::new(livebench::LiveBench),
        Box::new(tbench::TerminalBench),
    ]
}
```

Then `cargo build` to catch any issues. Fix compile errors, then test with `cargo run -- rank 2>/dev/null | python3 -c "import json,sys; d=json.load(sys.stdin); [print(f'{s[\"source\"]}: {s[\"status\"]} ({len(s[\"scores\"])} models)') for s in d['sources']]"`
