//! BibleHub interlinear — one table per word, with Strong's number,
//! transliteration, original text, gloss and parsing code.

use anyhow::{Result, bail};
use scraper::{ElementRef, Html, Selector};
use std::collections::{BTreeMap, HashSet};

use crate::model::{
    ChapterBody, CrossRef, CrossRefLink, InterlinearChapterData, InterlinearVerse, InterlinearWord,
    normalized,
};
use crate::sites::{ChapterRef, Site};

pub struct BibleHub;

impl Site for BibleHub {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        if !html.contains("<html") || !html.contains("</html>") {
            bail!("page is not complete HTML");
        }
        if !html.contains("strongs") || !html.contains("reftop") {
            bail!("no interlinear words found");
        }
        let verses = verse_numbers(html);
        if !verses.contains(&1) {
            bail!("first verse missing from {}", chapter.passage());
        }
        if verses.len() < 2 {
            bail!("only {} verse(s) found", verses.len());
        }
        Ok(())
    }

    fn verse_count(&self, _html: &str, _chapter: &ChapterRef<'_>) -> usize {
        verse_numbers(_html).len()
    }

    fn parse(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterBody> {
        parse_chapter(html, chapter).map(ChapterBody::Interlinear)
    }
}

fn selectors() -> (
    Selector,
    Selector,
    Selector,
    Selector,
    Selector,
    Selector,
    Selector,
) {
    (
        Selector::parse("span.translit").unwrap(),
        Selector::parse("span.strongs a").unwrap(),
        Selector::parse("span.hebrew").unwrap(),
        Selector::parse("span.greek").unwrap(),
        Selector::parse("span.eng").unwrap(),
        Selector::parse("span.strongsnt").unwrap(),
        Selector::parse("span.reftop").unwrap(),
    )
}

fn verse_numbers(html: &str) -> HashSet<u32> {
    let document = Html::parse_document(html);
    let (_, _, _, _, _, _, reftop) = selectors();
    document
        .select(&reftop)
        .filter_map(|span| normalized(&span.text().collect::<String>()).parse().ok())
        .collect()
}

fn text_of(element: ElementRef<'_>) -> String {
    normalized(&element.text().collect::<String>())
}

fn first_text(element: ElementRef<'_>, selector: &Selector) -> Option<String> {
    element
        .select(selector)
        .map(text_of)
        .find(|text| !text.is_empty())
}

fn strongs_number(element: ElementRef<'_>, selector: &Selector) -> String {
    element
        .select(selector)
        .map(text_of)
        .find(|text| text.chars().all(|c| c.is_ascii_digit()) && !text.is_empty())
        .unwrap_or_default()
}

/// Verse headings and the cross references they carry.
///
/// A heading (`div.hdg`) belongs to the next verse table, and its
/// `div.cross a` links are that verse's cross references.
fn heading_data(document: &Html, chapter: &ChapterRef<'_>) -> (BTreeMap<u32, String>, Vec<CrossRef>) {
    let mut headers = BTreeMap::new();
    let mut cross_refs: Vec<CrossRef> = Vec::new();
    let chapters = Selector::parse("div.chap").unwrap();
    let cross_links = Selector::parse("div.cross a").unwrap();
    let (_, _, _, _, _, _, reftop) = selectors();
    let Some(chapter_div) = document.select(&chapters).next() else {
        return (headers, cross_refs);
    };

    let mut pending_header: Option<String> = None;
    let mut pending_links: Vec<CrossRefLink> = Vec::new();
    for child in chapter_div.children() {
        let Some(element) = ElementRef::wrap(child) else {
            continue;
        };
        match element.value().name() {
            "div" if element.value().attr("class") == Some("hdg") => {
                pending_header = element
                    .children()
                    .filter_map(|node| node.value().as_text())
                    .map(|text| text.trim())
                    .find(|text| !text.is_empty())
                    .map(str::to_owned);
                pending_links = element
                    .select(&cross_links)
                    .filter_map(|link| parse_reference(&text_of(link)))
                    .collect();
            }
            "table" => {
                let Some(verse) = first_text(element, &reftop)
                    .and_then(|text| text.parse::<u32>().ok())
                    .filter(|number| *number > 0)
                else {
                    continue;
                };
                if let Some(header) = pending_header.take() {
                    headers.insert(verse, header);
                }
                if !pending_links.is_empty() {
                    let links = std::mem::take(&mut pending_links);
                    match cross_refs.iter_mut().find(|entry| entry.verse == verse) {
                        Some(existing) => existing.links.extend(links),
                        None => cross_refs.push(CrossRef {
                            id: format!("{}:{}:{verse}", chapter.book.short, chapter.chapter),
                            book: chapter.book.name.clone(),
                            chapter: chapter.chapter,
                            verse,
                            links,
                        }),
                    }
                }
            }
            _ => {}
        }
    }
    (headers, cross_refs)
}

/// `John 1:1-5` or `1 Samuel 1:1` → a cross-reference link.
fn parse_reference(text: &str) -> Option<CrossRefLink> {
    let (reference, verses) = text.trim().rsplit_once(':')?;
    let (book, chapter) = reference.rsplit_once(' ')?;
    let book = book.trim();
    if book.is_empty() {
        return None;
    }
    let chapter: u32 = chapter.trim().parse().ok()?;
    let (start, end) = match verses.trim().split_once('-') {
        Some((start, end)) => (
            start.trim().parse().ok()?,
            Some(end.trim().parse().ok()?),
        ),
        None => (verses.trim().parse().ok()?, None),
    };
    Some(CrossRefLink {
        book: book.to_owned(),
        chapter,
        verse: if end.is_none() { Some(start) } else { None },
        verse_start: if end.is_some() { Some(start) } else { None },
        verse_end: end,
    })
}

fn parse_chapter(html: &str, chapter: &ChapterRef<'_>) -> Result<InterlinearChapterData> {
    let document = Html::parse_document(html);
    let (translit, strongs, hebrew, greek, english, grammar, reftop) = selectors();

    let mut grouped: BTreeMap<u32, Vec<InterlinearWord>> = BTreeMap::new();
    // Only the first word of a verse carries a `reftop`; later words inherit it.
    let mut current_verse: Option<u32> = None;
    for marker in document.select(&translit) {
        // The word table is the nearest ancestor table; everything belonging to
        // this word is inside it, including the nested original/gloss table.
        let Some(table) = marker
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|element| element.value().name() == "table")
        else {
            continue;
        };
        if let Some(verse) = first_text(table, &reftop)
            .and_then(|text| text.parse::<u32>().ok())
            .filter(|number| *number > 0)
        {
            current_verse = Some(verse);
        }
        let Some(verse) = current_verse else {
            continue;
        };
        // The original text belongs to this word alone. Reading it from the
        // whole word table is wrong on pages where several words share one
        // table: the nearest ancestor that actually holds a Hebrew (or Greek)
        // span is the word's own container.
        let container = marker
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|element| {
                element.select(&hebrew).next().is_some() || element.select(&greek).next().is_some()
            })
            .unwrap_or(table);
        let original =
            first_text(container, &hebrew).or_else(|| first_text(container, &greek));
        let word = InterlinearWord {
            strongs: strongs_number(table, &strongs),
            reference: format!("{}.{verse}", chapter.passage()),
            transliteration: text_of(marker),
            original: original.unwrap_or_default(),
            english: first_text(table, &english).unwrap_or_default(),
            grammar: first_text(table, &grammar).unwrap_or_default(),
        };
        if word.transliteration.is_empty() && word.original.is_empty() {
            continue;
        }
        grouped.entry(verse).or_default().push(word);
    }

    if grouped.is_empty() {
        bail!("No interlinear words found in {}", chapter.passage());
    }

    let (headers, cross_refs) = heading_data(&document, chapter);
    let verses = grouped
        .into_iter()
        .map(|(verse, words)| InterlinearVerse {
            chapter: chapter.chapter,
            verse,
            header: headers.get(&verse).cloned().unwrap_or_default(),
            words,
        })
        .collect::<Vec<_>>();

    Ok(InterlinearChapterData {
        translation_code: chapter.translation.code.clone(),
        translation_name: chapter.translation.name.clone(),
        book_code: chapter.book.usfm.clone(),
        book_name: chapter.book.name.clone(),
        chapter: chapter.chapter,
        verses,
        crossrefs: cross_refs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn chapter_ref() -> ChapterRef<'static> {
        let catalog: &'static Catalog = Box::leak(Box::new(
            Catalog::from_str(include_str!("../../catalog.toml")).unwrap(),
        ));
        ChapterRef {
            translation: Box::leak(Box::new(catalog.resolve("en", "BIBLEHUB").unwrap())),
            book: catalog.book("GEN").unwrap(),
            chapter: 1,
        }
    }

    fn word(strongs: &str, translit: &str, original: &str, english: &str, grammar: &str, verse: u32) -> String {
        format!(
            r##"<table class="tablefloatheb"><tbody><tr><td><span class="strongs"><a href="/hebrew/{strongs}.htm">{strongs}</a></span>&nbsp;<span class="strongs"><a href="/hebrew/strongs_{strongs}.htm">[e]</a></span><span class="reftop">&nbsp;&nbsp;&nbsp;{verse}</span><br><span class="translit"><a href="#">{translit}</a></span><span class="reftrans">&nbsp;&nbsp;&nbsp;{verse}</span><br><table><tbody><tr><td><span class="hebrew">{original}</span></td><td><span class="refheb">&nbsp;&nbsp;&nbsp;{verse}</span></td></tr><tr><td><span class="eng">{english}</span><span class="refbot">&nbsp;&nbsp;&nbsp;{verse}</span><br><span class="strongsnt"><a href="#">{grammar}</a></span></td></tr></tbody></table></td></tr></tbody></table>"##
        )
    }

    #[test]
    fn parses_interlinear_words_grouped_by_verse() {
        let chapter = chapter_ref();
        let html = format!(
            "<html><body><div id=\"topheading\">Genesis 1</div>{}{}{}</body></html>",
            word("7225", "bə·rê·šîṯ", "בְּרֵאשִׁ֖ית", "In the beginning", "Prep‑b | N‑fs", 1),
            word("1254", "bā·rā", "בָּרָ֣א", "created", "Verb", 1),
            word("430", "ʾĕ·lō·hîm", "אֱלֹהִ֑ים", "God", "Noun", 2),
        );
        assert!(BibleHub.validate(&html, &chapter).is_ok());
        assert_eq!(BibleHub.verse_count(&html, &chapter), 2);

        let ChapterBody::Interlinear(data) = BibleHub.parse(&html, &chapter).unwrap() else {
            panic!("expected interlinear")
        };
        assert_eq!(data.verses.len(), 2);
        assert_eq!(data.verses[0].words.len(), 2);
        assert_eq!(data.verses[0].words[0].strongs, "7225");
        assert_eq!(data.verses[0].words[0].transliteration, "bə·rê·šîṯ");
        assert_eq!(data.verses[0].words[0].original, "בְּרֵאשִׁ֖ית");
        assert_eq!(data.verses[0].words[0].english, "In the beginning");
        assert_eq!(data.verses[0].words[0].grammar, "Prep‑b | N‑fs");
        assert_eq!(data.verses[0].words[0].reference, "GEN.1.1");
        assert_eq!(data.verses[1].words[0].strongs, "430");
    }

    #[test]
    fn reads_headings_and_cross_references() {
        let chapter = chapter_ref();
        let html = format!(
            r##"<html><body><div class="chap"><div class="hdg">The Beginning<br><div class="cross">(<a href="../john/1.htm">John 1:1-5</a>)</div></div>{}{}</div></body></html>"##,
            word("7225", "bə·rê·šîṯ", "בְּרֵאשִׁ֖ית", "In the beginning", "Prep", 1),
            word("430", "ʾĕ·lō·hîm", "אֱלֹהִ֑ים", "God", "Noun", 2),
        );
        let ChapterBody::Interlinear(data) = BibleHub.parse(&html, &chapter).unwrap() else {
            panic!("expected interlinear")
        };
        assert_eq!(data.verses[0].header, "The Beginning");
        assert_eq!(data.verses[1].header, "");
        assert_eq!(data.crossrefs.len(), 1);
        let cross_ref = &data.crossrefs[0];
        assert_eq!(cross_ref.verse, 1);
        assert_eq!(cross_ref.book, "Genesis");
        assert_eq!(
            (
                cross_ref.links[0].book.as_str(),
                cross_ref.links[0].chapter,
                cross_ref.links[0].verse_start,
                cross_ref.links[0].verse_end
            ),
            ("John", 1, Some(1), Some(5))
        );
    }

    #[test]
    fn rejects_a_page_without_interlinear_data() {
        let chapter = chapter_ref();
        let html = "<html><body><div id=\"topheading\">Genesis 1</div><p>nothing here</p></body></html>";
        assert!(BibleHub.validate(html, &chapter).is_err());
        assert!(BibleHub.parse(html, &chapter).is_err());
    }
}
