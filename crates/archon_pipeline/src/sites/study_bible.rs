//! StudyBible.info — one `div.passage` per translation, with `sup > a.verse_ref`
//! markers inline in the text.

use anyhow::{Context, Result, bail};
use scraper::{ElementRef, Html, Selector};
use std::collections::{BTreeMap, HashSet};

use crate::model::{ChapterBody, ChapterData, ContentItem, Verse, normalized};
use crate::sites::{ChapterRef, Site};

pub struct StudyBible;

impl Site for StudyBible {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        let verses = verse_numbers(html, chapter);
        if !html.contains("<html") || !html.contains("</html>") {
            bail!("page is not complete HTML");
        }
        if !html.contains("parallel_passages_container") {
            bail!("missing parallel passages container");
        }
        if !verses.contains(&1) {
            bail!("first verse missing from {}", chapter.passage());
        }
        if verses.len() < 2 {
            bail!("only {} verse(s) found", verses.len());
        }
        Ok(())
    }

    fn verse_count(&self, html: &str, chapter: &ChapterRef<'_>) -> usize {
        verse_numbers(html, chapter).len()
    }

    fn parse(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterBody> {
        parse_chapter(html, chapter).map(ChapterBody::Standard)
    }
}

/// The `div.passage` block for this translation.
fn passage_root<'a>(document: &'a Html, code: &str) -> Option<ElementRef<'a>> {
    let selector = Selector::parse("div.passage").ok()?;
    document
        .select(&selector)
        .find(|element| element.value().classes().any(|class| class == code))
}

fn verse_marker(element: &ElementRef<'_>) -> Option<u32> {
    element
        .select(&Selector::parse("a.verse_ref").ok()?)
        .next()
        .and_then(|anchor| anchor.text().collect::<String>().trim().parse().ok())
}

fn verse_numbers(html: &str, chapter: &ChapterRef<'_>) -> HashSet<u32> {
    let document = Html::parse_document(html);
    let Some(root) = passage_root(&document, &chapter.translation.code) else {
        return HashSet::new();
    };
    let sups = Selector::parse("sup").unwrap();
    root.select(&sups)
        .filter_map(|sup| verse_marker(&sup))
        .collect()
}



fn parse_chapter(html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterData> {
    let document = Html::parse_document(html);
    let root = passage_root(&document, &chapter.translation.code).with_context(|| {
        format!(
            "Missing {} passage for {}",
            chapter.translation.code,
            chapter.passage()
        )
    })?;
    // The page's own `h2`/`h3` headings are site chrome ("Popular Versions",
    // "Deutsch", …), and the passage block carries none, so there are no
    // headings to attach. The site's per-verse summary comes from the
    // "Treasury of Scripture Knowledge" panel instead.
    let summaries = treasury_summaries(&document, chapter);
    let mut pending_headings: Vec<String> = Vec::new();

    let mut verses: BTreeMap<u32, Verse> = BTreeMap::new();
    let mut current: Option<u32> = None;
    let mut buffer = String::new();
    let mut paragraph_start = true;

    for child in root.children() {
        match child.value() {
            scraper::Node::Text(text) => buffer.push_str(text),
            scraper::Node::Element(_) => {
                let Some(element) = ElementRef::wrap(child) else {
                    continue;
                };
                if element.value().name() == "sup" {
                    if let Some(number) = verse_marker(&element) {
                        flush(&mut buffer, current, &mut verses, chapter, &mut pending_headings, paragraph_start);
                        paragraph_start = false;
                        current = Some(number);
                        continue;
                    }
                    // version_info / footnote markers carry no verse text
                    continue;
                }
                if element.value().name() == "p" {
                    flush(&mut buffer, current, &mut verses, chapter, &mut pending_headings, paragraph_start);
                    paragraph_start = true;
                    buffer.push_str(&element.text().collect::<String>());
                    continue;
                }
                buffer.push_str(&element.text().collect::<String>());
            }
            _ => {}
        }
    }
    flush(&mut buffer, current, &mut verses, chapter, &mut pending_headings, paragraph_start);

    if verses.is_empty() {
        bail!("No verses found in {}", chapter.passage());
    }
    if !verses.contains_key(&1) {
        bail!("First verse missing from {}", chapter.passage());
    }

    for (number, summary) in summaries {
        if let Some(verse) = verses.get_mut(&number) {
            if verse.header.is_empty() {
                verse.header = vec![ContentItem::text("text", summary)];
            }
        }
    }

    let mut data = ChapterData::new(
        &chapter.translation.code,
        &chapter.translation.name,
        &chapter.book.short,
        &chapter.book.name,
        chapter.chapter,
    );
    data.verses = verses.into_values().collect();
    Ok(data)
}

/// The site's one-line summary of each verse, from the "Treasury of Scripture
/// Knowledge" panel: `<a class="reference" href="…11:1">2 Samuel 11:1</a> While
/// Joab besieges Rabbah, David commits adultery with Bath-sheba.`
fn treasury_summaries(
    document: &Html,
    chapter: &ChapterRef<'_>,
) -> BTreeMap<u32, String> {
    let mut summaries = BTreeMap::new();
    let heading = Selector::parse("h3.tsk").unwrap();
    let Some(heading) = document.select(&heading).next() else {
        return summaries;
    };
    let Some(container) = heading
        .next_siblings()
        .filter_map(ElementRef::wrap)
        .find(|element| element.value().classes().any(|class| class == "footer_text"))
    else {
        return summaries;
    };

    let mut current: Option<u32> = None;
    let mut buffer = String::new();
    for child in container.children() {
        match child.value() {
            scraper::Node::Text(text) => buffer.push_str(text),
            scraper::Node::Element(_) => {
                let Some(element) = ElementRef::wrap(child) else {
                    continue;
                };
                if element.value().name() == "br" {
                    flush_summary(&mut current, &mut buffer, &mut summaries);
                    continue;
                }
                if element.value().classes().any(|class| class == "reference") {
                    flush_summary(&mut current, &mut buffer, &mut summaries);
                    current = element
                        .value()
                        .attr("href")
                        .and_then(|href| verse_from_href(href, chapter.chapter));
                    continue;
                }
                buffer.push_str(&element.text().collect::<String>());
            }
            _ => {}
        }
    }
    flush_summary(&mut current, &mut buffer, &mut summaries);
    summaries
}

fn flush_summary(
    verse: &mut Option<u32>,
    buffer: &mut String,
    summaries: &mut BTreeMap<u32, String>,
) {
    let text = normalized(buffer);
    buffer.clear();
    let Some(number) = *verse else {
        return;
    };
    if text.is_empty() {
        return;
    }
    match summaries.get_mut(&number) {
        Some(existing) => {
            existing.push(' ');
            existing.push_str(&text);
        }
        None => {
            summaries.insert(number, text);
        }
    }
}

/// `/BSB/2%20Samuel%2011:1` → 1, when the chapter is 11.
fn verse_from_href(href: &str, chapter: u32) -> Option<u32> {
    // The path is URL-encoded, so `%20` would otherwise glue the chapter onto
    // the encoded space (`…Samuel%2011` → 2011).
    let href = href.replace("%20", " ").replace('+', " ");
    let (reference, verses) = href.rsplit_once(':')?;
    let digits: String = reference
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    let number: u32 = digits.chars().rev().collect::<String>().parse().ok()?;
    if number != chapter {
        return None;
    }
    verses.split('-').next()?.trim().parse().ok()
}

fn flush(
    buffer: &mut String,
    current: Option<u32>,
    verses: &mut BTreeMap<u32, Verse>,
    chapter: &ChapterRef<'_>,
    pending_headings: &mut Vec<String>,
    paragraph_start: bool,
) {
    let text = normalized(buffer);
    buffer.clear();
    let Some(number) = current else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let verse = verses.entry(number).or_insert_with(|| {
        let header = std::mem::take(pending_headings)
            .into_iter()
            .map(|text| ContentItem::text("text", text))
            .collect();
        Verse {
            chapter: chapter.chapter,
            verse: number,
            content: vec![ContentItem::new("verse_number", None, Some(number), None)],
            header,
            section: String::new(),
            paragraph_start,
            poetry_indent: None,
        }
    });
    verse.content.push(ContentItem::text("text", text));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn chapter_ref(catalog: &'static Catalog, code: &str, usfm: &str) -> ChapterRef<'static> {
        ChapterRef {
            translation: Box::leak(Box::new(catalog.resolve("en", code).unwrap())),
            book: catalog.book(usfm).unwrap(),
            chapter: 11,
        }
    }

    fn fixture(code: &str) -> String {
        format!(
            r#"<html><body><div class="parallel_passages_container"><div class="parallel_passages"><article>
<div class="passage row {code}">{code}<sup><a class="version_info" href="/version/{code}">(i)</a></sup>
 <sup><a class="verse_ref {code}" href="/{code}/2%20Samuel%2011:1" title="2 Samuel 11:1 {code}">1</a></sup> In the spring David sent out Joab.
 <sup><a class="verse_ref {code}" href="/{code}/2%20Samuel%2011:2" title="2 Samuel 11:2 {code}">2</a></sup> One evening David got up from his bed.
</div></article></div></div></body></html>"#
        )
    }

    #[test]
    fn parses_verses_from_markers() {
        let catalog: &'static Catalog = Box::leak(Box::new(
            Catalog::from_str(include_str!("../../catalog.toml")).unwrap(),
        ));
        let chapter = chapter_ref(catalog, "BSB", "2SA");
        let html = fixture("BSB");
        assert!(StudyBible.validate(&html, &chapter).is_ok());
        assert_eq!(StudyBible.verse_count(&html, &chapter), 2);

        let ChapterBody::Standard(data) = StudyBible.parse(&html, &chapter).unwrap() else {
            panic!("expected standard")
        };
        assert_eq!(data.verses.len(), 2);
        assert_eq!(
            crate::model::render_text_chapter(&data),
            "1 In the spring David sent out Joab.\n2 One evening David got up from his bed.\n"
        );
    }

    #[test]
    fn reads_the_treasury_summaries_as_headers() {
        let catalog: &'static Catalog = Box::leak(Box::new(
            Catalog::from_str(include_str!("../../catalog.toml")).unwrap(),
        ));
        let chapter = chapter_ref(catalog, "BSB", "2SA");
        let html = format!(
            r##"<html><body><h2>Online Parallel Study Bible</h2>{}
<h3 class="tsk">Treasury of Scripture Knowledge</h3><div class="footer_text">
<br> <a class="reference" href="/BSB/2%20Samuel%2011:1" title="2 Samuel 11:1 BSB">2 Samuel 11:1</a> While Joab besieges Rabbah, David commits adultery with Bath-sheba.
<br> <a class="reference" href="/BSB/2%20Samuel%2011:2" title="2 Samuel 11:2 BSB">2 Samuel 11:2</a> David sees Bath-sheba bathing.
<br> <a class="reference" href="/BSB/2%20Samuel%2012:1" title="2 Samuel 12:1 BSB">2 Samuel 12:1</a> A verse from another chapter.
</div></body></html>"##,
            fixture("BSB")
        );
        let ChapterBody::Standard(data) = StudyBible.parse(&html, &chapter).unwrap() else {
            panic!("expected standard")
        };

        // The page chrome is not a verse heading.
        assert!(!data
            .verses
            .iter()
            .flat_map(|verse| verse.header.iter())
            .any(|item| item.text.as_deref() == Some("Online Parallel Study Bible")));

        assert_eq!(
            data.verses[0].header[0].text.as_deref(),
            Some("While Joab besieges Rabbah, David commits adultery with Bath-sheba.")
        );
        assert_eq!(
            data.verses[1].header[0].text.as_deref(),
            Some("David sees Bath-sheba bathing.")
        );
        // Entries from another chapter are not this chapter's verses.
        assert_eq!(data.verses.len(), 2);
    }

    #[test]
    fn ignores_other_translations_in_the_same_page() {
        let catalog: &'static Catalog = Box::leak(Box::new(
            Catalog::from_str(include_str!("../../catalog.toml")).unwrap(),
        ));
        let chapter = chapter_ref(catalog, "BSB", "2SA");
        let html = fixture("WNT");
        assert!(StudyBible.validate(&html, &chapter).is_err());
    }
}
