pub mod mock;

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
        // TODO: add real sources as they're implemented
        // Box::new(aa::ArtificialAnalysis),
        // Box::new(arena::Arena),
        // Box::new(swebench::SweBench),
        // Box::new(aider::Aider),
        // Box::new(livebench::LiveBench),
        // Box::new(tbench::TerminalBench),
        // Box::new(seal::Seal),
        // Box::new(swebench_r::SweRebench),
    ]
}

/// Returns all sources including the mock (for testing/development).
pub fn all_sources_with_mock() -> Vec<Box<dyn Source>> {
    vec![Box::new(mock::MockSource)]
}
