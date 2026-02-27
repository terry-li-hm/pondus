pub mod aa;
pub mod aider;
pub mod arena;
pub mod livebench;
pub mod mock;
pub mod swebench;
pub mod tbench;

use crate::cache::Cache;
use crate::config::Config;
use crate::models::SourceResult;
use anyhow::Result;

pub trait Source {
    fn name(&self) -> &str;
    fn fetch(&self, config: &Config, cache: &Cache) -> Result<SourceResult>;
}

/// Returns all registered sources.
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

/// Returns all sources including the mock (for testing/development).
pub fn all_sources_with_mock() -> Vec<Box<dyn Source>> {
    vec![Box::new(mock::MockSource)]
}
