//! Site adapters.
//!
//! A site module supplies only what is genuinely site-specific: the readiness
//! signal is in the catalog, the URL is in the catalog, the path is in the
//! catalog — a module adds validation and extraction. That is why adding a
//! site is a small file, not a pipeline.

use anyhow::Result;

use crate::catalog::{Book, Resolved};
use crate::model::ChapterBody;

pub mod bible_com;
pub mod bible_gateway;
pub mod bible_hub;
pub mod ibep;
pub mod obd;
pub mod oeb;
pub mod study_bible;

/// One chapter to load, with everything a site needs to identify it.
pub struct ChapterRef<'a> {
    pub translation: &'a Resolved,
    pub book: &'a Book,
    pub chapter: u32,
}

impl ChapterRef<'_> {
    pub fn usfm(&self) -> &str {
        &self.book.usfm
    }

    /// `MRK.1` — the USFM passage id most sites use.
    pub fn passage(&self) -> String {
        format!("{}.{}", self.book.usfm, self.chapter)
    }
}

pub trait Site: Send + Sync {
    /// Reject a page that is not the expected book, chapter and translation.
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()>;

    /// How many verses the page actually contains.
    fn verse_count(&self, html: &str, chapter: &ChapterRef<'_>) -> usize;

    /// Extract the chapter.
    fn parse(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterBody>;
}

/// Look up a site module by the `module` field in the catalog.
pub fn implementation(module: &str) -> Option<&'static dyn Site> {
    match module {
        "bible_com" => Some(&bible_com::BibleCom),
        "ibep" => Some(&ibep::Ibep),
        "bible_gateway" => Some(&bible_gateway::BibleGateway),
        "study_bible" => Some(&study_bible::StudyBible),
        "bible_hub" => Some(&bible_hub::BibleHub),
        "obd" => Some(&obd::Obd),
        "oeb" => Some(&oeb::Oeb),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_modules_resolve() {
        assert!(implementation("bible_com").is_some());
        assert!(implementation("ibep").is_some());
    }

    #[test]
    fn every_catalog_module_is_implemented() {
        let catalog = crate::catalog::Catalog::from_str(include_str!("../../catalog.toml")).unwrap();
        for (name, site) in &catalog.sites {
            assert!(
                implementation(&site.module).is_some(),
                "site {name} needs module {}",
                site.module
            );
        }
    }

    #[test]
    fn unknown_modules_do_not_resolve() {
        assert!(implementation("nope").is_none());
    }
}
