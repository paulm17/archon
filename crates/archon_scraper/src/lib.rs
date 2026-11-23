use anyhow::Result;
use archon_pipeline::{catalog, config, resolve_path, settings, sites};
use std::path::Path;
pub mod engine;
mod proxy;
mod scrape;
pub use engine::report::Report;
pub use scrape::Narrow;

pub fn preview(output_root: &Path, target: Option<&str>, filters: &Narrow) -> Result<String> {
    let (config, catalog) = settings()?;
    scrape::describe(output_root, &catalog, &config, target, filters)
}

pub async fn run(output_root: &Path, target: Option<&str>, filters: &Narrow) -> Result<Report> {
    let (config, catalog) = settings()?;
    let report = scrape::run(output_root, &config, &catalog, target, filters).await?;
    let store = engine::store::Store::new(output_root, &config.output);
    report.write(&store.report.join("scrape.json"))?;
    Ok(report)
}
