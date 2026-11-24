//! Where scraped and derived artefacts live, and how they are written.
//!
//! Layout, all under the configured roots:
//!   gospels/<language>/<translation>/<USFM>/chapter_<n>.html by default
//!   gospels/en/<book short name>/<translation>/ for BLB and CPDV
//!   manifest.json beside the source HTML
//!   json/<language>/<translation>/<USFM>/chapter_<n>.json

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::catalog::{Book, Resolved};
use crate::config::Output;
use crate::resolve_path;

pub struct Store {
    pub gospels: PathBuf,
    pub json: PathBuf,
    pub books: PathBuf,
    pub report: PathBuf,
}

impl Store {
    pub fn new(root: &Path, output: &Output) -> Self {
        Self {
            gospels: resolve_path(root, &output.gospels),
            json: resolve_path(root, &output.json),
            books: resolve_path(root, &output.books),
            report: resolve_path(root, &output.report),
        }
    }

    pub fn html_path(&self, translation: &Resolved, book: &Book, chapter: u32) -> PathBuf {
        translation.source_path(&self.gospels, book, chapter)
    }

    pub fn json_path(&self, translation: &Resolved, book: &Book, chapter: u32) -> PathBuf {
        translation.json_path(&self.json, book, chapter)
    }

    pub fn manifest_path(&self, translation: &Resolved, book: &Book) -> PathBuf {
        translation.manifest_path(&self.gospels, book)
    }

    /// Read a file if it exists.
    pub fn read(path: &Path) -> Option<String> {
        fs::read_to_string(path).ok()
    }

    /// Write via a temporary file and a rename, so a crash never leaves a
    /// half-written chapter behind.
    pub fn save_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
        let parent = path
            .parent()
            .with_context(|| format!("{} has no parent directory", path.display()))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
        let temporary = match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => path.with_extension(format!("{ext}.tmp")),
            None => path.with_extension("tmp"),
        };
        fs::write(&temporary, bytes)
            .with_context(|| format!("Failed to write {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("Failed to save {}", path.display()))?;
        Ok(())
    }

    pub fn write_manifest(&self, path: &Path, manifest: &Manifest) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(manifest).context("Failed to encode manifest")?;
        Self::save_atomic(path, &bytes)
    }
}

/// Provenance for every chapter saved under one book.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub language: String,
    pub translation: String,
    pub site: String,
    pub book: String,
    #[serde(default)]
    pub chapters: BTreeMap<u32, ChapterEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterEntry {
    pub url: String,
    pub scraped_at: u64,
    pub verses: usize,
    pub bytes: usize,
}

impl ChapterEntry {
    pub fn new(url: impl Into<String>, verses: usize, bytes: usize) -> Self {
        Self {
            url: url.into(),
            scraped_at: now(),
            verses,
            bytes,
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn store() -> Store {
        Store::new(
            Path::new("/project"),
            &Output {
                gospels: "gospels".into(),
                json: "json".into(),
                books: "books".into(),
                report: "reports".into(),
            },
        )
    }

    fn catalog() -> Catalog {
        Catalog::from_str(include_str!("../../catalog.toml")).unwrap()
    }

    #[test]
    fn paths_follow_the_agreed_layout() {
        let catalog = catalog();
        let store = store();
        let mark = catalog.book("MRK").unwrap();
        let es = catalog.resolve("es", "NVI").unwrap();
        let pt = catalog.resolve("pt", "NVI").unwrap();

        assert_eq!(
            store.html_path(&es, mark, 1),
            Path::new("/project/gospels/es/NVI/MRK/chapter_1.html")
        );
        assert_eq!(
            store.html_path(&pt, mark, 1),
            Path::new("/project/gospels/pt/NVI/MRK/chapter_1.html")
        );
        assert_eq!(
            store.json_path(&es, mark, 1),
            Path::new("/project/json/es/NVI/MRK/chapter_1.json")
        );
        assert_eq!(
            store.manifest_path(&es, mark),
            Path::new("/project/gospels/es/NVI/MRK/manifest.json")
        );
    }

    #[test]
    fn whole_book_sites_keep_one_document_per_book() {
        let catalog = catalog();
        let store = store();
        let luke = catalog.book("LUK").unwrap();
        let oeb = catalog.resolve("en", "OEB").unwrap();

        assert_eq!(
            store.html_path(&oeb, luke, 1),
            Path::new("/project/gospels/en/OEB/LUK/book.html")
        );
        assert_eq!(
            store.html_path(&oeb, luke, 7),
            Path::new("/project/gospels/en/OEB/LUK/book.html"),
            "every chapter reads the same document"
        );
        // Derived output is still per chapter.
        assert_eq!(
            store.json_path(&oeb, luke, 7),
            Path::new("/project/json/en/OEB/LUK/chapter_7.json")
        );
    }

    #[test]
    fn blb_and_cpdv_manifests_follow_the_book_first_html_layout() {
        let catalog = catalog();
        let store = store();
        let genesis = catalog.book("GEN").unwrap();
        for (code, filename) in [("BLB", "chapter_1.html"), ("CPDV", "book.html")] {
            let translation = catalog.resolve("en", code).unwrap();
            let directory = Path::new("/project/gospels/en/Gen").join(code);
            assert_eq!(store.html_path(&translation, genesis, 1), directory.join(filename));
            assert_eq!(
                store.manifest_path(&translation, genesis),
                directory.join("manifest.json")
            );
            assert_eq!(
                store.json_path(&translation, genesis, 1),
                Path::new("/project/json/en").join(code).join("GEN/chapter_1.json")
            );
        }
    }

    #[test]
    fn atomic_write_round_trips() {
        let dir = std::env::temp_dir().join(format!("scraper_unified_store_{}", now()));
        let path = dir.join("es/NVI/MRK/chapter_1.html");
        Store::save_atomic(&path, b"<html></html>").unwrap();
        assert_eq!(Store::read(&path).as_deref(), Some("<html></html>"));
        assert!(
            !path.with_extension("html.tmp").exists(),
            "temporary file should have been renamed away"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn manifest_serialises_provenance() {
        let dir = std::env::temp_dir().join(format!("scraper_unified_manifest_{}", now()));
        let store = store();
        let catalog = catalog();
        let mark = catalog.book("MRK").unwrap();
        let es = catalog.resolve("es", "NVI").unwrap();

        let mut manifest = Manifest {
            language: es.language.clone(),
            translation: es.code.clone(),
            site: es.site.clone(),
            book: mark.usfm.clone(),
            chapters: BTreeMap::new(),
        };
        manifest.chapters.insert(
            1,
            ChapterEntry::new("https://example.test/MRK.1", 45, 1234),
        );

        let path = dir.join("manifest.json");
        store.write_manifest(&path, &manifest).unwrap();

        let text = Store::read(&path).unwrap();
        let parsed: Manifest = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed.translation, "NVI");
        assert_eq!(parsed.chapters[&1].verses, 45);
        assert_eq!(parsed.chapters[&1].url, "https://example.test/MRK.1");
        fs::remove_dir_all(&dir).ok();
    }
}
