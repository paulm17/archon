use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use scraper::{Html, Node, Selector};

use crate::model::{ChapterBody, ChapterData, ContentItem, Verse, normalized};
use crate::sites::{ChapterRef, Site};

pub struct SacredBible;

impl Site for SacredBible {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        self.parse(html, chapter).map(|_| ())
    }

    fn verse_count(&self, html: &str, chapter: &ChapterRef<'_>) -> usize {
        self.parse(html, chapter).map_or(0, |body| body.verses())
    }

    fn parse(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterBody> {
        let mut chapters = book_chapters(html, chapter)?;
        chapters
            .remove(&chapter.chapter)
            .map(ChapterBody::Standard)
            .with_context(|| format!("Missing CPDV chapter {}", chapter.passage()))
    }
}

/// Extract all chapters from SacredBible's Original Edition book page.
pub fn book_chapters(html: &str, reference: &ChapterRef<'_>) -> Result<BTreeMap<u32, ChapterData>> {
    let document = Html::parse_document(html);
    let title = Selector::parse("title").unwrap();
    let book_links = Selector::parse("a.title[href]").unwrap();
    let filename = format!("{}.htm", reference.translation.book_slug(reference.book)?);
    ensure!(
        document.select(&title).any(|element| element
            .text()
            .collect::<String>()
            .trim()
            .ends_with(" - Catholic Public Domain Version of the Sacred Bible"))
            && document
                .select(&book_links)
                .any(|element| element.attr("href") == Some(&filename)),
        "Unexpected CPDV page for {}",
        reference.book.name
    );

    let marker = Selector::parse("a.chapter").unwrap();
    let first_chapter = document
        .select(&marker)
        .next()
        .context("Missing CPDV chapter marker")?;
    let body = first_chapter
        .ancestors()
        .find(|node| {
            node.value()
                .as_element()
                .is_some_and(|element| element.name() == "font")
        })
        .context("Missing CPDV scripture text")?;

    let mut chapters = BTreeMap::new();
    let mut line = String::new();
    for node in body.descendants() {
        match node.value() {
            Node::Text(text) => line.push_str(text),
            Node::Element(element) if element.name() == "br" => {
                add_verse(&line, reference, &mut chapters)?;
                line.clear();
            }
            _ => {}
        }
    }
    add_verse(&line, reference, &mut chapters)?;
    ensure!(
        !chapters.is_empty(),
        "No CPDV verses found for {}",
        reference.book.name
    );
    Ok(chapters)
}

fn add_verse(
    line: &str,
    reference: &ChapterRef<'_>,
    chapters: &mut BTreeMap<u32, ChapterData>,
) -> Result<()> {
    let Some((label, text)) = line
        .trim_start()
        .strip_prefix('{')
        .and_then(|line| line.split_once('}'))
    else {
        return Ok(());
    };
    let (chapter, verse) = label.split_once(':').context("Invalid CPDV verse label")?;
    // Lamentations and Sirach label their prologues P rather than giving
    // them a numbered chapter.
    if chapter == "P" {
        return Ok(());
    }
    let chapter = chapter.parse::<u32>()?;
    let verse = verse.parse::<u32>()?;
    let text = normalized(text);
    ensure!(
        chapter > 0 && !text.is_empty(),
        "Invalid CPDV verse {{{label}}}"
    );

    let data = chapters.entry(chapter).or_insert_with(|| {
        ChapterData::new(
            &reference.translation.code,
            &reference.translation.name,
            &reference.book.short,
            &reference.book.name,
            chapter,
        )
    });
    ensure!(
        data.verses
            .last()
            .is_none_or(|previous| previous.verse < verse),
        "Duplicate or out-of-order CPDV verse {{{label}}}"
    );
    data.verses.push(Verse {
        chapter,
        verse,
        content: vec![ContentItem::text("text", text)],
        header: Vec::new(),
        section: String::new(),
        paragraph_start: false,
        poetry_indent: None,
    });
    Ok(())
}
