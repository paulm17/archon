//! bible.com — `data-usfm` passage markup.

use anyhow::{Context, Result, bail};
use scraper::{ElementRef, Html, Node, Selector};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::model::{
    ChapterBody, ChapterData, ContentItem, Footnote, ReferenceBlock, ReferenceCitation, Verse,
    normalized,
};
use crate::sites::{ChapterRef, Site};

pub struct BibleCom;

impl Site for BibleCom {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        let verses = verse_numbers(html, chapter);
        if !html.contains("<html") || !html.contains("</html>") {
            bail!("page is not complete HTML");
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

/// The chapter root and passage element for this book/chapter/translation.
fn chapter_elements<'a>(
    document: &'a Html,
    chapter: &ChapterRef<'_>,
) -> Option<(ElementRef<'a>, ElementRef<'a>)> {
    let expected = chapter.passage();
    let version = chapter.translation.bible_id?.to_string();
    let roots = Selector::parse("[data-testid=\"chapter-content\"]").ok()?;
    let passages = Selector::parse("div[data-usfm]").ok()?;
    for root in document.select(&roots) {
        if root.value().attr("lang") != Some(chapter.translation.language.as_str()) {
            continue;
        }
        for passage in root.select(&passages) {
            if passage.value().attr("data-usfm") != Some(expected.as_str()) {
                continue;
            }
            // The nearest version container must identify this passage, not a
            // different translation elsewhere on a parallel-reader page.
            let passage_version = std::iter::once(passage)
                .chain(passage.ancestors().filter_map(ElementRef::wrap))
                .find_map(|element| element.value().attr("data-vid"));
            if passage_version == Some(version.as_str()) {
                return Some((root, passage));
            }
        }
    }
    None
}

fn verse_numbers(html: &str, chapter: &ChapterRef<'_>) -> HashSet<u32> {
    let document = Html::parse_document(html);
    let Some((_, passage)) = chapter_elements(&document, chapter) else {
        return HashSet::new();
    };
    let prefix = format!("{}.", chapter.passage());
    verses_from_marker(&passage.html(), &format!("data-usfm=\"{prefix}"), &prefix)
}

fn verses_from_marker(html: &str, marker: &str, prefix: &str) -> HashSet<u32> {
    html.match_indices(marker)
        .filter_map(|(offset, _)| {
            html[offset + marker.len()..]
                .split_once('"')
                .map(|(verses, _)| verses)
        })
        .flat_map(|verses| verses.split('+'))
        .filter_map(|verse| {
            verse
                .strip_prefix(prefix)
                .unwrap_or(verse)
                .parse::<u32>()
                .ok()
        })
        .filter(|number| *number > 0)
        .collect()
}

/// Match a CSS-module class in either shape bible.com serves it: the older
/// `ChapterContent-module__cat7xG__verse` and the newer `cat7xG_verse`.
fn class_is(element: ElementRef<'_>, name: &str) -> bool {
    let double = format!("__{name}");
    let single = format!("_{name}");
    element.value().classes().any(|class| {
        class == name || class.ends_with(&double) || class.ends_with(&single)
    })
}

fn text_of(element: ElementRef<'_>) -> String {
    normalized(&element.text().collect::<String>())
}

fn flush_text(buffer: &mut String, content: &mut Vec<ContentItem>) {
    let text = normalized(buffer);
    if !text.is_empty() {
        content.push(ContentItem::text("text", text));
    }
    buffer.clear();
}

fn note_body(note: ElementRef<'_>) -> String {
    note.descendent_elements()
        .find(|element| class_is(*element, "body"))
        .map(text_of)
        .unwrap_or_else(|| text_of(note))
}

fn note_references(note: ElementRef<'_>) -> BTreeMap<String, ReferenceBlock> {
    let mut references = BTreeMap::new();
    for element in note.descendent_elements() {
        if !element.value().classes().any(|class| class == "ref") {
            continue;
        }
        let Some(usfm) = element.value().attr("data-usfm") else {
            continue;
        };
        let mut parts = usfm.split('.');
        let (Some(book), Some(chapter), Some(verse)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let display = text_of(element);
        if display.is_empty() {
            continue;
        }
        references.insert(
            display.clone(),
            ReferenceBlock {
                display,
                citations: vec![ReferenceCitation {
                    book: book.into(),
                    verses: format!("{chapter}:{verse}"),
                }],
            },
        );
    }
    references
}

fn walk_verse_content(
    element: ElementRef<'_>,
    buffer: &mut String,
    content: &mut Vec<ContentItem>,
    footnotes: &mut Vec<Footnote>,
    note_prefix: &str,
    note_number: &mut usize,
) {
    for child in element.children() {
        match child.value() {
            Node::Text(text) => buffer.push_str(text),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(child) else {
                    continue;
                };
                if class_is(child, "label") {
                    continue;
                }
                if class_is(child, "note") {
                    flush_text(buffer, content);
                    *note_number += 1;
                    let id = format!("{note_prefix}-{note_number}");
                    let marker = child
                        .descendent_elements()
                        .find(|element| class_is(*element, "label"))
                        .map(text_of)
                        .filter(|text| !text.is_empty())
                        .unwrap_or_else(|| "#".into());
                    content.push(ContentItem::new(
                        "footnote",
                        Some(marker),
                        None,
                        Some(id.clone()),
                    ));
                    footnotes.push(Footnote {
                        id,
                        text: note_body(child),
                        references: note_references(child),
                    });
                    continue;
                }
                walk_verse_content(child, buffer, content, footnotes, note_prefix, note_number);
            }
            _ => {}
        }
    }
}

fn parse_verse_number(usfm: &str, prefix: &str) -> Option<u32> {
    usfm.strip_prefix(prefix)
        .and_then(|tail| tail.strip_prefix('.'))
        .map(|tail| tail.split('+').next().unwrap_or(tail))
        .and_then(|tail| tail.parse::<u32>().ok())
        .filter(|number| *number > 0)
}

fn verse_range_display(usfm: &str, prefix: &str) -> Option<String> {
    let numbers = usfm
        .split('+')
        .map(|part| {
            part.strip_prefix(prefix)
                .and_then(|tail| tail.strip_prefix('.'))
                .and_then(|tail| tail.parse::<u32>().ok())
                .filter(|number| *number > 0)
        })
        .collect::<Option<Vec<_>>>()?;
    if numbers.len() < 2 || numbers.windows(2).any(|pair| pair[1] != pair[0] + 1) {
        return None;
    }
    Some(format!("{}-{}", numbers[0], numbers[numbers.len() - 1]))
}

fn escaped_page_field(html: &str, field: &str) -> Option<String> {
    let marker = format!(r#"{field}\":\""#);
    html.split_once(&marker)
        .and_then(|(_, rest)| rest.split_once(r#"\""#))
        .map(|(value, _)| {
            serde_json::from_str::<String>(&format!("\"{value}\""))
                .unwrap_or_else(|_| value.to_owned())
        })
        .filter(|value| !value.is_empty())
}

fn parse_chapter(html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterData> {
    let book = chapter.book;
    let expected_usfm = chapter.passage();
    let document = Html::parse_document(html);
    let h1_selector = Selector::parse("h1").unwrap();
    let (root, chapter_element) = chapter_elements(&document, chapter).with_context(|| {
        format!(
            "Missing expected {} translation/chapter {expected_usfm}",
            chapter.translation.code
        )
    })?;

    let local_book_name = root
        .select(&h1_selector)
        .next()
        .map(text_of)
        .map(|heading| {
            heading
                .strip_suffix(&format!(" {}", chapter.chapter))
                .unwrap_or(&heading)
                .to_owned()
        })
        .unwrap_or_else(|| book.name.clone());
    let translation_code = escaped_page_field(html, "local_abbreviation")
        .unwrap_or_else(|| chapter.translation.code.clone());
    let translation_name = escaped_page_field(html, "local_title")
        .unwrap_or_else(|| chapter.translation.name.clone());

    let verse_selector = Selector::parse("[data-usfm]").unwrap();
    let mut verses: BTreeMap<u32, Verse> = BTreeMap::new();
    let mut footnotes = Vec::new();
    let mut pending_headings = Vec::new();
    let mut note_numbers: HashMap<u32, usize> = HashMap::new();

    for node in chapter_element.children() {
        let Some(block) = ElementRef::wrap(node) else {
            continue;
        };
        if class_is(block, "s1") || class_is(block, "tl") {
            let heading = text_of(block);
            if !heading.is_empty() {
                pending_headings.push(heading);
            }
            continue;
        }
        if class_is(block, "sr") {
            continue;
        }
        let poetry_indent = if class_is(block, "q1") {
            Some(1)
        } else if class_is(block, "q2") {
            Some(2)
        } else {
            None
        };
        let mut first_new_in_block = true;
        let mut poetry_line_started = HashSet::new();
        for span in block.select(&verse_selector) {
            if !class_is(span, "verse") {
                continue;
            }
            let Some(usfm) = span.value().attr("data-usfm") else {
                continue;
            };
            let Some(number) = parse_verse_number(usfm, &expected_usfm) else {
                continue;
            };
            let mut content = Vec::new();
            let mut text_buffer = String::new();
            let note_number = note_numbers.entry(number).or_default();
            let note_prefix = format!("{}:{}:{number}", book.name, chapter.chapter);
            walk_verse_content(
                span,
                &mut text_buffer,
                &mut content,
                &mut footnotes,
                &note_prefix,
                note_number,
            );
            flush_text(&mut text_buffer, &mut content);
            if content.is_empty() {
                continue;
            }
            let verse = verses.entry(number).or_insert_with(|| {
                let header = std::mem::take(&mut pending_headings)
                    .into_iter()
                    .map(|text| ContentItem::text("text", text))
                    .collect();
                let verse = Verse {
                    chapter: chapter.chapter,
                    verse: number,
                    content: vec![ContentItem {
                        display: verse_range_display(usfm, &expected_usfm),
                        ..ContentItem::new("verse_number", None, Some(number), None)
                    }],
                    header,
                    section: String::new(),
                    paragraph_start: first_new_in_block,
                    poetry_indent,
                };
                first_new_in_block = false;
                verse
            });
            if verse.poetry_indent.is_none() {
                verse.poetry_indent = poetry_indent;
            }
            if let Some(indent) = poetry_indent {
                if poetry_line_started.insert(number) {
                    verse
                        .content
                        .push(ContentItem::new("poetry_line", None, Some(indent), None));
                }
            }
            verse.content.extend(content);
        }
    }

    if verses.is_empty() {
        bail!("No verses found in {expected_usfm}");
    }
    if !verses.contains_key(&1) {
        bail!("First verse missing from {expected_usfm}");
    }

    let mut data = ChapterData::new(
        &translation_code,
        &translation_name,
        &book.short,
        &local_book_name,
        chapter.chapter,
    );
    data.verses = verses.into_values().collect();
    data.footnotes = footnotes;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Book, Catalog};

    fn catalog() -> Catalog {
        Catalog::from_str(include_str!("../../catalog.toml")).unwrap()
    }

    fn fixture<'a>(
        catalog: &'a Catalog,
        language: &str,
        code: &str,
        usfm: &'a str,
    ) -> (crate::catalog::Resolved, &'a Book) {
        (catalog.resolve(language, code).unwrap(), catalog.book(usfm).unwrap())
    }

    fn html(lang: &str, bible_id: u32, usfm: &str, chapter: u32, body: &str) -> String {
        format!(
            r#"<html><div data-testid="chapter-content" lang="{lang}"><h1>Name {chapter}</h1><div data-vid="{bible_id}"><div data-usfm="{usfm}.{chapter}">{body}</div></div></div></html>"#
        )
    }

    #[test]
    fn parses_verses_and_merges_split_ones() {
        let catalog = catalog();
        let (translation, book) = fixture(&catalog, "es", "NVI", "MRK");
        let html = html(
            "es",
            128,
            "MRK",
            1,
            r#"<div class="x__p"><span class="x__verse" data-usfm="MRK.1.1"><span class="x__label">1</span>First part</span></div><div class="x__q1"><span class="x__verse" data-usfm="MRK.1.1"><span class="x__content"> continues</span></span><span class="x__verse" data-usfm="MRK.1.2">Second verse</span></div>"#,
        );
        let chapter = ChapterRef { translation: &translation, book, chapter: 1 };
        let body = BibleCom.parse(&html, &chapter).unwrap();
        let ChapterBody::Standard(data) = body else { panic!("expected standard") };
        assert_eq!(data.verses.len(), 2);
        assert_eq!(data.book_code, "Mark");
        assert_eq!(data.verses[0].content[1].text.as_deref(), Some("First part"));
        assert_eq!(data.verses[0].content[2].kind, "poetry_line");
        assert_eq!(data.verses[0].content[3].text.as_deref(), Some("continues"));
        assert_eq!(data.verses[0].poetry_indent, Some(1));
        assert_eq!(crate::model::render_text_chapter(&data), "1 First part continues\n2 Second verse\n");
    }

    #[test]
    fn rejects_the_wrong_language_or_chapter() {
        let catalog = catalog();
        let (translation, book) = fixture(&catalog, "es", "NVI", "MRK");
        let good = html("es", 128, "MRK", 1, r#"<span class="x__verse" data-usfm="MRK.1.1">A</span><span class="x__verse" data-usfm="MRK.1.2">B</span>"#);
        assert!(BibleCom.validate(&good, &ChapterRef { translation: &translation, book, chapter: 1 }).is_ok());
        assert!(BibleCom.validate(&good, &ChapterRef { translation: &translation, book, chapter: 2 }).is_err());

        let pt = catalog.resolve("pt", "NVI").unwrap();
        assert!(BibleCom.validate(&good, &ChapterRef { translation: &pt, book, chapter: 1 }).is_err());
    }

    #[test]
    fn counts_combined_first_verses() {
        let catalog = catalog();
        let (translation, book) = fixture(&catalog, "fr", "NFC", "JHN");
        let html = html(
            "fr",
            2367,
            "JHN",
            4,
            r#"<div class="x__p"><span class="x__verse" data-usfm="JHN.4.1+JHN.4.2+JHN.4.3">Texte commun.</span><span class="x__verse" data-usfm="JHN.4.4">Suivant.</span></div>"#,
        );
        let chapter = ChapterRef { translation: &translation, book, chapter: 4 };
        assert_eq!(BibleCom.verse_count(&html, &chapter), 4);
        assert!(BibleCom.validate(&html, &chapter).is_ok());
        let ChapterBody::Standard(data) = BibleCom.parse(&html, &chapter).unwrap() else { panic!() };
        assert_eq!(data.verses.len(), 2);
        assert_eq!(data.verses[0].content[0].display.as_deref(), Some("1-3"));
    }

    #[test]
    fn numbered_books_cannot_be_confused() {
        let catalog = catalog();
        let (translation, _) = fixture(&catalog, "es", "NVI", "JHN");
        let first_john = catalog.book("1JN").unwrap();
        let john = catalog.book("JHN").unwrap();
        let html = html("es", 128, "1JN", 1, r#"<span class="x__verse" data-usfm="1JN.1.1">A</span><span class="x__verse" data-usfm="1JN.1.2">B</span>"#);
        assert!(BibleCom.validate(&html, &ChapterRef { translation: &translation, book: first_john, chapter: 1 }).is_ok());
        assert!(BibleCom.validate(&html, &ChapterRef { translation: &translation, book: john, chapter: 1 }).is_err());
    }
}
