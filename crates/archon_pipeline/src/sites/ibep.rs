//! IBEP web-component markup — one adapter for bible.bskorea.or.kr and
//! die-bibel.de, because the two differ only in host and translation id, both
//! of which live in the catalog.

use anyhow::{Context, Result, bail};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

use crate::model::{
    ChapterBody, ChapterData, ContentItem, CrossRef, CrossRefLink, Footnote, Verse, normalized,
};
use crate::sites::{ChapterRef, Site};

pub struct Ibep;

impl Site for Ibep {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        let verses = verse_numbers(html, chapter);
        if !html.contains("<html") || !html.contains("</html>") {
            bail!("page is not complete HTML");
        }
        if !html.contains("<ibep-render-bible-content") {
            bail!("missing IBEP content element");
        }
        let expected = format!(
            "id=\"{}.{}.1\"",
            chapter.translation.version,
            chapter.passage()
        );
        if !html.contains(&expected) {
            bail!("missing first verse id {expected}");
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

fn verse_numbers(html: &str, chapter: &ChapterRef<'_>) -> HashSet<u32> {
    let translation = &chapter.translation.version;
    let prefix = format!("{}.", chapter.passage());
    let first_id = format!("{translation}.{prefix}1");
    let document = Html::parse_document(html);
    let contents = Selector::parse("ibep-render-bible-content").unwrap();
    let verses = Selector::parse("[data-verse-org-id]").unwrap();
    let Some(content) = document.select(&contents).find(|content| {
        content
            .select(&verses)
            .any(|verse| verse.value().attr("id") == Some(first_id.as_str()))
    }) else {
        return HashSet::new();
    };
    content
        .select(&verses)
        .filter_map(|verse| {
            let reference = verse.value().attr("data-verse-org-id")?;
            let expected_id = format!("{translation}.{reference}");
            if verse.value().attr("id") != Some(expected_id.as_str()) {
                return None;
            }
            reference
                .strip_prefix(&prefix)?
                .parse::<u32>()
                .ok()
                .filter(|number| *number > 0)
        })
        .collect()
}

fn parse_verse_number(id: &str, prefix: &str) -> Option<u32> {
    id.strip_prefix(prefix)
        .and_then(|tail| tail.strip_prefix('.'))
        .map(|tail| tail.split('+').next().unwrap_or(tail))
        .and_then(|tail| tail.parse::<u32>().ok())
        .filter(|number| *number > 0)
}

fn text_of(element: ElementRef<'_>) -> String {
    normalized(&element.text().collect::<String>())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NoteKind {
    Footnote,
    CrossRef,
}

impl NoteKind {
    fn item_kind(self) -> &'static str {
        match self {
            Self::Footnote => "footnote",
            Self::CrossRef => "crossref",
        }
    }
}

/// The study notes a chapter page carries in its `IBEP-main-state` payload.
///
/// The rendered page leaves `ibep-bible-footnote` and
/// `ibep-bible-cross-reference` empty until Angular hydrates them, so the note
/// bodies only exist in that JSON. It does not record where inside a verse a
/// marker sat, so markers are appended to the verse they belong to.
#[derive(Default)]
struct StudyNotes {
    footnotes: Vec<Footnote>,
    cross_refs: Vec<CrossRef>,
    markers: BTreeMap<u32, Vec<ContentItem>>,
}

struct RawNote {
    kind: NoteKind,
    id: String,
    verse: u32,
    order: u32,
    marker: Option<String>,
    text: String,
    links: Vec<CrossRefLink>,
}

fn study_notes(document: &Html, chapter: &ChapterRef<'_>) -> StudyNotes {
    let mut notes = StudyNotes::default();
    let selector = Selector::parse("script#IBEP-main-state").unwrap();
    let Some(script) = document.select(&selector).next() else {
        return notes;
    };
    let Ok(state) = serde_json::from_str::<Value>(&script.text().collect::<String>()) else {
        return notes;
    };
    let Some(study) = chapter_study(&state, chapter) else {
        return notes;
    };

    let mut raw = Vec::new();
    for (section, kind) in [
        ("footNotes", NoteKind::Footnote),
        ("extendedFootnotes", NoteKind::Footnote),
        ("crossReferences", NoteKind::CrossRef),
    ] {
        let Some(entries) = study.get(section).and_then(Value::as_object) else {
            continue;
        };
        for (key, entry) in entries {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(key)
                .to_owned();
            let Some(verse) = entry
                .get("verseId")
                .and_then(Value::as_str)
                .and_then(verse_from_reference)
            else {
                continue;
            };
            let mut text = String::new();
            if let Some(content) = entry.get("content") {
                flatten_note(content, &mut text);
            }
            let mut links = Vec::new();
            if kind == NoteKind::CrossRef {
                if let Some(content) = entry.get("content") {
                    collect_links(content, &mut links);
                }
            }
            raw.push(RawNote {
                kind,
                order: note_order(&id),
                id,
                verse,
                marker: entry
                    .get("caller")
                    .and_then(Value::as_str)
                    .filter(|caller| !caller.is_empty() && !matches!(*caller, "+" | "-"))
                    .map(str::to_owned),
                text: normalized(&text),
                links,
            });
        }
    }
    raw.sort_by_key(|note| (note.verse, note.order));

    let mut counters: BTreeMap<(u32, &'static str), usize> = BTreeMap::new();
    for kind in [NoteKind::CrossRef, NoteKind::Footnote] {
        for note in raw.iter().filter(|note| note.kind == kind) {
            // A cross reference with no readable target is not a cross reference.
            if kind == NoteKind::CrossRef && note.links.is_empty() {
                continue;
            }
            let index = counters.entry((note.verse, kind.item_kind())).or_default();
            let marker = note
                .marker
                .clone()
                .unwrap_or_else(|| marker_for(kind, *index));
            *index += 1;
            notes
                .markers
                .entry(note.verse)
                .or_default()
                .push(ContentItem::new(
                    kind.item_kind(),
                    Some(marker),
                    None,
                    Some(note.id.clone()),
                ));
            match kind {
                NoteKind::Footnote => notes.footnotes.push(Footnote {
                    id: note.id.clone(),
                    text: note.text.clone(),
                    references: BTreeMap::new(),
                }),
                NoteKind::CrossRef => notes.cross_refs.push(CrossRef {
                    id: note.id.clone(),
                    book: chapter.book.name.clone(),
                    chapter: chapter.chapter,
                    verse: note.verse,
                    links: note.links.clone(),
                }),
            }
        }
    }
    notes
}

/// The `studyContent` block for this chapter, out of the page's request cache.
fn chapter_study<'a>(state: &'a Value, chapter: &ChapterRef<'_>) -> Option<&'a Value> {
    let expected = chapter.passage();
    let mut fallback = None;
    for value in state.as_object()?.values() {
        let Some(data) = value.get("data") else {
            continue;
        };
        let Some(study) = data.get("studyContent") else {
            continue;
        };
        if data.pointer("/chapter/id").and_then(Value::as_str) == Some(expected.as_str()) {
            return Some(study);
        }
        fallback.get_or_insert(study);
    }
    fallback
}

/// `MRK.11.9` → 9
fn verse_from_reference(reference: &str) -> Option<u32> {
    reference.rsplit('.').next()?.parse().ok()
}

/// `MRK.11.1-FN.10` → 10
fn note_order(id: &str) -> u32 {
    id.rsplit(['.', '-'])
        .next()
        .and_then(|tail| tail.parse().ok())
        .unwrap_or(0)
}

/// Flatten a note's content tree, skipping the verse labels (`fr`, `xo`).
fn flatten_note(node: &Value, out: &mut String) {
    match node {
        Value::String(text) => out.push_str(text),
        Value::Array(items) => {
            for item in items {
                flatten_note(item, out);
            }
        }
        Value::Object(map) => {
            let style = map.get("style").and_then(Value::as_str).unwrap_or_default();
            if matches!(style, "fr" | "xo") {
                return;
            }
            if let Some(text) = map.get("text").and_then(Value::as_str) {
                out.push_str(text);
                return;
            }
            if let Some(content) = map.get("content") {
                flatten_note(content, out);
            }
        }
        _ => {}
    }
}

fn collect_links(node: &Value, links: &mut Vec<CrossRefLink>) {
    match node {
        Value::Array(items) => {
            for item in items {
                collect_links(item, links);
            }
        }
        Value::Object(map) => {
            if let Some(link) = map
                .get("loc")
                .and_then(Value::as_str)
                .and_then(parse_loc)
            {
                links.push(link);
            }
            if let Some(content) = map.get("content") {
                collect_links(content, links);
            }
        }
        _ => {}
    }
}

/// `PSA 118:25-26` → Psalms 118, verses 25–26.
fn parse_loc(loc: &str) -> Option<CrossRefLink> {
    let (book, rest) = loc.trim().split_once(' ')?;
    let book = book.trim();
    if book.is_empty() {
        return None;
    }
    let (chapter, verses) = rest.split_once(':')?;
    let chapter: u32 = chapter.trim().parse().ok()?;
    let verses = verses.trim();
    if let Some((start, end)) = verses.split_once('-') {
        let start: u32 = start.trim().parse().ok()?;
        // A range that leaves the chapter keeps its first verse.
        if end.contains(':') {
            return Some(CrossRefLink {
                book: book.into(),
                chapter,
                verse: Some(start),
                verse_start: None,
                verse_end: None,
            });
        }
        return Some(CrossRefLink {
            book: book.into(),
            chapter,
            verse: None,
            verse_start: Some(start),
            verse_end: Some(end.trim().parse().ok()?),
        });
    }
    Some(CrossRefLink {
        book: book.into(),
        chapter,
        verse: Some(verses.parse().ok()?),
        verse_start: None,
        verse_end: None,
    })
}

/// `0 → a`, `25 → z`, `26 → aa`; cross references are upper case.
fn marker_for(kind: NoteKind, mut index: usize) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push((b'a' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    let marker: String = letters.iter().rev().collect();
    match kind {
        NoteKind::Footnote => marker,
        NoteKind::CrossRef => marker.to_uppercase(),
    }
}

fn parse_chapter(html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterData> {
    let translation_code = chapter.translation.version.as_str();
    let expected_usfm = chapter.passage();
    let document = Html::parse_document(html);
    let chapter_selector = Selector::parse("ibep-bible-chapter").unwrap();
    let heading_selector = Selector::parse("h1").unwrap();
    let content_selector = Selector::parse("ibep-render-bible-content").unwrap();
    let verse_selector = Selector::parse("[data-verse-org-id]").unwrap();
    let expected_heading_suffix = format!(" {}", chapter.chapter);
    let first_verse_id = format!("{translation_code}.{expected_usfm}.1");

    let chapter_element = document
        .select(&chapter_selector)
        .find(|element| {
            element
                .select(&content_selector)
                .next()
                .is_some_and(|content| {
                    content
                        .select(&verse_selector)
                        .any(|verse| verse.value().attr("id") == Some(first_verse_id.as_str()))
                })
        })
        .with_context(|| {
            format!(
                "Missing {} chapter {expected_usfm}",
                chapter.translation.site
            )
        })?;

    let local_book_name = chapter_element
        .select(&heading_selector)
        .next()
        .map(text_of)
        .map(|heading| {
            heading
                .strip_suffix(&expected_heading_suffix)
                .unwrap_or(&heading)
                .to_owned()
        })
        .unwrap_or_else(|| chapter.book.name.clone());

    let content = chapter_element
        .select(&content_selector)
        .next()
        .with_context(|| format!("Missing {} Bible content", chapter.translation.site))?;
    let blocks = content
        .children()
        .filter_map(ElementRef::wrap)
        .find(|element| element.value().name() == "div")
        .with_context(|| {
            format!(
                "Missing {} passage blocks",
                chapter.translation.site
            )
        })?;

    let mut verses: BTreeMap<u32, Verse> = BTreeMap::new();
    let mut pending_headings = Vec::new();

    for node in blocks.children() {
        let Some(block) = ElementRef::wrap(node) else {
            continue;
        };
        let tag = block.value().name();
        if tag == "bible-s" || tag == "bible-ms" {
            let heading = text_of(block);
            if !heading.is_empty() {
                pending_headings.push(heading);
            }
            continue;
        }
        if tag == "bible-r" {
            continue;
        }
        let poetry_indent = (tag == "bible-q").then_some(1);
        let mut grouped: BTreeMap<u32, String> = BTreeMap::new();
        for span in block.select(&verse_selector) {
            if !span
                .value()
                .attr("id")
                .is_some_and(|id| id.starts_with(&format!("{translation_code}.")))
            {
                continue;
            }
            let Some(number) = span
                .value()
                .attr("data-verse-org-id")
                .and_then(|id| parse_verse_number(id, &expected_usfm))
            else {
                continue;
            };
            if span.value().attr("id")
                != Some(format!("{translation_code}.{expected_usfm}.{number}").as_str())
            {
                continue;
            }
            grouped
                .entry(number)
                .or_default()
                .push_str(&span.text().collect::<String>());
        }

        let mut first_new_in_block = true;
        for (number, raw_text) in grouped {
            let verse_text = normalized(&raw_text);
            if verse_text.is_empty() {
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
                    content: vec![ContentItem::new("verse_number", None, Some(number), None)],
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
                verse
                    .content
                    .push(ContentItem::new("poetry_line", None, Some(indent), None));
            }
            verse.content.push(ContentItem::text("text", verse_text));
        }
    }

    if !verses.contains_key(&1) || verses.len() < 2 {
        bail!("Incomplete {} chapter {expected_usfm}", chapter.translation.site);
    }

    let StudyNotes {
        footnotes,
        cross_refs,
        markers,
    } = study_notes(&document, chapter);
    for (number, items) in markers {
        if let Some(verse) = verses.get_mut(&number) {
            verse.content.extend(items);
        }
    }

    let mut data = ChapterData::new(
        translation_code,
        &chapter.translation.name,
        &chapter.book.short,
        &local_book_name,
        chapter.chapter,
    );
    data.verses = verses.into_values().collect();
    data.footnotes = footnotes;
    data.cross_refs = cross_refs;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn catalog() -> Catalog {
        Catalog::from_str(include_str!("../../catalog.toml")).unwrap()
    }

    #[test]
    fn parses_korean_split_verses_and_headings() {
        let catalog = catalog();
        let translation = catalog.resolve("kr", "NKT").unwrap();
        let book = catalog.book("MRK").unwrap();
        let html = r#"<html><ibep-bible-chapter><h1>마가복음 1</h1><ibep-render-bible-content><div><bible-s><h4>세례자 요한이 선포하다</h4></bible-s><bible-r><h5>(마 3:1-12)</h5></bible-r><bible-p><span id="NKT.MRK.1.1" data-verse-org-id="MRK.1.1">예수 </span><span id="NKT.MRK.1.1" data-verse-org-id="MRK.1.1">그리스도</span><span id="NKT.MRK.1.2" data-verse-org-id="MRK.1.2">복음의 시작.</span></bible-p><bible-q><span id="NKT.MRK.1.2" data-verse-org-id="MRK.1.2"> “좋은 소식”</span></bible-q></div></ibep-render-bible-content></ibep-bible-chapter></html>"#;
        let chapter = ChapterRef { translation: &translation, book, chapter: 1 };
        assert!(Ibep.validate(html, &chapter).is_ok());
        let ChapterBody::Standard(data) = Ibep.parse(html, &chapter).unwrap() else { panic!() };
        assert_eq!(data.translation_code, "NKT");
        assert_eq!(data.book_name, "마가복음");
        assert_eq!(data.verses.len(), 2);
        assert_eq!(data.verses[0].header[0].text.as_deref(), Some("세례자 요한이 선포하다"));
        assert_eq!(data.verses[1].poetry_indent, Some(1));
        assert_eq!(
            crate::model::render_text_chapter(&data),
            "1 예수 그리스도\n2 복음의 시작. “좋은 소식”\n"
        );
    }

    #[test]
    fn parses_german_and_rejects_the_wrong_chapter() {
        let catalog = catalog();
        let translation = catalog.resolve("de", "BB").unwrap();
        let book = catalog.book("JHN").unwrap();
        let html = r#"<html><ibep-bible-chapter><h1>Johannes 1</h1><ibep-render-bible-content><div><bible-s><h4>Das Wort Gottes</h4></bible-s><bible-m><p><span id="BB.JHN.1.1" data-verse-org-id="JHN.1.1">Von Anfang an gab es das Wort</span></p></bible-m><bible-m><p><span id="BB.JHN.1.2" data-verse-org-id="JHN.1.2">Dieses Wort gehörte zu Gott.</span></p></bible-m></div></ibep-render-bible-content></ibep-bible-chapter></html>"#;
        let chapter = ChapterRef { translation: &translation, book, chapter: 1 };
        let ChapterBody::Standard(data) = Ibep.parse(html, &chapter).unwrap() else { panic!() };
        assert_eq!(data.translation_name, "BasisBibel");
        assert_eq!(data.book_name, "Johannes");
        assert_eq!(
            crate::model::render_text_chapter(&data),
            "1 Von Anfang an gab es das Wort\n2 Dieses Wort gehörte zu Gott.\n"
        );
        assert!(Ibep.validate(html, &ChapterRef { translation: &translation, book, chapter: 2 }).is_err());
    }

    #[test]
    fn reads_notes_from_the_page_state() {
        let catalog = catalog();
        let translation = catalog.resolve("de", "BB").unwrap();
        let book = catalog.book("MRK").unwrap();
        let html = r##"<html><ibep-bible-chapter><h1>Markus 1</h1><ibep-render-bible-content><div>
<bible-p><span id="BB.MRK.1.1" data-verse-org-id="MRK.1.1">Anfang des Evangeliums.</span></bible-p>
<bible-p><span id="BB.MRK.1.2" data-verse-org-id="MRK.1.2">Wie geschrieben steht.</span></bible-p>
</div></ibep-render-bible-content></ibep-bible-chapter>
<script id="IBEP-main-state" type="application/json">{"https://api.ibep-prod.com/bibles/x/chapters/MRK.1/with-study-content":{"data":{"chapter":{"id":"MRK.1"},"studyContent":{"footNotes":{"MRK.1.1-FN.1":{"type":"foot-note","style":"f","caller":"+","content":[{"type":"char","content":"1,1 ","style":"fr"},{"type":"char","content":"Ein Hinweis.","style":"ft"}],"verseId":"MRK.1.1","id":"MRK.1.1-FN.1"}},"crossReferences":{"MRK.1.2-CR.1":{"type":"note","style":"x","caller":"-","content":[{"type":"char","content":"1:2 ","style":"xo"},{"type":"char","style":"xt","content":[{"type":"ref","text":"Jes 40,3","loc":"ISA 40:3"},{"type":"ref","text":"Mal 3,1","loc":"MAL 3:1"}]}],"verseId":"MRK.1.2","id":"MRK.1.2-CR.1"}}}}}}</script></html>"##;
        let chapter = ChapterRef { translation: &translation, book, chapter: 1 };
        let ChapterBody::Standard(data) = Ibep.parse(html, &chapter).unwrap() else { panic!() };

        assert_eq!(data.footnotes.len(), 1);
        assert_eq!(data.footnotes[0].id, "MRK.1.1-FN.1");
        // The `fr` verse label is not part of the note.
        assert_eq!(data.footnotes[0].text, "Ein Hinweis.");

        assert_eq!(data.cross_refs.len(), 1);
        let cross_ref = &data.cross_refs[0];
        assert_eq!(cross_ref.id, "MRK.1.2-CR.1");
        assert_eq!((cross_ref.chapter, cross_ref.verse), (1, 2));
        assert_eq!(cross_ref.links.len(), 2);
        assert_eq!(
            (
                cross_ref.links[0].book.as_str(),
                cross_ref.links[0].chapter,
                cross_ref.links[0].verse
            ),
            ("ISA", 40, Some(3))
        );

        // Markers are appended to the verse they belong to.
        assert!(data.verses[0].content.iter().any(|item| item.kind == "footnote"
            && item.text.as_deref() == Some("a")
            && item.id.as_deref() == Some("MRK.1.1-FN.1")));
        assert!(data.verses[1].content.iter().any(|item| item.kind == "crossref"
            && item.text.as_deref() == Some("A")
            && item.id.as_deref() == Some("MRK.1.2-CR.1")));
        assert_eq!(
            crate::model::render_text_chapter(&data),
            "1 Anfang des Evangeliums.\n2 Wie geschrieben steht.\n"
        );
    }

    #[test]
    fn refuses_verses_from_a_different_version() {
        let catalog = catalog();
        let translation = catalog.resolve("de", "BB").unwrap();
        let book = catalog.book("2JN").unwrap();
        let html = r#"<html><ibep-render-bible-content><span id="BB.2JN.1.1" data-verse-org-id="2JN.1.1">Eins</span><span id="NKT.2JN.1.2" data-verse-org-id="2JN.1.2">Zwei</span></ibep-render-bible-content></html>"#;
        let chapter = ChapterRef { translation: &translation, book, chapter: 1 };
        assert_eq!(Ibep.verse_count(html, &chapter), 1);
        assert!(Ibep.validate(html, &chapter).is_err());
    }
}
