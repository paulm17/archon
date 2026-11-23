//! Fetch chapters and save the HTML.
//!
//! This is the only workflow that touches the network. It is deliberately
//! sequential: one page, one chapter at a time, exactly like the scrapers it
//! replaces.

use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::catalog::{Book, Catalog, Resolved, Unit};
use crate::config::{Config, Target};
use crate::engine::browser::{self, Session};
use crate::engine::fetch::{self, FetchTiming};
use crate::engine::report::Report;
use crate::engine::retry::RetryPolicy;
use crate::engine::store::{ChapterEntry, Manifest, Store, now};
use crate::sites::{self, ChapterRef};

/// One translation's worth of work.
pub struct Plan {
    pub translation: Resolved,
    pub books: Vec<(Book, Vec<u32>)>,
}

/// `only` selects one target by name or by its zero-based position.
pub fn plan_targets_filtered(
    catalog: &Catalog,
    config: &Config,
    only: Option<&str>,
) -> Result<Vec<Plan>> {
    let mut plans = Vec::new();
    for (index, target) in config.targets.iter().enumerate() {
        if let Some(only) = only {
            let by_name = target.name == only;
            let by_index = only.parse::<usize>().map(|n| n == index).unwrap_or(false);
            if !by_name && !by_index {
                continue;
            }
        }
        for (language, code) in target_ids(catalog, target) {
            let translation = catalog
                .resolve(&language, &code)
                .with_context(|| format!("cannot resolve {language}/{code}"))?;
            let books = match &target.books {
                Some(list) => expand_books(catalog, &translation, list)?,
                None => translation.books(catalog)?,
            };
            let books = books
                .into_iter()
                .map(|book| {
                    let chapters: Vec<u32> = match &target.chapters {
                        Some(list) => list.clone(),
                        None => (1..=translation.chapter_count(&book)).collect(),
                    };
                    (book, chapters)
                })
                .collect();
            plans.push(Plan { translation, books });
        }
    }
    if plans.is_empty() {
        if let Some(only) = only {
            bail!("no target matches {only:?}");
        }
    }
    Ok(plans)
}

/// Optional `--book`/`--chapter`/`--lang`/`--translation` limits on top of a
/// target.
#[derive(Debug, Default, Clone)]
pub struct Narrow {
    pub books: Vec<String>,
    pub chapters: Vec<u32>,
    pub language: Option<String>,
    pub translations: Vec<String>,
}

impl Narrow {
    pub fn is_empty(&self) -> bool {
        self.books.is_empty()
            && self.chapters.is_empty()
            && self.language.is_none()
            && self.translations.is_empty()
    }
}

/// Keep only the requested translations, books and chapters, dropping what is
/// left empty.
pub fn narrow_plans(plans: &mut Vec<Plan>, narrow: &Narrow) {
    if narrow.is_empty() {
        return;
    }
    if let Some(language) = &narrow.language {
        plans.retain(|plan| plan.translation.language.eq_ignore_ascii_case(language));
    }
    if !narrow.translations.is_empty() {
        plans.retain(|plan| {
            narrow
                .translations
                .iter()
                .any(|wanted| wanted.eq_ignore_ascii_case(&plan.translation.code))
        });
    }
    for plan in plans.iter_mut() {
        plan.books.retain_mut(|(book, chapters)| {
            if !narrow.books.is_empty()
                && !narrow
                    .books
                    .iter()
                    .any(|wanted| wanted.eq_ignore_ascii_case(&book.usfm))
            {
                return false;
            }
            if !narrow.chapters.is_empty() {
                chapters.retain(|chapter| narrow.chapters.contains(chapter));
            }
            !chapters.is_empty()
        });
    }
    plans.retain(|plan| !plan.books.is_empty());
}

/// Resolve a target's `books` list: USFM ids, or the groups `ot`/`nt`/`dc`/`all`,
/// always limited to what the translation actually covers.
fn expand_books(catalog: &Catalog, translation: &Resolved, list: &[String]) -> Result<Vec<Book>> {
    let in_scope = translation.books(catalog)?;
    let mut chosen: Vec<Book> = Vec::new();
    for token in list {
        let group = token.to_ascii_lowercase();
        if matches!(group.as_str(), "ot" | "nt" | "dc" | "all") {
            for book in &in_scope {
                let wanted = group == "all" || book.testament == group;
                if wanted && !chosen.iter().any(|existing| existing.usfm == book.usfm) {
                    chosen.push(book.clone());
                }
            }
            continue;
        }
        let book = catalog
            .book(token)
            .with_context(|| format!("unknown book {token}"))?;
        if !chosen.iter().any(|existing| existing.usfm == book.usfm) {
            chosen.push(book.clone());
        }
    }
    Ok(chosen)
}

fn target_ids(catalog: &Catalog, target: &Target) -> Vec<(String, String)> {
    let mut ids = catalog.translations_for_site(&target.site);
    if !target.languages.is_empty() {
        ids.retain(|(language, _)| target.languages.iter().any(|l| l == language));
    }
    if !target.translations.is_empty() {
        ids.retain(|(_, code)| {
            target
                .translations
                .iter()
                .any(|c| c.eq_ignore_ascii_case(code))
        });
    }
    ids
}

/// Reuse the running Chromium, and start one only if there is none yet or the
/// process behind it has died.
async fn ensure_session(
    session: &mut Option<Session>,
    config: &Config,
    root: &Path,
    user_agent_index: usize,
) -> Result<()> {
    if session.as_ref().is_some_and(Session::is_alive) {
        return Ok(());
    }
    if let Some(dead) = session.take() {
        dead.close().await.ok();
    }
    *session = Some(
        browser::launch(&config.browser, &config.proxy, root, user_agent_index).await?,
    );
    Ok(())
}

pub async fn run(
    root: &Path,
    config: &Config,
    catalog: &Catalog,
    only: Option<&str>,
    narrow: &Narrow,
) -> Result<Report> {
    let mut plans = plan_targets_filtered(catalog, config, only)?;
    narrow_plans(&mut plans, narrow);
    if plans.is_empty() && !narrow.is_empty() {
        bail!("no chapter matches --lang/--translation/--book/--chapter");
    }
    let mut report = Report::new();

    if plans.is_empty() {
        report.finish();
        return Ok(report);
    }

    let scrape = fetch_plans(root, config, plans, report);
    if config.proxy_pool.enabled {
        crate::proxy::with_proxy(&config.proxy_pool, scrape).await
    } else {
        scrape.await
    }
}

async fn fetch_plans(root: &Path, config: &Config, plans: Vec<Plan>, mut report: Report) -> Result<Report> {
    let store = Store::new(root, &config.output);
    let timing = FetchTiming::from_config(&config.timing);
    let policy = RetryPolicy::new(
        config.retry.max_attempts,
        config.retry.initial_delay_ms,
        config.retry.max_delay_ms,
        config.retry.backoff_multiplier,
        config.retry.jitter_factor,
    )?;

    let mut session: Option<Session> = None;
    let user_agent_index = 0usize;

    for plan in plans {
        let translation = plan.translation;
        let Some(site) = sites::implementation(&translation.module) else {
            eprintln!(
                "skipping {}/{}: no module named {}",
                translation.language, translation.code, translation.module
            );
            continue;
        };

        for (book, chapters) in plan.books {
            let mut manifest = Manifest {
                language: translation.language.clone(),
                translation: translation.code.clone(),
                site: translation.site.clone(),
                book: book.usfm.clone(),
                chapters: Default::default(),
            };

            // Whole-book sites deliver every chapter in one document: one fetch.
            let book_level = translation.unit == Unit::Book;
            let fetch_targets: Vec<u32> = if book_level {
                chapters.first().copied().into_iter().collect()
            } else {
                chapters.clone()
            };

            for chapter in fetch_targets {
                let url = match translation.url(&book, chapter) {
                    Ok(url) => url,
                    Err(error) => {
                        report.record_failure(
                            format!("{}/{}", translation.code, book.usfm),
                            "",
                            format!("{error:#}"),
                        );
                        continue;
                    }
                };
                let path = store.html_path(&translation, &book, chapter);
                let chapter_ref = ChapterRef {
                    translation: &translation,
                    book: &book,
                    chapter,
                };

                if config.scrape.skip_existing {
                    if let Some(existing) = Store::read(&path) {
                        if site.validate(&existing, &chapter_ref).is_ok() {
                            report.record_skipped();
                            manifest.chapters.insert(
                                chapter,
                                ChapterEntry::new(url, site.verse_count(&existing, &chapter_ref), existing.len()),
                            );
                            continue;
                        }
                    }
                }

                let ready = translation.ready.clone().unwrap_or_default();
                let mut attempt = 1u32;

                // One browser for the whole run. The proxy endpoint never
                // changes, and 8889 picks the region per connection, so a retry
                // is just another navigation on the same page.
                let result = loop {
                    ensure_session(&mut session, config, root, user_agent_index).await?;
                    let outcome = {
                        let page = &session.as_ref().expect("session was just created").page;
                        fetch::load_chapter(page, &url, &ready, site, &chapter_ref, &timing).await
                    };
                    match outcome {
                        Ok(html) => break Ok(html),
                        Err(error) => {
                            eprintln!(
                                "Attempt {attempt}/{} failed for {url}: {error:#}",
                                policy.max_attempts
                            );
                            // A permanent answer from the site is not worth
                            // another navigation or a new region.
                            if error.downcast_ref::<fetch::PermanentFailure>().is_some() {
                                eprintln!("not retrying {url}: the site answered a permanent error");
                                break Err(error);
                            }
                            if attempt >= policy.max_attempts {
                                break Err(error);
                            }
                            tokio::time::sleep(policy.delay(attempt)).await;
                            attempt += 1;
                        }
                    }
                };

                match result {
                    Ok(html) => {
                        if config.scrape.verify_after_save {
                            if let Err(error) = site.validate(&html, &chapter_ref) {
                                report.record_failure(&url, path.display().to_string(), format!("{error:#}"));
                                continue;
                            }
                        }
                        let verses = site.verse_count(&html, &chapter_ref);
                        match Store::save_atomic(&path, html.as_bytes()) {
                            Ok(()) => {
                                report.record_saved();
                                manifest.chapters.insert(
                                    chapter,
                                    ChapterEntry::new(url, verses, html.len()),
                                );
                            }
                            Err(error) => report.record_failure(
                                &url,
                                path.display().to_string(),
                                format!("{error:#}"),
                            ),
                        }
                    }
                    Err(error) => report.record_failure(
                        &url,
                        path.display().to_string(),
                        format!("{error:#}"),
                    ),
                }
            }

            // The single book document covers every chapter; record each one.
            if book_level {
                if let Some(html) = Store::read(&store.html_path(&translation, &book, 1)) {
                    for chapter in &chapters {
                        let chapter_ref = ChapterRef {
                            translation: &translation,
                            book: &book,
                            chapter: *chapter,
                        };
                        if site.validate(&html, &chapter_ref).is_ok() {
                            let url = translation.url(&book, *chapter).unwrap_or_default();
                            manifest.chapters.insert(
                                *chapter,
                                ChapterEntry::new(
                                    url,
                                    site.verse_count(&html, &chapter_ref),
                                    html.len(),
                                ),
                            );
                        }
                    }
                }
            }

            if config.scrape.write_manifest && !manifest.chapters.is_empty() {
                let manifest_path = store.manifest_path(&translation, &book);
                if let Err(error) = store.write_manifest(&manifest_path, &manifest) {
                    eprintln!("{}: {error:#}", manifest_path.display());
                }
            }
        }
    }

    if let Some(session) = session {
        session.close().await.ok();
    }
    report.finish();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::from_str(archon_pipeline::CATALOG).unwrap()
    }

    fn tokens(list: &[&str]) -> Vec<String> {
        list.iter().map(|token| token.to_string()).collect()
    }

    #[test]
    fn book_groups_expand_within_the_translation_scope() {
        let catalog = catalog();
        let oeb = catalog.resolve("en", "OEB").unwrap();

        let books = expand_books(&catalog, &oeb, &tokens(&["nt"])).unwrap();
        assert_eq!(books.len(), 26, "NT minus Revelation");
        assert!(!books.iter().any(|book| book.usfm == "REV"));

        let books = expand_books(&catalog, &oeb, &tokens(&["all"])).unwrap();
        assert_eq!(books.len(), 26, "`all` still respects the scope");
    }

    #[test]
    fn explicit_books_and_groups_combine_without_duplicates() {
        let catalog = catalog();
        let niv = catalog.resolve("en", "NIV").unwrap();

        let books = expand_books(&catalog, &niv, &tokens(&["nt", "GEN", "nt"])).unwrap();
        assert_eq!(books.len(), 28);
        assert_eq!(books.last().unwrap().usfm, "GEN");

        let both = expand_books(&catalog, &niv, &tokens(&["ot", "nt"])).unwrap();
        assert_eq!(both.len(), 66);
    }

    #[test]
    fn unknown_books_are_reported() {
        let catalog = catalog();
        let niv = catalog.resolve("en", "NIV").unwrap();
        assert!(expand_books(&catalog, &niv, &tokens(&["NOPE"])).is_err());
    }
}

/// Human-readable plan, used by `scrape --dry-run`.
pub fn describe(
    root: &Path,
    catalog: &Catalog,
    config: &Config,
    only: Option<&str>,
    narrow: &Narrow,
) -> Result<String> {
    let store = Store::new(root, &config.output);
    let mut out = String::new();
    let mut total = 0usize;
    let mut plans = plan_targets_filtered(catalog, config, only)?;
    narrow_plans(&mut plans, narrow);
    if plans.is_empty() && !narrow.is_empty() {
        bail!("no chapter matches --lang/--translation/--book/--chapter");
    }
    for plan in plans {
        let translation = &plan.translation;
        let mut count = 0usize;
        for (_book, chapters) in &plan.books {
            count += chapters.len();
        }
        total += count;
        let fetches = if translation.unit == Unit::Book {
            plan.books.len()
        } else {
            count
        };
        out.push_str(&format!(
            "{}/{} {:<14} {} books, {} chapters, {} fetch(es)\n",
            translation.language,
            translation.code,
            translation.site,
            plan.books.len(),
            count,
            fetches
        ));
        for (book, chapters) in &plan.books {
            let count = if translation.unit == Unit::Book { 1 } else { chapters.len() };
            for &chapter in chapters.iter().take(count) {
                let url = translation.url(book, chapter)?;
                let path = store.html_path(translation, book, chapter);
                out.push_str(&format!("    {url}\n      -> {}\n", path.display()));
            }
        }
    }
    out.push_str(&format!("\n{total} chapter(s) planned at {}\n", now()));
    Ok(out)
}
