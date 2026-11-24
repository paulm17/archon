//! The domain registry: what exists, and how a (language, translation, book,
//! chapter) resolves to a URL and an output path.
//!
//! Nothing here performs I/O beyond reading the catalog file.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub books: Vec<Book>,
    #[serde(default)]
    pub languages: BTreeMap<String, Language>,
    pub sites: BTreeMap<String, SiteSpec>,
    #[serde(default)]
    pub translations: BTreeMap<String, BTreeMap<String, Vec<TranslationSpec>>>,
    #[serde(default)]
    pub book_aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Book {
    pub usfm: String,
    pub name: String,
    #[serde(default)]
    pub short: String,
    pub chapters: u32,
    pub testament: String,
    /// Overrides the derived slug for sites that spell a book unusually.
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Language {
    pub name: String,
    #[serde(default)]
    pub native: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SiteSpec {
    pub name: String,
    /// Name of the code module that parses this site.
    pub module: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub ready: Option<String>,
    #[serde(default = "default_book_slug")]
    pub book_slug: String,
    /// The book ids this site uses when `book_slug = "map"`. Anything absent
    /// keeps its USFM id, so only a site's exceptions need listing.
    #[serde(default)]
    pub book_codes: BTreeMap<String, String>,
    /// Books this site does not publish at all, for any of its translations.
    /// Asking for them only produces permanent 404s. A translation's own
    /// `exclude` adds to this list.
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub unit: Option<Unit>,
    #[serde(default)]
    pub interlinear: bool,
}

fn default_book_slug() -> String {
    "usfm".into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    #[default]
    Chapter,
    Book,
}

/// A translation's book scope: a named group, or an explicit list of USFM ids.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum BookFilter {
    Group(String),
    List(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranslationSpec {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub bible_id: Option<u32>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub books: Option<BookFilter>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub chapters: BTreeMap<String, u32>,
    #[serde(default)]
    pub interlinear: Option<bool>,
    #[serde(default)]
    pub unit: Option<Unit>,
}

/// A translation with its site defaults already folded in.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub language: String,
    pub site: String,
    pub module: String,
    pub code: String,
    pub name: String,
    pub version: String,
    pub bible_id: Option<u32>,
    pub url_template: Option<String>,
    pub ready: Option<String>,
    pub book_slug: String,
    pub book_codes: BTreeMap<String, String>,
    pub unit: Unit,
    pub interlinear: bool,
    pub books: Option<BookFilter>,
    pub exclude: Vec<String>,
    pub chapter_overrides: BTreeMap<String, u32>,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        Self::from_str(&text).with_context(|| format!("Invalid {}", path.display()))
    }

    pub fn from_str(text: &str) -> Result<Self> {
        let catalog = Self::parse(text)?;
        let problems = catalog.validate();
        if !problems.is_empty() {
            bail!("Catalog is invalid:\n  - {}", problems.join("\n  - "));
        }
        Ok(catalog)
    }

    /// Parse without validating, so problems can be reported all at once.
    pub fn parse(text: &str) -> Result<Self> {
        toml::from_str(text).context("Invalid catalog TOML")
    }

    pub fn book(&self, usfm: &str) -> Option<&Book> {
        self.books.iter().find(|b| b.usfm == usfm)
    }

    /// Fold a site's defaults into a translation row.
    pub fn resolve(&self, language: &str, code: &str) -> Option<Resolved> {
        for (site_name, rows) in self.translations.get(language)? {
            let Some(row) = rows.iter().find(|r| r.code == code) else {
                continue;
            };
            let site = self.sites.get(site_name)?;
            return Some(Resolved {
                language: language.to_owned(),
                site: site_name.clone(),
                module: site.module.clone(),
                code: row.code.clone(),
                name: row.name.clone(),
                version: row.version.clone().unwrap_or_else(|| row.code.clone()),
                bible_id: row.bible_id,
                url_template: row.url.clone().or_else(|| site.url.clone()),
                ready: site.ready.clone(),
                book_slug: site.book_slug.clone(),
                book_codes: site.book_codes.clone(),
                unit: row.unit.or(site.unit).unwrap_or_default(),
                interlinear: row.interlinear.or(Some(site.interlinear)).unwrap_or(false),
                books: row.books.clone(),
                exclude: site
                    .exclude
                    .iter()
                    .chain(row.exclude.iter())
                    .cloned()
                    .collect(),
                chapter_overrides: row.chapters.clone(),
            });
        }
        None
    }

    /// Every (language, code) pair, in catalog order.
    pub fn translation_ids(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (language, by_site) in &self.translations {
            for rows in by_site.values() {
                for row in rows {
                    out.push((language.clone(), row.code.clone()));
                }
            }
        }
        out
    }

    /// Every translation serving `site`.
    pub fn translations_for_site(&self, site: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (language, by_site) in &self.translations {
            for row in by_site.get(site).map(Vec::as_slice).unwrap_or_default() {
                out.push((language.clone(), row.code.clone()));
            }
        }
        out
    }

    /// Structural checks that do not need the network.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();

        let mut seen_usfm = BTreeSet::new();
        for book in &self.books {
            if !seen_usfm.insert(book.usfm.as_str()) {
                problems.push(format!("duplicate book usfm {}", book.usfm));
            }
            if book.chapters == 0 {
                problems.push(format!("book {} has zero chapters", book.usfm));
            }
            if !matches!(book.testament.as_str(), "ot" | "nt" | "dc") {
                problems.push(format!(
                    "book {} has unknown testament {:?}",
                    book.usfm, book.testament
                ));
            }
        }

        for (site_name, site) in &self.sites {
            if site.module.trim().is_empty() {
                problems.push(format!("site {site_name} has no module"));
            }
            if !matches!(
                site.book_slug.as_str(),
                "usfm" | "short" | "lower" | "toc" | "map"
            ) {
                problems.push(format!(
                    "site {site_name} has unknown book_slug {:?}",
                    site.book_slug
                ));
            }
            for usfm in &site.exclude {
                if self.book(usfm).is_none() {
                    problems.push(format!("site {site_name} excludes unknown book {usfm}"));
                }
            }
            if let Some(url) = &site.url {
                let whole_work = site.unit == Some(Unit::Book) && !url.contains("{chapter}");
                check_placeholders(
                    url,
                    &mut problems,
                    site_name,
                    site.unit != Some(Unit::Book),
                    !whole_work,
                );
            }
        }

        for (language, by_site) in &self.translations {
            if !self.languages.is_empty() && !self.languages.contains_key(language) {
                problems.push(format!("language {language} is used but not declared"));
            }
            let mut codes = BTreeSet::new();
            for (site_name, rows) in by_site {
                if !self.sites.contains_key(site_name) {
                    problems.push(format!(
                        "language {language} references unknown site {site_name}"
                    ));
                }
                for row in rows {
                    if !codes.insert(row.code.as_str()) {
                        problems.push(format!(
                            "duplicate translation code {} in language {language}",
                            row.code
                        ));
                    }
                    if row.url.is_none()
                        && self
                            .sites
                            .get(site_name)
                            .and_then(|s| s.url.as_ref())
                            .is_none()
                    {
                        problems.push(format!(
                            "{language}/{} has no url template (neither site nor translation)",
                            row.code
                        ));
                    }
                    if let Some(url) = &row.url {
                        let unit = row
                            .unit
                            .or_else(|| self.sites.get(site_name).and_then(|s| s.unit));
                        let whole_work =
                            unit == Some(Unit::Book) && !url.contains("{chapter}");
                        check_placeholders(
                            url,
                            &mut problems,
                            &format!("{language}/{}", row.code),
                            unit != Some(Unit::Book),
                            !whole_work,
                        );
                    }
                    match &row.books {
                        Some(BookFilter::Group(group)) => {
                            if !matches!(group.as_str(), "ot" | "nt" | "dc" | "all") {
                                problems.push(format!(
                                    "{language}/{} has unknown book group {group:?}",
                                    row.code
                                ));
                            }
                        }
                        Some(BookFilter::List(list)) => {
                            for usfm in list {
                                if self.book(usfm).is_none() {
                                    problems.push(format!(
                                        "{language}/{} references unknown book {usfm}",
                                        row.code
                                    ));
                                }
                            }
                        }
                        None => {}
                    }
                    for usfm in &row.exclude {
                        if self.book(usfm).is_none() {
                            problems.push(format!(
                                "{language}/{} excludes unknown book {usfm}",
                                row.code
                            ));
                        }
                    }
                    for usfm in row.chapters.keys() {
                        if self.book(usfm).is_none() {
                            problems.push(format!(
                                "{language}/{} overrides chapters for unknown book {usfm}",
                                row.code
                            ));
                        }
                    }
                }
            }
        }

        problems
    }
}

fn check_placeholders(
    url: &str,
    problems: &mut Vec<String>,
    owner: &str,
    needs_chapter: bool,
    needs_book: bool,
) {
    // A site that publishes the whole work as one document addresses no book
    // in its URL at all.
    if needs_book && !url.contains("{book}") {
        problems.push(format!("{owner} url template is missing {{book}}: {url}"));
    }
    // Whole-book sites (unit = "book") address a book, not a chapter.
    if needs_chapter && !url.contains("{chapter}") {
        problems.push(format!("{owner} url template is missing {{chapter}}: {url}"));
    }
    let mut rest = url.to_string();
    for known in ["{language}", "{book}", "{chapter}", "{version}", "{bible_id}"] {
        rest = rest.replace(known, "");
    }
    if rest.contains('{') || rest.contains('}') {
        problems.push(format!("{owner} url template has an unknown placeholder: {url}"));
    }
}

impl Resolved {
    /// The language and translation segment of translation-first paths.
    pub fn relative_dir(&self) -> PathBuf {
        Path::new(&self.language).join(&self.code)
    }

    pub fn chapter_path(&self, root: &Path, book: &Book, chapter: u32) -> PathBuf {
        self.source_dir(root, book)
            .join(format!("chapter_{chapter}.html"))
    }

    fn source_dir(&self, root: &Path, book: &Book) -> PathBuf {
        if self.language == "en" && matches!(self.code.as_str(), "BLB" | "CPDV") {
            root.join(&self.language).join(&book.short).join(&self.code)
        } else {
            root.join(self.relative_dir()).join(&book.usfm)
        }
    }

    /// Where the scraped HTML for a chapter lives. Whole-book sites deliver
    /// every chapter in one document, so they keep a single `book.html`.
    pub fn source_path(&self, root: &Path, book: &Book, chapter: u32) -> PathBuf {
        match self.unit {
            Unit::Book => self.source_dir(root, book).join("book.html"),
            Unit::Chapter => self.chapter_path(root, book, chapter),
        }
    }

    pub fn json_path(&self, root: &Path, book: &Book, chapter: u32) -> PathBuf {
        root.join(self.relative_dir())
            .join(&book.usfm)
            .join(format!("chapter_{chapter}.json"))
    }

    pub fn manifest_path(&self, root: &Path, book: &Book) -> PathBuf {
        self.source_dir(root, book).join("manifest.json")
    }

    /// Chapter count for a book, honouring a per-translation override.
    pub fn chapter_count(&self, book: &Book) -> u32 {
        self.chapter_overrides
            .get(&book.usfm)
            .copied()
            .unwrap_or(book.chapters)
    }

    /// The books this translation actually contains.
    pub fn books(&self, catalog: &Catalog) -> Result<Vec<Book>> {
        let mut out: Vec<Book> = match &self.books {
            None => catalog.books.clone(),
            Some(BookFilter::Group(group)) => catalog
                .books
                .iter()
                .filter(|b| match group.as_str() {
                    "all" => true,
                    "ot" => b.testament == "ot",
                    "nt" => b.testament == "nt",
                    "dc" => b.testament == "dc",
                    _ => false,
                })
                .cloned()
                .collect(),
            Some(BookFilter::List(list)) => {
                let mut books = Vec::new();
                for usfm in list {
                    let book = catalog
                        .book(usfm)
                        .with_context(|| format!("{}/{} references unknown book {usfm}", self.language, self.code))?;
                    books.push(book.clone());
                }
                books
            }
        };
        out.retain(|b| !self.exclude.iter().any(|e| e == &b.usfm));
        out.retain(|b| self.chapter_count(b) > 0);
        Ok(out)
    }

    /// The chapter URL for a book and chapter.
    pub fn url(&self, book: &Book, chapter: u32) -> Result<String> {
        let template = self
            .url_template
            .as_deref()
            .with_context(|| format!("{}/{} has no url template", self.language, self.code))?;
        let mut url = template.to_string();
        url = url.replace("{language}", &self.language);
        // A whole-work document has no book in its address, so resolving a
        // book slug there would fail for no reason.
        if url.contains("{book}") {
            let slug = self.book_slug(book)?;
            url = url.replace("{book}", &slug);
        }
        url = url.replace("{chapter}", &chapter.to_string());
        url = url.replace("{version}", &self.version);
        url = url.replace(
            "{bible_id}",
            &self.bible_id.map(|id| id.to_string()).unwrap_or_default(),
        );
        if url.contains('{') || url.contains('}') {
            bail!("unresolved placeholder in {url}");
        }
        Ok(url)
    }

    /// How this site spells a book in a URL.
    pub fn book_slug(&self, book: &Book) -> Result<String> {
        match self.book_slug.as_str() {
            "usfm" => Ok(book.usfm.clone()),
            "short" => {
                if book.short.is_empty() {
                    bail!("book {} has no short code", book.usfm);
                }
                Ok(book.short.clone())
            }
            "lower" => Ok(book
                .slug
                .clone()
                .unwrap_or_else(|| slugify(&book.name))),
            "map" => Ok(self
                .book_codes
                .get(&book.usfm)
                .cloned()
                .unwrap_or_else(|| book.usfm.clone())),
            "toc" => bail!(
                "site {} resolves books from its contents page, which is not implemented yet",
                self.site
            ),
            other => bail!("unknown book_slug {other:?}"),
        }
    }
}

fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_underscore = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::from_str(include_str!("../catalog.toml")).expect("embedded catalog")
    }

    #[test]
    fn embedded_catalog_is_valid() {
        let catalog = catalog();
        assert_eq!(catalog.validate(), Vec::<String>::new());
        assert_eq!(catalog.books.len(), 85);
        assert_eq!(catalog.sites.len(), 9);
        assert_eq!(catalog.translation_ids().len(), 91);
    }

    #[test]
    fn resolves_bible_com_spanish() {
        let catalog = catalog();
        let mark = catalog.book("MRK").unwrap();
        let nvi = catalog.resolve("es", "NVI").unwrap();
        assert_eq!(nvi.site, "bible_com");
        assert_eq!(nvi.module, "bible_com");
        assert_eq!(
            nvi.url(mark, 1).unwrap(),
            "https://www.bible.com/es/bible/128/MRK.1.NVI"
        );
        assert_eq!(
            nvi.chapter_path(Path::new("gospels"), mark, 1),
            Path::new("gospels/es/NVI/MRK/chapter_1.html")
        );
    }

    #[test]
    fn same_code_in_two_languages_keeps_them_apart() {
        let catalog = catalog();
        let mark = catalog.book("MRK").unwrap();
        let es = catalog.resolve("es", "NVI").unwrap();
        let pt = catalog.resolve("pt", "NVI").unwrap();
        assert_ne!(es.url(mark, 1).unwrap(), pt.url(mark, 1).unwrap());
        assert_ne!(
            es.chapter_path(Path::new("gospels"), mark, 1),
            pt.chapter_path(Path::new("gospels"), mark, 1)
        );
    }

    #[test]
    fn tagalog_uses_its_own_url_shape() {
        let catalog = catalog();
        let mark = catalog.book("MRK").unwrap();
        let asd = catalog.resolve("tl", "ASD").unwrap();
        assert_eq!(
            asd.url(mark, 1).unwrap(),
            "https://www.bible.com/bible/1264/MRK.1.ASD"
        );
    }

    #[test]
    fn ibep_hosts_share_a_module() {
        let catalog = catalog();
        let mark = catalog.book("MRK").unwrap();
        let korean = catalog.resolve("kr", "NKT").unwrap();
        let german = catalog.resolve("de", "BB").unwrap();
        assert_eq!(korean.module, "ibep");
        assert_eq!(german.module, "ibep");
        assert_eq!(
            korean.url(mark, 1).unwrap(),
            "https://bible.bskorea.or.kr/bible/NKT/MRK.1"
        );
        assert_eq!(
            german.url(mark, 1).unwrap(),
            "https://www.die-bibel.de/bibel/BB/MRK.1"
        );
    }

    #[test]
    fn bible_gateway_defaults_version_to_code() {
        let catalog = catalog();
        let genesis = catalog.book("GEN").unwrap();
        let niv = catalog.resolve("en", "NIV").unwrap();
        assert_eq!(niv.version, "NIV");
        assert_eq!(
            niv.url(genesis, 1).unwrap(),
            "https://www.biblegateway.com/passage/?search=Gen+1&version=NIV"
        );
        assert_eq!(
            niv.chapter_path(Path::new("gospels"), genesis, 1),
            Path::new("gospels/en/NIV/GEN/chapter_1.html")
        );
    }

    #[test]
    fn book_slugs_follow_the_site() {
        let catalog = catalog();
        let song = catalog.book("SNG").unwrap();
        let genesis = catalog.book("GEN").unwrap();
        let hub = catalog.resolve("en", "BIBLEHUB").unwrap();
        assert_eq!(hub.book_slug(genesis).unwrap(), "genesis");
        assert_eq!(hub.book_slug(song).unwrap(), "songs");
        assert_eq!(
            hub.url(genesis, 1).unwrap(),
            "https://biblehub.com/interlinear/genesis/1.htm"
        );
        let niv = catalog.resolve("en", "NIV").unwrap();
        assert_eq!(niv.book_slug(genesis).unwrap(), "Gen");
    }

    #[test]
    fn mapped_book_codes_override_usfm() {
        let catalog = catalog();
        let samuel = catalog.book("1SA").unwrap();
        let genesis = catalog.book("GEN").unwrap();
        let fbv = catalog.resolve("en", "FBV").unwrap();
        assert_eq!(fbv.site, "obd");
        assert_eq!(fbv.book_slug(samuel).unwrap(), "SA1");
        // A book the map does not name keeps its USFM id.
        assert_eq!(fbv.book_slug(genesis).unwrap(), "GEN");
        assert_eq!(
            fbv.url(samuel, 2).unwrap(),
            "https://freely-given.org/OBD/FBV/byC/SA1_C2.htm"
        );
    }

    #[test]
    fn book_scope_and_excludes_are_applied() {
        let catalog = catalog();
        let oeb = catalog.resolve("en", "OEB").unwrap();
        assert_eq!(oeb.unit, Unit::Book);
        let books = oeb.books(&catalog).unwrap();
        assert_eq!(books.len(), 26, "NT minus Revelation");
        assert!(!books.iter().any(|b| b.usfm == "REV"));

        let wlc = catalog.resolve("he", "WLC").unwrap();
        assert!(wlc.books(&catalog).unwrap().iter().all(|b| b.testament == "ot"));

        let niv = catalog.resolve("en", "NIV").unwrap();
        assert_eq!(niv.books(&catalog).unwrap().len(), 85);
    }

    #[test]
    fn site_excludes_apply_to_every_translation_of_that_site() {
        // freely-given.org publishes the 66-book canon only; the deuterocanon
        // is a permanent 404 for every OBD version, not a transient failure.
        let catalog = catalog();
        let ids = catalog.translations_for_site("obd");
        assert_eq!(ids.len(), 7);
        for (language, code) in ids {
            let translation = catalog.resolve(&language, &code).unwrap();
            let books = translation.books(&catalog).unwrap();
            assert_eq!(books.len(), 66, "{language}/{code} should be the canon");
            assert!(
                books.iter().all(|book| book.testament != "dc"),
                "{language}/{code} must not ask for deuterocanon"
            );
        }

        // Nahum is in scope, and the site spells it NAH.
        let t4t = catalog.resolve("en", "T4T").unwrap();
        let nahum = catalog.book("NAM").unwrap();
        assert_eq!(
            t4t.url(nahum, 1).unwrap(),
            "https://freely-given.org/OBD/T4T/byC/NAH_C1.htm"
        );
    }

    #[test]
    fn chapter_overrides_win_over_the_canon() {
        let catalog = catalog();
        let genesis = catalog.book("GEN").unwrap();
        let niv = catalog.resolve("en", "NIV").unwrap();
        assert_eq!(niv.chapter_count(genesis), 50);
    }

    #[test]
    fn a_whole_work_document_needs_no_book_slug() {
        // OEB publishes the whole Bible as one file, so its URL has no {book}
        // placeholder and no contents-page lookup is needed.
        let catalog = catalog();
        let genesis = catalog.book("GEN").unwrap();
        let luke = catalog.book("LUK").unwrap();
        let oeb = catalog.resolve("en", "OEB").unwrap();
        let url = oeb.url(genesis, 1).expect("url");
        assert_eq!(url, oeb.url(luke, 3).unwrap());
        assert!(url.starts_with("https://openenglishbible.org/"));
    }

    #[test]
    fn slugify_handles_numbered_and_spaced_names() {
        assert_eq!(slugify("1 Corinthians"), "1_corinthians");
        assert_eq!(slugify("Song of Solomon"), "song_of_solomon");
        assert_eq!(slugify("Psalms"), "psalms");
    }
}
