//! BibleGateway — `div.passage-text` with `span.text {Book}-{Chapter}-{Verse}`.
//!
//! Also serves the Greek and Hebrew translations: same markup, different
//! version id from the catalog.
//!
//! Footnotes and cross references are two-part: an inline marker inside the
//! verse text (`sup.footnote`, `sup.crossreference`) and the note itself in a
//! chapter-level list (`div.footnotes`, `div.crossrefs`). Both parts are joined
//! by the note id, which is why the marker carries the id.

use anyhow::{Context, Result, bail};
use scraper::{ElementRef, Html, Node, Selector};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::model::{
    ChapterBody, ChapterData, ContentItem, CrossRef, CrossRefLink, Footnote, Verse, normalized,
};
use crate::sites::{ChapterRef, Site};

pub struct BibleGateway;

impl Site for BibleGateway {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        let verses = verse_numbers(html, chapter);
        if !html.contains("<html") || !html.contains("</html>") {
            bail!("page is not complete HTML");
        }
        if !html.contains("passage-text") {
            bail!("missing passage container");
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

fn verse_from_classes(element: &ElementRef<'_>, short: &str, chapter: u32) -> Option<u32> {
    let prefix = format!("{short}-{chapter}-");
    element
        .value()
        .classes()
        .find_map(|class| class.strip_prefix(&prefix)?.parse::<u32>().ok())
        .filter(|number| *number > 0)
}

/// Classes that carry no verse text.
fn is_skipped(element: &ElementRef<'_>) -> bool {
    element.value().classes().any(|class| {
        matches!(
            class,
            "versenum"
                | "chapternum"
                | "footnote"
                | "crossreference"
                | "footnote-text"
                | "crossreference-text"
        )
    })
}

fn class_is(element: &ElementRef<'_>, name: &str) -> bool {
    element.value().classes().any(|class| class == name)
}

fn flush_text(buffer: &mut String, content: &mut Vec<ContentItem>) {
    let text = normalized(buffer);
    if !text.is_empty() {
        content.push(ContentItem::text("text", text));
    }
    buffer.clear();
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

/// A note marker found in the verse text.
struct Note {
    kind: NoteKind,
    /// The verse the marker annotates.
    verse: u32,
    /// BibleGateway's sequential verse number, taken from the note id.
    global: u32,
    marker: String,
}

impl Note {
    /// The id shared by the inline marker and the note entry.
    ///
    /// Footnotes keep the sequential verse number the site uses in its note
    /// ids, cross references use the verse of the chapter.
    fn id(&self, context: &NoteContext<'_>) -> String {
        let number = match self.kind {
            NoteKind::Footnote => self.global,
            NoteKind::CrossRef => self.verse,
        };
        format!(
            "{}:{}:{number}-{}",
            context.book, context.chapter, self.marker
        )
    }
}

struct NoteContext<'a> {
    /// The short book code the site uses (`Gen`), which is also what the
    /// existing corpora put in note ids.
    book: &'a str,
    chapter: u32,
}

/// `sup.footnote` / `sup.crossreference` → the note it links to.
fn note_ref(element: &ElementRef<'_>, verse: u32) -> Option<(String, Note)> {
    let kind = if class_is(element, "footnote") {
        NoteKind::Footnote
    } else if class_is(element, "crossreference") {
        NoteKind::CrossRef
    } else {
        return None;
    };
    let anchor = Selector::parse("a[href]").ok()?;
    let href = element.select(&anchor).next()?.value().attr("href")?;
    let id = href.trim_start_matches('#');
    // `fen-NIV-1366a` → sequential verse 1366, marker `a`.
    let tail = id.rsplit('-').next()?;
    let split = tail.find(|character: char| !character.is_ascii_digit())?;
    let global = tail[..split].parse().ok()?;
    let marker = &tail[split..];
    if id.is_empty() || marker.is_empty() {
        return None;
    }
    Some((
        id.to_owned(),
        Note {
            kind,
            verse,
            global,
            marker: marker.to_owned(),
        },
    ))
}

fn walk_verse_content(
    element: ElementRef<'_>,
    buffer: &mut String,
    content: &mut Vec<ContentItem>,
    verse: Option<u32>,
    notes: &mut HashMap<String, Note>,
    context: &NoteContext<'_>,
) {
    for child in element.children() {
        match child.value() {
            Node::Text(text) => buffer.push_str(text),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(child) else {
                    continue;
                };
                if let Some(verse) = verse {
                    if let Some((id, note)) = note_ref(&child, verse) {
                        flush_text(buffer, content);
                        content.push(ContentItem::new(
                            note.kind.item_kind(),
                            Some(note.marker.clone()),
                            None,
                            Some(note.id(context)),
                        ));
                        notes.insert(id, note);
                        continue;
                    }
                }
                if is_skipped(&child) {
                    continue;
                }
                walk_verse_content(child, buffer, content, verse, notes, context);
            }
            _ => {}
        }
    }
}

fn collect_footnotes(
    document: &Html,
    notes: &HashMap<String, Note>,
    context: &NoteContext<'_>,
) -> Vec<Footnote> {
    let selector = Selector::parse("div.footnotes li[id]").unwrap();
    let text_selector = Selector::parse("span.footnote-text").unwrap();
    let mut footnotes = Vec::new();
    for li in document.select(&selector) {
        let Some(id) = li.value().attr("id") else {
            continue;
        };
        let Some(note) = notes.get(id) else {
            continue;
        };
        if note.kind != NoteKind::Footnote {
            continue;
        }
        footnotes.push(Footnote {
            id: note.id(context),
            text: li
                .select(&text_selector)
                .next()
                .map(text_of)
                .unwrap_or_default(),
            references: BTreeMap::new(),
        });
    }
    footnotes
}

fn collect_cross_refs(
    document: &Html,
    notes: &HashMap<String, Note>,
    context: &NoteContext<'_>,
    book_name: &str,
) -> Vec<CrossRef> {
    let selector = Selector::parse("div.crossrefs li[id]").unwrap();
    let link_selector = Selector::parse("a.crossref-link").unwrap();
    let mut cross_refs = Vec::new();
    for li in document.select(&selector) {
        let Some(id) = li.value().attr("id") else {
            continue;
        };
        let Some(note) = notes.get(id) else {
            continue;
        };
        if note.kind != NoteKind::CrossRef {
            continue;
        }
        let mut links = Vec::new();
        for link in li.select(&link_selector) {
            if let Some(bibleref) = link.value().attr("data-bibleref") {
                parse_bibleref(bibleref, &mut links);
            }
        }
        if links.is_empty() {
            continue;
        }
        cross_refs.push(CrossRef {
            id: note.id(context),
            book: book_name.to_owned(),
            chapter: context.chapter,
            verse: note.verse,
            links,
        });
    }
    cross_refs
}

/// `data-bibleref` is a comma-separated reference list, usually expanded to
/// full book names: `Psalm 102:25, Isaiah 41:4, John 1:1-John 1:2`.
fn parse_bibleref(text: &str, links: &mut Vec<CrossRefLink>) {
    let mut book = None;
    let mut chapter = None;
    for part in text.split(',') {
        parse_reference(part.trim(), &mut book, &mut chapter, links);
    }
}

fn parse_reference(
    part: &str,
    book: &mut Option<String>,
    chapter: &mut Option<u32>,
    links: &mut Vec<CrossRefLink>,
) {
    if part.is_empty() {
        return;
    }
    // `Mark 6:49-Mark 6:50` names both ends of one range.
    if let Some((left, right)) = part.rsplit_once('-') {
        if right.contains(' ') && right.contains(':') {
            if let (Some((book1, chapter1, verse1)), Some((book2, chapter2, verse2))) =
                (split_citation(left), split_citation(right))
            {
                *book = Some(book1.clone());
                *chapter = Some(chapter1);
                if book1 == book2 && chapter1 == chapter2 {
                    links.push(CrossRefLink {
                        book: book1,
                        chapter: chapter1,
                        verse: None,
                        verse_start: Some(verse1),
                        verse_end: Some(verse2),
                    });
                } else {
                    // A range that leaves the chapter keeps its first verse.
                    links.push(CrossRefLink {
                        book: book1,
                        chapter: chapter1,
                        verse: Some(verse1),
                        verse_start: None,
                        verse_end: None,
                    });
                }
                return;
            }
        }
    }

    match part.split_once(':') {
        Some((reference, verses)) => {
            let Some((book, chapter)) = split_book_chapter(reference, book, chapter) else {
                return;
            };
            let Some((start, end)) = parse_verse_range(verses) else {
                return;
            };
            links.push(CrossRefLink {
                book,
                chapter,
                verse: if end.is_none() { Some(start) } else { None },
                verse_start: if end.is_some() { Some(start) } else { None },
                verse_end: end,
            });
        }
        None => match part.rsplit_once(' ') {
            // `Genesis 2` — a whole chapter.
            Some((name, number)) => {
                let name = name.trim();
                let Ok(chapter_number) = number.trim().parse::<u32>() else {
                    return;
                };
                if name.is_empty() {
                    return;
                }
                *book = Some(canonical_book(name).to_owned());
                *chapter = Some(chapter_number);
                links.push(CrossRefLink {
                    book: canonical_book(name).to_owned(),
                    chapter: chapter_number,
                    verse: None,
                    verse_start: None,
                    verse_end: None,
                });
            }
            // `26` — a verse in the chapter carried over.
            None => {
                let Ok(verse) = part.parse::<u32>() else {
                    return;
                };
                let (Some(book), Some(chapter)) = (book.clone(), *chapter) else {
                    return;
                };
                links.push(CrossRefLink {
                    book,
                    chapter,
                    verse: Some(verse),
                    verse_start: None,
                    verse_end: None,
                });
            }
        },
    }
}

/// `Book 1:2` → book, chapter and verse.
fn split_citation(citation: &str) -> Option<(String, u32, u32)> {
    let (reference, verses) = citation.split_once(':')?;
    let (book, chapter) = reference.rsplit_once(' ')?;
    let book = book.trim();
    if book.is_empty() {
        return None;
    }
    Some((
        canonical_book(book).to_owned(),
        chapter.trim().parse().ok()?,
        verses.trim().parse().ok()?,
    ))
}

/// The publisher spells two books differently from the catalog, which is the
/// spelling every other site and the existing corpora use. Localized names are
/// left as the page gives them.
fn canonical_book(name: &str) -> &str {
    match name {
        "Psalm" => "Psalms",
        "Song of Songs" => "Song of Solomon",
        other => other,
    }
}

/// `Book 1` or `1` → the book and chapter, carrying both forward.
fn split_book_chapter(
    reference: &str,
    book: &mut Option<String>,
    chapter: &mut Option<u32>,
) -> Option<(String, u32)> {
    match reference.rsplit_once(' ') {
        Some((name, number)) => {
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            *book = Some(canonical_book(name).to_owned());
            *chapter = Some(number.trim().parse().ok()?);
        }
        None => {
            *chapter = Some(reference.trim().parse().ok()?);
        }
    }
    Some((book.clone()?, (*chapter)?))
}

fn parse_verse_range(verses: &str) -> Option<(u32, Option<u32>)> {
    match verses.split_once('-') {
        Some((start, end)) => Some((start.trim().parse().ok()?, Some(end.trim().parse().ok()?))),
        None => Some((verses.trim().parse().ok()?, None)),
    }
}

fn collect_text(element: ElementRef<'_>, buffer: &mut String) {
    for child in element.children() {
        match child.value() {
            scraper::Node::Text(text) => buffer.push_str(text),
            scraper::Node::Element(_) => {
                let Some(child) = ElementRef::wrap(child) else {
                    continue;
                };
                if is_skipped(&child) {
                    continue;
                }
                collect_text(child, buffer);
            }
            _ => {}
        }
    }
}

fn text_of(element: ElementRef<'_>) -> String {
    let mut buffer = String::new();
    collect_text(element, &mut buffer);
    normalized(&buffer)
}

fn passage_root(document: &Html) -> Option<ElementRef<'_>> {
    let selector = Selector::parse("div.passage-text").ok()?;
    document.select(&selector).next()
}

fn verse_numbers(html: &str, chapter: &ChapterRef<'_>) -> HashSet<u32> {
    let document = Html::parse_document(html);
    let Some(root) = passage_root(&document) else {
        return HashSet::new();
    };
    let spans = Selector::parse("span.text").unwrap();
    let headings = Selector::parse("h3, h4").unwrap();
    let heading_ids: HashSet<_> = root
        .select(&headings)
        .flat_map(|heading| heading.select(&spans).collect::<Vec<_>>())
        .map(|span| span.id())
        .collect();
    root.select(&spans)
        .filter(|span| !heading_ids.contains(&span.id()))
        .filter_map(|span| verse_from_classes(&span, &chapter.book.short, chapter.chapter))
        .collect()
}

/// Poetry lines are grouped under a `div.poetry`, one `span.text` per line.
fn in_poetry(span: &ElementRef<'_>) -> bool {
    span.ancestors().filter_map(ElementRef::wrap).any(|ancestor| {
        ancestor.value().name() == "div"
            && ancestor.value().classes().any(|class| class == "poetry")
    })
}

/// The `Footnotes` / `Cross references` labels are `h4`s inside the note
/// panels, not passage headings.
fn in_notes_panel(element: &ElementRef<'_>) -> bool {
    element
        .ancestors()
        .filter_map(ElementRef::wrap)
        .any(|ancestor| {
            ancestor
                .value()
                .classes()
                .any(|class| matches!(class, "footnotes" | "crossrefs" | "passage-utils"))
        })
}

/// The indent of a poetry line, from the `span.indent-N` around its text.
fn poetry_line_indent(span: &ElementRef<'_>) -> Option<u32> {
    for ancestor in span.ancestors().filter_map(ElementRef::wrap) {
        for class in ancestor.value().classes() {
            for level in 1..=4u32 {
                if class == format!("indent-{level}") {
                    return Some(level);
                }
            }
        }
    }
    None
}

fn poetry_indent(span: &ElementRef<'_>) -> Option<u32> {
    for ancestor in span.ancestors().filter_map(ElementRef::wrap) {
        if ancestor.value().name() != "p" && ancestor.value().name() != "div" {
            continue;
        }
        for class in ancestor.value().classes() {
            for level in 1..=4u32 {
                if class == format!("line-{level}") || class == format!("indent-{level}") {
                    return Some(level);
                }
            }
        }
    }
    None
}

fn parse_chapter(html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterData> {
    let document = Html::parse_document(html);
    let root = passage_root(&document)
        .with_context(|| format!("Missing passage container for {}", chapter.passage()))?;
    let spans = Selector::parse("span.text").unwrap();
    let headings = Selector::parse("h3, h4").unwrap();

    let mut heading_ids = HashSet::new();
    let mut pending_headings: Vec<ContentItem> = Vec::new();
    let mut notes: HashMap<String, Note> = HashMap::new();
    let note_context = NoteContext {
        book: &chapter.book.short,
        chapter: chapter.chapter,
    };
    for heading in root.select(&headings) {
        if in_notes_panel(&heading) {
            continue;
        }
        for span in heading.select(&spans) {
            heading_ids.insert(span.id());
        }
        // A heading belongs to the verse its first span names, and may carry
        // its own cross-reference markers.
        let verse = heading
            .select(&spans)
            .find_map(|span| verse_from_classes(&span, &chapter.book.short, chapter.chapter));
        let mut buffer = String::new();
        walk_verse_content(
            heading,
            &mut buffer,
            &mut pending_headings,
            verse,
            &mut notes,
            &note_context,
        );
        flush_text(&mut buffer, &mut pending_headings);
    }

    let mut verses: BTreeMap<u32, Verse> = BTreeMap::new();
    let mut current_block = None;

    for span in root.select(&spans) {
        if heading_ids.contains(&span.id()) {
            continue;
        }
        let Some(number) = verse_from_classes(&span, &chapter.book.short, chapter.chapter) else {
            continue;
        };
        let block = span
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|element| element.value().name() == "p")
            .map(|element| element.id());
        let paragraph_start = block.is_some() && block != current_block;
        if paragraph_start {
            current_block = block;
        }
        let indent = poetry_indent(&span);
        let mut content = Vec::new();
        let mut buffer = String::new();
        walk_verse_content(
            span,
            &mut buffer,
            &mut content,
            Some(number),
            &mut notes,
            &note_context,
        );
        flush_text(&mut buffer, &mut content);
        if content.is_empty() {
            continue;
        }
        // One marker per poetry line, before its text, carrying the line's
        // indent — the corpus has `number: null` for the unindented first line.
        if in_poetry(&span) {
            content.insert(
                0,
                ContentItem::new("poetry_line", None, poetry_line_indent(&span), None),
            );
        }

        let verse = verses.entry(number).or_insert_with(|| {
            let header = std::mem::take(&mut pending_headings);
            Verse {
                chapter: chapter.chapter,
                verse: number,
                content: vec![ContentItem::new("verse_number", None, Some(number), None)],
                header,
                section: String::new(),
                paragraph_start,
                poetry_indent: indent,
            }
        });
        if indent.is_some() && verse.poetry_indent.is_none() {
            verse.poetry_indent = indent;
        }
        verse.content.extend(content);
    }

    if verses.is_empty() {
        bail!("No verses found in {}", chapter.passage());
    }
    if !verses.contains_key(&1) {
        bail!("First verse missing from {}", chapter.passage());
    }

    let mut data = ChapterData::new(
        &chapter.translation.code,
        &chapter.translation.name,
        &chapter.book.short,
        &chapter.book.name,
        chapter.chapter,
    );
    data.verses = verses.into_values().collect();
    data.footnotes = collect_footnotes(&document, &notes, &note_context);
    data.cross_refs = collect_cross_refs(&document, &notes, &note_context, &chapter.book.name);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn setup(language: &str, code: &str, usfm: &str) -> ChapterRef<'static> {
        let catalog: &'static Catalog = Box::leak(Box::new(
            Catalog::from_str(include_str!("../../catalog.toml")).unwrap(),
        ));
        ChapterRef {
            translation: Box::leak(Box::new(catalog.resolve(language, code).unwrap())),
            book: catalog.book(usfm).unwrap(),
            chapter: 1,
        }
    }

    fn fixture(translation: &str, book: &str, chapter: u32) -> String {
        format!(
            r#"<html><div class="passage-text"><div class="passage-content"><div class="std-text">
<h3><span class="text {book}-{chapter}-1">The Beginning</span></h3>
<p class="hang-2 chapter-{chapter}"><span class="text {book}-{chapter}-1"><span class="chapternum">1&nbsp;</span>In the beginning<sup class="crossreference">(A)</sup> God created the heavens</span></p>
<p class="line-1"><span class="text {book}-{chapter}-2">And the earth was<span class="footnote">a</span> formless.</span></p>
</div></div><div class="version-{translation}"></div></div></html>"#
        )
    }

    #[test]
    fn parses_headings_verses_and_poetry() {
        let chapter = setup("en", "NIV", "GEN");
        let html = fixture("NIV", "Gen", 1);
        assert!(BibleGateway.validate(&html, &chapter).is_ok());
        assert_eq!(BibleGateway.verse_count(&html, &chapter), 2);

        let ChapterBody::Standard(data) = BibleGateway.parse(&html, &chapter).unwrap() else {
            panic!("expected standard")
        };
        assert_eq!(data.verses.len(), 2);
        assert_eq!(data.verses[0].header[0].text.as_deref(), Some("The Beginning"));
        assert_eq!(
            data.verses[0].content[1].text.as_deref(),
            Some("In the beginning God created the heavens")
        );
        assert_eq!(data.verses[1].poetry_indent, Some(1));
        assert_eq!(
            crate::model::render_text_chapter(&data),
            "1 In the beginning God created the heavens\n2 And the earth was formless.\n"
        );
    }

    fn notes_fixture() -> String {
        r##"<html><div class="passage-text"><div class="std-text">
<h3><span class="text Gen-45-1">Joseph<sup class="crossreference" data-cr="#cen-NIV-1360Z"><a href="#cen-NIV-1360Z">(Z)</a></sup> Reveals Himself</span></h3>
<p class="hang-2 chapter-45"><span class="text Gen-45-1"><span class="chapternum">1&nbsp;</span>Then Joseph<sup class="crossreference" data-cr="#cen-NIV-1360A"><a href="#cen-NIV-1360A">(A)</a></sup> could no longer control himself<sup class="footnote" data-link="[&lt;a href=&quot;#fen-NIV-1366a&quot;&gt;a&lt;/a&gt;]"><a href="#fen-NIV-1366a">[a]</a></sup> before all his attendants, and the <span class="small-caps">Lord</span> was with him.</span></p>
</div></div>
<div class="footnotes"><h4>Footnotes</h4><ol><li id="fen-NIV-1366a"><a href="#en-NIV-1366" title="Go to Genesis 45:7">Genesis 45:7</a> <span class="footnote-text">Or <i>save you as a great band of survivors</i></span></li></ol></div>
<div class="crossrefs hidden"><h4>Cross references</h4><ol><li id="cen-NIV-1360Z"><a href="#en-NIV-1360" title="Go to Genesis 45:1">Genesis 45:1</a> : <a class="crossref-link" data-bibleref="Genesis 45:4">Ge 45:4</a></li><li id="cen-NIV-1360A"><a href="#en-NIV-1360" title="Go to Genesis 45:1">Genesis 45:1</a> : <a class="crossref-link" href="/passage/?search=Genesis%2043" data-bibleref="Genesis 43:31, Genesis 24:4, Genesis 45:1-3, John 1:1-John 1:2">Ge 43:31; 24:4; Ge 45:1-3; Jn 1:1-2</a></li></ol></div>
</html>"##
            .to_string()
    }

    #[test]
    fn extracts_footnotes_and_cross_references() {
        let chapter = ChapterRef {
            chapter: 45,
            ..setup("en", "NIV", "GEN")
        };
        let ChapterBody::Standard(data) = BibleGateway.parse(&notes_fixture(), &chapter).unwrap()
        else {
            panic!("expected standard")
        };

        assert_eq!(data.footnotes.len(), 1);
        assert_eq!(data.footnotes[0].id, "Gen:45:1366-a");
        assert_eq!(
            data.footnotes[0].text,
            "Or save you as a great band of survivors"
        );
        assert!(data.footnotes[0].references.is_empty());

        // The heading's marker is a cross reference too, and belongs to the
        // verse its span names.
        assert_eq!(data.cross_refs.len(), 2);
        assert_eq!(data.cross_refs[0].id, "Gen:45:1-Z");
        assert_eq!(data.cross_refs[0].links[0].chapter, 45);
        assert_eq!(data.cross_refs[0].links[0].verse, Some(4));
        let header = &data.verses[0].header;
        // The note panels' own `h4` labels are not passage headings.
        assert!(
            !header.iter().any(|item| matches!(
                item.text.as_deref(),
                Some("Footnotes") | Some("Cross references")
            )),
            "note panel labels leaked into the header: {header:?}"
        );
        assert_eq!(header[0].text.as_deref(), Some("Joseph"));
        assert_eq!(header[1].kind, "crossref");
        assert_eq!(header[1].id.as_deref(), Some("Gen:45:1-Z"));
        assert_eq!(header[2].text.as_deref(), Some("Reveals Himself"));

        let cross_ref = &data.cross_refs[1];
        assert_eq!(cross_ref.id, "Gen:45:1-A");
        assert_eq!(cross_ref.book, "Genesis");
        assert_eq!((cross_ref.chapter, cross_ref.verse), (45, 1));
        assert_eq!(cross_ref.links.len(), 4);
        assert_eq!(
            (&cross_ref.links[0].book, cross_ref.links[0].chapter, cross_ref.links[0].verse),
            (&"Genesis".to_string(), 43, Some(31))
        );
        assert_eq!(
            (
                cross_ref.links[2].chapter,
                cross_ref.links[2].verse_start,
                cross_ref.links[2].verse_end
            ),
            (45, Some(1), Some(3))
        );
        // A range naming both ends is one range when it stays in a chapter.
        assert_eq!(
            (
                cross_ref.links[3].book.as_str(),
                cross_ref.links[3].chapter,
                cross_ref.links[3].verse_start,
                cross_ref.links[3].verse_end
            ),
            ("John", 1, Some(1), Some(2))
        );

        let content = &data.verses[0].content;
        assert!(content.iter().any(|item| item.kind == "crossref"
            && item.text.as_deref() == Some("A")
            && item.id.as_deref() == Some("Gen:45:1-A")));
        assert!(content.iter().any(|item| item.kind == "footnote"
            && item.text.as_deref() == Some("a")
            && item.id.as_deref() == Some("Gen:45:1366-a")));
        assert_eq!(
            crate::model::render_text_chapter(&data),
            "1 Then Joseph could no longer control himself before all his attendants, and the Lord was with him.\n"
        );
    }

    #[test]
    fn marks_each_poetry_line() {
        let chapter = setup("en", "NIV", "GEN");
        let html = r##"<html><div class="passage-text"><div class="std-text">
<p class="hang-2"><span class="text Gen-1-1"><span class="chapternum">1&nbsp;</span>In the beginning God created.</span></p>
<div class="poetry"><p class="line"><span class="text Gen-1-2">Praise him,<sup class="crossreference" data-cr="#cen-NIV-2A"><a href="#cen-NIV-2A">(A)</a></sup> all his angels;</span><br><span class="indent-1"><span class="indent-1-breaks">&nbsp;&nbsp;&nbsp;&nbsp;</span><span class="text Gen-1-2">praise him, all his hosts.</span></span></p></div>
</div></div>
<div class="crossrefs hidden"><h4>Cross references</h4><ol><li id="cen-NIV-2A"><a href="#en-NIV-2" title="Go to Genesis 1:2">Genesis 1:2</a> : <a class="crossref-link" data-bibleref="Psalm 148:2">Ps 148:2</a></li></ol></div>
</html>"##;
        let ChapterBody::Standard(data) = BibleGateway.parse(html, &chapter).unwrap() else {
            panic!("expected standard")
        };
        let kinds: Vec<&str> = data.verses[1]
            .content
            .iter()
            .map(|item| item.kind.as_str())
            .collect();
        assert_eq!(
            kinds,
            [
                "verse_number",
                "poetry_line",
                "text",
                "crossref",
                "text",
                "poetry_line",
                "text"
            ]
        );
        // The marker carries the indent of indented lines, and none for the
        // first line, as the corpus has it.
        assert_eq!(data.verses[1].content[1].number, None);
        assert_eq!(data.verses[1].content[5].number, Some(1));
        // The corpus records no indent for these lines; the marker is the
        // line break itself.
        assert_eq!(data.verses[1].poetry_indent, None);
    }

    #[test]
    fn rejects_a_different_chapter_or_book() {
        let chapter = setup("en", "NIV", "GEN");
        let html = fixture("NIV", "Gen", 1);
        let wrong_chapter = ChapterRef { chapter: 2, ..chapter };
        assert!(BibleGateway.validate(&html, &wrong_chapter).is_err());

        let mark = setup("en", "NIV", "MRK");
        assert!(BibleGateway.validate(&html, &mark).is_err());
    }
}
