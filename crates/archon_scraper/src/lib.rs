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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genesis_preview_uses_book_first_paths_and_one_fetch_per_translation() {
        let filters = Narrow {
            translations: vec!["BLB".into(), "CPDV".into()],
            books: vec!["GEN".into()],
            chapters: vec![1],
            ..Default::default()
        };
        let root = std::env::temp_dir();
        let plan = preview(&root, None, &filters).unwrap();
        assert!(
            plan.contains("https://biblehub.com/blb/genesis/1.htm"),
            "{plan}"
        );
        assert!(
            plan.contains("https://sacredbible.org/catholic/OT-01_Genesis.htm"),
            "{plan}"
        );
        assert!(plan.contains("gospels/en/Gen/BLB/chapter_1.html"), "{plan}");
        assert!(plan.contains("gospels/en/Gen/CPDV/book.html"), "{plan}");
        assert_eq!(
            plan.matches("1 books, 1 chapters, 1 fetch(es)").count(),
            2,
            "{plan}"
        );
    }
}
