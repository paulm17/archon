mod convert;
use anyhow::Result;
use std::path::Path;

use std::path::PathBuf;

/// What a batch conversion did.
#[derive(Debug, Default)]
pub struct Outcome {
    pub converted: usize,
    pub skipped: usize,
    pub failed: Vec<(PathBuf, String)>,
}

impl Outcome {
    pub fn summary(&self, noun: &str) -> String {
        format!(
            "{noun}: {} converted, {} skipped, {} failed",
            self.converted,
            self.skipped,
            self.failed.len()
        )
    }

    pub fn print_failures(&self) {
        for (path, error) in &self.failed {
            eprintln!("{}: {error}", path.display());
        }
    }
}

pub fn run(output_root: &Path, translations: &[&str]) -> Result<Outcome> {
    let (config, catalog) = archon_pipeline::settings()?;
    convert::run_selected(output_root, &config, &catalog, translations)
}
