//! Open English Bible.
//!
//! The contents document holds the whole Bible: books are `<h1>` sections and
//! chapters are `<p class="c-num">[Luke 1]</p>` markers. One fetch yields every
//! book, so `unit = "book"` in the catalog and parsing slices out a chapter.
//!
//! Notes are plain text in the flow, `[Note: …]`, not markup, so they are split
//! out of the verse text and turned into footnote entries with generated
//! markers (a, b, c … per chapter).

use anyhow::{Context, Result, bail};
use scraper::{ElementRef, Html, Node, Selector};
use std::collections::BTreeMap;

use crate::model::{ChapterBody, ChapterData, ContentItem, Footnote, Verse, normalized};
use crate::sites::{ChapterRef, Site};

pub struct Oeb;

impl Site for Oeb {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        let document = Html::parse_document(html);
        let marker = chapter_marker(&document, chapter)
            .with_context(|| format!("Missing chapter marker for {}", chapter.passage()))?;
        let verses = verses_in(&marker, &context(chapter)).verses;
        if !verses.contains_key(&1) {
            bail!("first verse missing from {}", chapter.passage());
        }
        if verses.len() < 2 {
            bail!("only {} verse(s) found", verses.len());
        }
        Ok(())
    }

    fn verse_count(&self, html: &str, chapter: &ChapterRef<'_>) -> usize {
        let document = Html::parse_document(html);
        chapter_marker(&document, chapter)
            .map(|marker| verses_in(&marker, &context(chapter)).verses.len())
            .unwrap_or(0)
    }

    fn parse(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterBody> {
        parse_chapter(html, chapter).map(ChapterBody::Standard)
    }
}

fn is_chapter_marker(element: &ElementRef<'_>) -> bool {
    element.value().name() == "p" && element.value().classes().any(|class| class == "c-num")
}

/// `[Luke 1]` → ("Luke", 1)
fn parse_marker_text(text: &str) -> Option<(String, u32)> {
    let inner = text.trim().trim_start_matches('[').trim_end_matches(']');
    let (book, number) = inner.rsplit_once(' ')?;
    Some((book.trim().to_string(), number.trim().parse().ok()?))
}

fn chapter_marker<'a>(document: &'a Html, chapter: &ChapterRef<'_>) -> Option<ElementRef<'a>> {
    let markers = Selector::parse("p.c-num").ok()?;
    document.select(&markers).find(|element| {
        let text = normalized(&element.text().collect::<String>());
        match parse_marker_text(&text) {
            Some((book, number)) => number == chapter.chapter && book.eq_ignore_ascii_case(&chapter.book.name),
            None => false,
        }
    })
}

fn verse_number(element: &ElementRef<'_>) -> Option<u32> {
    let text = normalized(&element.text().collect::<String>());
    text.trim_matches(['[', ']']).trim().parse().ok()
}

struct NoteContext<'a> {
    book: &'a str,
    chapter: u32,
}

fn context<'a>(chapter: &'a ChapterRef<'_>) -> NoteContext<'a> {
    NoteContext {
        book: &chapter.book.short,
        chapter: chapter.chapter,
    }
}

/// One chapter sliced out of the document, with the notes it carries.
struct ChapterContent {
    verses: BTreeMap<u32, (String, Vec<ContentItem>)>,
    footnotes: Vec<Footnote>,
}

/// `0 → a`, `25 → z`, `26 → aa`, `27 → ab`.
fn note_marker(mut index: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push((b'a' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    letters.iter().rev().collect()
}

/// Collect the verses that follow a chapter marker, up to the next chapter.
fn verses_in(marker: &ElementRef<'_>, context: &NoteContext<'_>) -> ChapterContent {
    let mut content = ChapterContent {
        verses: BTreeMap::new(),
        footnotes: Vec::new(),
    };
    let mut current: Option<u32> = None;
    let mut buffer = String::new();
    let mut heading = String::new();
    let mut note_index = 0usize;

    for sibling in marker.next_siblings() {
        let Some(element) = ElementRef::wrap(sibling) else {
            continue;
        };
        if is_chapter_marker(&element) {
            break;
        }
        match element.value().name() {
            "h5" => {
                heading = normalized(&element.text().collect::<String>());
            }
            "p" => {
                flush(
                    &mut buffer,
                    current,
                    &mut content.verses,
                    &mut heading,
                );
                collect_paragraph(
                    element,
                    &mut current,
                    &mut buffer,
                    &mut content,
                    &mut heading,
                    &mut note_index,
                    context,
                );
                flush(
                    &mut buffer,
                    current,
                    &mut content.verses,
                    &mut heading,
                );
            }
            _ => {}
        }
    }
    flush(
        &mut buffer,
        current,
        &mut content.verses,
        &mut heading,
    );
    content
}

fn collect_paragraph(
    paragraph: ElementRef<'_>,
    current: &mut Option<u32>,
    buffer: &mut String,
    content: &mut ChapterContent,
    heading: &mut String,
    note_index: &mut usize,
    context: &NoteContext<'_>,
) {
    for child in paragraph.children() {
        match child.value() {
            Node::Text(text) => push_text(
                text,
                *current,
                buffer,
                content,
                heading,
                note_index,
                context,
            ),
            Node::Element(_) => {
                let Some(element) = ElementRef::wrap(child) else {
                    continue;
                };
                if element.value().classes().any(|class| class == "v-num") {
                    flush(buffer, *current, &mut content.verses, heading);
                    *current = verse_number(&element);
                    continue;
                }
                let text = element.text().collect::<String>();
                push_text(
                    &text,
                    *current,
                    buffer,
                    content,
                    heading,
                    note_index,
                    context,
                );
            }
            _ => {}
        }
    }
}

/// Add text to the current verse, lifting `[Note: …]` out of the flow.
fn push_text(
    text: &str,
    current: Option<u32>,
    buffer: &mut String,
    content: &mut ChapterContent,
    heading: &mut String,
    note_index: &mut usize,
    context: &NoteContext<'_>,
) {
    let mut rest = text;
    while let Some(start) = rest.find("[Note:") {
        buffer.push_str(&rest[..start]);
        let after = &rest[start + "[Note:".len()..];
        let Some(end) = after.find(']') else {
            buffer.push_str(&rest[start..]);
            return;
        };
        let note_text = normalized(&after[..end]);
        rest = &after[end + 1..];
        let Some(verse) = current else {
            continue;
        };
        flush(buffer, current, &mut content.verses, heading);
        let marker = note_marker(*note_index);
        *note_index += 1;
        let id = format!("{}:{}:{verse}-{marker}", context.book, context.chapter);
        content
            .verses
            .entry(verse)
            .or_insert_with(|| (std::mem::take(heading), Vec::new()))
            .1
            .push(ContentItem::new(
                "footnote",
                Some(marker),
                None,
                Some(id.clone()),
            ));
        content.footnotes.push(Footnote {
            id,
            text: note_text,
            references: BTreeMap::new(),
        });
    }
    buffer.push_str(rest);
}

fn flush(
    buffer: &mut String,
    current: Option<u32>,
    verses: &mut BTreeMap<u32, (String, Vec<ContentItem>)>,
    heading: &mut String,
) {
    let text = normalized(buffer);
    buffer.clear();
    let Some(number) = current else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let entry = verses
        .entry(number)
        .or_insert_with(|| (std::mem::take(heading), Vec::new()));
    entry.1.push(ContentItem::text("text", text));
}

fn parse_chapter(html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterData> {
    let document = Html::parse_document(html);
    let marker = chapter_marker(&document, chapter)
        .with_context(|| format!("Missing chapter marker for {}", chapter.passage()))?;
    let ChapterContent {
        verses: collected,
        footnotes,
    } = verses_in(&marker, &context(chapter));
    if collected.is_empty() {
        bail!("No verses found in {}", chapter.passage());
    }

    let mut data = ChapterData::new(
        &chapter.translation.code,
        &chapter.translation.name,
        &chapter.book.short,
        &chapter.book.name,
        chapter.chapter,
    );
    data.verses = collected
        .into_iter()
        .map(|(number, (heading, items))| Verse {
            chapter: chapter.chapter,
            verse: number,
            content: std::iter::once(ContentItem::new(
                "verse_number",
                None,
                Some(number),
                None,
            ))
            .chain(items)
            .collect(),
            header: if heading.is_empty() {
                Vec::new()
            } else {
                vec![ContentItem::text("text", heading)]
            },
            section: String::new(),
            paragraph_start: true,
            poetry_indent: None,
        })
        .collect();
    data.footnotes = footnotes;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn chapter_ref(chapter: u32) -> ChapterRef<'static> {
        let catalog: &'static Catalog = Box::leak(Box::new(
            Catalog::from_str(include_str!("../../catalog.toml")).unwrap(),
        ));
        ChapterRef {
            translation: Box::leak(Box::new(catalog.resolve("en", "OEB").unwrap())),
            book: catalog.book("LUK").unwrap(),
            chapter,
        }
    }

    fn fixture() -> String {
        r#"<html><body>
<h1 id="041"></h1><h1>Mark</h1><p class="c-num">[Mark 16]</p><p><span class="v-num">[1]</span> End of Mark.</p>
<h1 id="042"></h1><h1>Luke</h1>
<p class="c-num">[Luke 1]</p>
<h5>Dedication</h5>
<p> <span class="v-num">[1]</span>  To his Excellency, Theophilus. </p>
<p>  Many attempts have been made,  <span class="v-num">[2]</span>  just as they were reported to us. </p>
<p class="c-num">[Luke 2]</p>
<p> <span class="v-num">[1]</span> In those days a decree went out. </p>
</body></html>"#
            .to_string()
    }

    #[test]
    fn slices_one_chapter_out_of_the_whole_document() {
        let chapter = chapter_ref(1);
        let html = fixture();
        assert!(Oeb.validate(&html, &chapter).is_ok());
        assert_eq!(Oeb.verse_count(&html, &chapter), 2);

        let ChapterBody::Standard(data) = Oeb.parse(&html, &chapter).unwrap() else {
            panic!("expected standard")
        };
        assert_eq!(data.verses.len(), 2);
        assert_eq!(data.verses[0].header[0].text.as_deref(), Some("Dedication"));
        assert_eq!(
            crate::model::render_text_chapter(&data),
            // Text before a [2] marker still belongs to verse 1.
            "1 To his Excellency, Theophilus. Many attempts have been made,\n2 just as they were reported to us.\n"
        );
    }

    #[test]
    fn stops_at_the_next_chapter() {
        let chapter = chapter_ref(1);
        let ChapterBody::Standard(data) = Oeb.parse(&fixture(), &chapter).unwrap() else {
            panic!()
        };
        assert!(!data.verses.iter().any(|v| v.content.iter().any(|c| c
            .text
            .as_deref()
            .is_some_and(|t| t.contains("decree")))));
    }

    #[test]
    fn rejects_a_missing_chapter() {
        let chapter = chapter_ref(9);
        assert!(Oeb.validate(&fixture(), &chapter).is_err());
    }
}
