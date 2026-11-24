//! Convert saved HTML with the original site builders.
mod biblegateway;
mod biblehub;
mod bibleoeb;
mod biblestudy;

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::thread;
use walkdir::WalkDir;

use archon_pipeline::catalog::{Book, Catalog, Resolved, Unit};
use archon_pipeline::config::Config;
use archon_pipeline::engine::store::Store;
use archon_pipeline::sites::{self, ChapterRef};
use crate::Outcome;

type Chapter = (u32, Vec<u8>);

fn render(number: u32, data: &impl Serialize, pretty: bool) -> Result<Chapter> {
    let bytes = if pretty {
        serde_json::to_vec_pretty(data)?
    } else {
        serde_json::to_vec(data)?
    };
    Ok((number, bytes))
}

struct Source<'a> {
    path: PathBuf,
    translation: &'a Resolved,
    book: &'a Book,
    chapter: u32,
    priority: u8,
}

impl Source<'_> {
    fn convert(&self, catalog: &Catalog, pretty: bool) -> Result<Vec<Chapter>> {
        let path = &self.path;
        let book = &self.book.short;
        match self.translation.module.as_str() {
            "bible_gateway" => {
                let aliases: Vec<_> = catalog
                    .book_aliases
                    .iter()
                    .filter(|(name, id)| **id == self.book.usfm && **name != self.book.short)
                    .map(|(name, _)| name.as_str())
                    .collect();
                biblegateway::convert(path, book, &aliases, pretty)
            }
            "bible_hub" => biblehub::convert(path, book, pretty),
            "study_bible" => biblestudy::convert(path, book, pretty),
            "oeb" => bibleoeb::convert(path, book, &self.translation.code, pretty),
            "sacred_bible" => {
                let bytes = std::fs::read(path)?;
                // Chromium saves UTF-8, but the publisher also distributes
                // HTML declared as Windows-1252.
                let html = match std::str::from_utf8(&bytes) {
                    Ok(html) => std::borrow::Cow::Borrowed(html),
                    Err(_) => encoding_rs::WINDOWS_1252.decode(&bytes).0,
                };
                let reference = ChapterRef {
                    book: self.book,
                    translation: self.translation,
                    chapter: self.chapter,
                };
                sites::sacred_bible::book_chapters(&html, &reference)?
                    .into_iter()
                    .map(|(number, data)| render(number, &data, pretty))
                    .collect()
            }
            module => {
                let site = sites::implementation(module)
                    .with_context(|| format!("unknown site {module}"))?;
                let html = std::fs::read_to_string(path)?;
                let reference = ChapterRef {
                    book: self.book,
                    translation: self.translation,
                    chapter: self.chapter,
                };
                if site.verse_count(&html, &reference) == 0 {
                    return Ok(Vec::new());
                }
                Ok(vec![render(
                    self.chapter,
                    &site.parse(&html, &reference)?,
                    pretty,
                )?])
            }
        }
    }
}

pub fn run_selected(
    root: &Path,
    config: &Config,
    catalog: &Catalog,
    translations: &[&str],
) -> Result<Outcome> {
    let ids = catalog.translation_ids();
    for &selection in translations {
        anyhow::ensure!(
            ids.iter().any(|(_, code)| code.eq_ignore_ascii_case(selection)),
            "Unknown translation {selection:?}"
        );
    }
    let store = Store::new(root, &config.output);
    let mut outcome = Outcome::default();
    let mut editions = BTreeMap::new();
    for (lang, code) in ids {
        if !translations.is_empty()
            && !translations.iter().any(|selection| code.eq_ignore_ascii_case(selection))
        {
            continue;
        }
        let translation = catalog
            .resolve(&lang, &code)
            .context("unknown translation")?;
        editions.insert((lang, code), translation);
    }
    let mut books = BTreeMap::new();
    for book in &catalog.books {
        books.insert(book.short.as_str(), book);
        books.insert(book.usfm.as_str(), book);
    }
    for (alias, id) in &catalog.book_aliases {
        books.insert(
            alias,
            catalog.book(id).context("unknown book alias target")?,
        );
    }

    let mut sources: BTreeMap<_, Source> = BTreeMap::new();
    for entry in WalkDir::new(&store.gospels).min_depth(4).max_depth(4) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let parts: Vec<_> = path
            .strip_prefix(&store.gospels)?
            .iter()
            .filter_map(|p| p.to_str())
            .collect();
        let [lang, first, second, file] = parts.as_slice() else {
            continue;
        };
        if !file.ends_with(".html") {
            continue;
        }
        let (edition, book, priority) = if let Some(book) = books.get(first) {
            let edition = editions
                .get(&(lang.to_string(), second.to_string()))
                .or_else(|| {
                    // The merged English archive also contains Greek and Hebrew editions.
                    editions.values().find(|t| {
                        *lang == "en"
                            && matches!(t.language.as_str(), "gr" | "he")
                            && t.code == *second
                    })
                });
            (edition, Some(*book), u8::from(*first != book.short))
        } else {
            (
                editions.get(&(lang.to_string(), first.to_string())),
                books.get(second).copied(),
                2,
            )
        };
        let (Some(translation), Some(book)) = (edition, book) else {
            continue;
        };
        let chapter = if translation.unit == Unit::Book && *file == "book.html" {
            1
        } else {
            let Some(n) = file
                .strip_prefix("chapter_")
                .and_then(|s| s.strip_suffix(".html"))
                .and_then(|s| s.parse::<u32>().ok())
            else {
                continue;
            };
            n
        };
        let key = (
            &translation.language,
            &translation.code,
            &book.usfm,
            chapter,
        );
        let source = Source {
            path: path.to_owned(),
            translation,
            book,
            chapter,
            priority,
        };
        if sources
            .get(&key)
            .is_none_or(|old| (priority, path) < (old.priority, old.path.as_path()))
        {
            sources.insert(key, source);
        }
    }

    let files: Vec<_> = sources.into_values().collect();
    let total = files.len();
    let mut progress = io::stderr();
    if total == 0 {
        writeln!(progress, "Corpus 0/0 files")?;
        return Ok(outcome);
    }

    // Sorted chapter inputs have distinct destinations. Whole-book sources for
    // the same edition and book can overwrite one another's chapter outputs.
    let queue = Mutex::new(files.chunk_by(|a, b| {
        a.translation.unit == Unit::Book
            && std::ptr::eq(a.translation, b.translation)
            && std::ptr::eq(a.book, b.book)
    }));
    enum Event<'a> {
        Started(usize, &'a Resolved),
        Finished(usize, Result<Outcome>),
    }
    thread::scope(|scope| -> Result<()> {
        let (sender, receiver) = mpsc::channel();
        for worker in 0..total.min(2) {
            let sender = sender.clone();
            let queue = &queue;
            let store = &store;
            scope.spawn(move || {
                loop {
                    let group = queue.lock().unwrap().next();
                    let Some(group) = group else { break };
                    for source in group {
                        if sender
                            .send(Event::Started(worker, source.translation))
                            .is_err()
                        {
                            return;
                        }
                        let mut file = Outcome::default();
                        let result =
                            process(source, store, config, catalog, &mut file).map(|()| file);
                        let failed = result.is_err();
                        if sender.send(Event::Finished(worker, result)).is_err() || failed {
                            return;
                        }
                    }
                }
            });
        }
        drop(sender);
        let interactive = progress.is_terminal();
        let mut active = [None, None];
        let mut done = 0;
        let mut last_width = 0;
        for event in receiver {
            match event {
                Event::Started(worker, translation) => active[worker] = Some(translation),
                Event::Finished(worker, result) => {
                    active[worker] = None;
                    let file = result?;
                    outcome.converted += file.converted;
                    outcome.skipped += file.skipped;
                    outcome.failed.extend(file.failed);
                    done += 1;
                }
            }
            if interactive || done == total {
                let filled = 24 * done / total;
                let mut line = format!(
                    "Corpus [{}{}] {done}/{total} files",
                    "#".repeat(filled),
                    "-".repeat(24 - filled)
                );
                for (worker, translation) in active.iter().enumerate() {
                    if let Some(t) = translation {
                        line.push_str(&format!("  W{} {}/{}", worker + 1, t.language, t.code));
                    }
                }
                write!(
                    progress,
                    "{}{line:<last_width$}",
                    if interactive { "\r" } else { "" }
                )?;
                if done == total {
                    writeln!(progress)?;
                }
                progress.flush()?;
                last_width = line.len();
            }
        }
        Ok(())
    })?;
    outcome.failed.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(outcome)
}

fn process(
    source: &Source<'_>,
    store: &Store,
    config: &Config,
    catalog: &Catalog,
    outcome: &mut Outcome,
) -> Result<()> {
    let destination = |chapter| store.json_path(source.translation, source.book, chapter);
    if config.json.skip_existing
        && source.translation.unit != Unit::Book
        && destination(source.chapter).exists()
    {
        outcome.skipped += 1;
        return Ok(());
    }
    match source.convert(catalog, config.json.pretty) {
        Ok(chapters) => {
            if chapters.is_empty() {
                outcome.skipped += 1;
            }
            for (number, bytes) in chapters {
                // Only a whole-book input determines multiple output filenames.
                let output = if source.translation.unit == Unit::Book {
                    destination(number)
                } else {
                    destination(source.chapter)
                };
                if config.json.skip_existing && output.exists() {
                    outcome.skipped += 1;
                    continue;
                }
                Store::save_atomic(&output, &bytes)?;
                outcome.converted += 1;
            }
        }
        Err(error) => outcome
            .failed
            .push((source.path.clone(), format!("{error:#}"))),
    }
    Ok(())
}
