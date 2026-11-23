//! Open Bible Data (freely-given.org) — one static page per chapter, at
//! `/OBD/{VERSION}/byC/{BOOK}_C{n}.htm`.
//!
//! Every version on the site comes from the same generator, so the markup is
//! shared: `<h1 id="Top">FBV GEN Chapter 1</h1>` opens the chapter, then a
//! paragraph per USFM marker (`p.p`, `p.q1`, `p.q2`, `p.m`, `p.pmo`), with
//! `p.s1` / `p.s2` / `p.d` for a heading and a psalm title.
//!
//! Verses are marked by id, not by class: a `<span id="C1V3">` (verse 1 arrives
//! as `class="c"`, or `class="cPsa"` in a psalm) carrying its number, followed
//! by `<span class="{VERSION}_verseTextChunk">` for the text. The id is the one
//! part the generator keeps stable, which is why the class is ignored.
//!
//! Notes are two-part, like BibleGateway: an inline caller (`span.fnCaller`,
//! `span.xrCaller`) inside the verse text, and the note itself in
//! `div#footnotes` / `div#crossRefs` below the chapter, joined by the href
//! (`#fn3`, `#xr1`). The page's own navigation repeats above and below the
//! chapter text and is skipped.
//!
//! Nothing needs JavaScript: the page is static, so the catalog waits only for
//! the `h1` and `validate` does the real checking against the site's
//! `data-version` / `data-book` / `data-chapter` attributes.

use anyhow::{Context, Result, bail};
use scraper::{ElementRef, Html, Node, Selector};
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::model::{
    ChapterBody, ChapterData, ContentItem, CrossRef, CrossRefLink, Footnote, Verse, normalized,
};
use crate::sites::{ChapterRef, Site};

pub struct Obd;

impl Site for Obd {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        let document = Html::parse_document(html);
        let header = navigation(&document)
            .with_context(|| format!("Missing navigation header for {}", chapter.passage()))?;
        let attribute = |name: &str| header.value().attr(name).unwrap_or_default();

        let expected_book = chapter.translation.book_slug(chapter.book)?;
        if attribute("data-version") != chapter.translation.version {
            bail!(
                "page is version {:?}, expected {:?}",
                attribute("data-version"),
                chapter.translation.version
            );
        }
        if attribute("data-book") != expected_book {
            bail!(
                "page is book {:?}, expected {expected_book:?}",
                attribute("data-book")
            );
        }
        if attribute("data-chapter").parse::<u32>().ok() != Some(chapter.chapter) {
            bail!(
                "page is chapter {:?}, expected {}",
                attribute("data-chapter"),
                chapter.chapter
            );
        }

        let verses = verse_numbers(&document, chapter);
        if !verses.contains(&1) {
            bail!("first verse missing from {}", chapter.passage());
        }
        if verses.len() < 2 {
            bail!("only {} verse(s) found", verses.len());
        }
        Ok(())
    }

    fn verse_count(&self, html: &str, chapter: &ChapterRef<'_>) -> usize {
        verse_numbers(&Html::parse_document(html), chapter).len()
    }

    fn parse(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterBody> {
        parse_chapter(html, chapter).map(ChapterBody::Standard)
    }
}

fn navigation(document: &Html) -> Option<ElementRef<'_>> {
    let selector = Selector::parse("div#bibleNavigation").ok()?;
    document.select(&selector).next()
}

/// `C1V3` → the chapter and verse this span marks.
fn marker_of(id: &str) -> Option<(u32, u32)> {
    let rest = id.strip_prefix('C')?;
    let (chapter, verse) = rest.split_once('V')?;
    Some((chapter.parse().ok()?, verse.parse().ok()?))
}

/// Every verse number the chapter page carries, from the markers' ids.
fn verse_numbers(document: &Html, chapter: &ChapterRef<'_>) -> HashSet<u32> {
    let spans = Selector::parse("span[id]").unwrap();
    document
        .select(&spans)
        .filter(|span| !is_chrome(span))
        .filter_map(|span| span.value().attr("id").and_then(marker_of))
        .filter(|(number, _)| *number == chapter.chapter)
        .map(|(_, verse)| verse)
        .collect()
}

/// Paragraphs, spans and containers that are page furniture rather than the
/// chapter: the book and chapter lists that repeat above and below the text,
/// the site's own header and footer, and the note panels.
fn is_chrome(element: &ElementRef<'_>) -> bool {
    let in_panel = element
        .ancestors()
        .filter_map(ElementRef::wrap)
        .any(|ancestor| {
            let value = ancestor.value();
            matches!(value.id(), Some("footnotes") | Some("crossRefs"))
                || value
                    .classes()
                    .any(|class| matches!(class, "header" | "footer" | "topLine"))
        });
    in_panel
        || element.value().classes().any(|class| {
            matches!(
                class,
                "bkLst"
                    | "cNav"
                    | "chLst"
                    | "copyright"
                    | "fn"
                    | "note"
                    | "site"
                    | "viewLst"
                    | "vsLst"
                    | "wrkLst"
                    | "xr"
                    | "bibleReferenceChoices"
            )
        })
}

fn has_class(element: &ElementRef<'_>, name: &str) -> bool {
    element.value().classes().any(|class| class == name)
}

/// A paragraph that carries a heading, not verse text: the USFM section and
/// title markers, plus the reference line the site puts above a parallel
/// passage.
fn is_heading(paragraph: &ElementRef<'_>) -> bool {
    paragraph.value().classes().any(|class| {
        matches!(class, "d" | "r" | "ms" | "mr" | "sp" | "qa" | "qd" | "cl")
            || (class.len() == 2
                && class.starts_with('s')
                && class.as_bytes()[1].is_ascii_digit())
    })
}

/// `q1` → 1, `q2` → 2, and nothing for a prose paragraph.
fn poetry_indent(paragraph: &ElementRef<'_>) -> Option<u32> {
    paragraph.value().classes().find_map(|class| {
        let level: u32 = class.strip_prefix('q')?.parse().ok()?;
        (1..=4).contains(&level).then_some(level)
    })
}

fn text_of(element: ElementRef<'_>) -> String {
    let mut buffer = String::new();
    collect_text(element, &mut buffer);
    normalized(&buffer)
}

/// The text of a note, with the inline caller markers left out.
fn collect_text(element: ElementRef<'_>, buffer: &mut String) {
    for child in element.children() {
        match child.value() {
            Node::Text(text) => buffer.push_str(text),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(child) else {
                    continue;
                };
                if has_class(&child, "fnCaller") || has_class(&child, "xrCaller") {
                    continue;
                }
                collect_text(child, buffer);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Notes: the chapter's own lists, keyed by the ids the callers point at.
// ---------------------------------------------------------------------------

struct FootnoteEntry {
    verse: u32,
    text: String,
}

struct CrossRefEntry {
    verse: u32,
    links: Vec<CrossRefLink>,
}

/// `#C1V16` → 16.
fn verse_from_href(href: &str) -> Option<u32> {
    marker_of(href.trim_start_matches('#')).map(|(_, verse)| verse)
}

/// The id a caller links to (`#fn3` → `fn3`).
fn caller_id(caller: &ElementRef<'_>) -> Option<String> {
    let anchor = Selector::parse("a[href]").ok()?;
    let href = caller.select(&anchor).next()?.value().attr("href")?;
    let id = href.trim_start_matches('#').to_owned();
    (!id.is_empty()).then_some(id)
}

fn collect_footnotes(document: &Html) -> HashMap<String, FootnoteEntry> {
    let rows = Selector::parse("div#footnotes p.fn[id]").unwrap();
    let reference = Selector::parse("span.fnRef a[href]").unwrap();
    let text = Selector::parse("span.fnText").unwrap();
    let mut out = HashMap::new();
    for row in document.select(&rows) {
        let Some(id) = row.value().attr("id") else {
            continue;
        };
        let Some(verse) = row
            .select(&reference)
            .next()
            .and_then(|anchor| anchor.value().attr("href"))
            .and_then(verse_from_href)
        else {
            continue;
        };
        out.insert(
            id.to_owned(),
            FootnoteEntry {
                verse,
                text: row
                    .select(&text)
                    .next()
                    .map(text_of)
                    .unwrap_or_else(|| text_of(row)),
            },
        );
    }
    out
}

fn collect_cross_refs(document: &Html) -> HashMap<String, CrossRefEntry> {
    let rows = Selector::parse("div#crossRefs p.xr[id]").unwrap();
    let reference = Selector::parse("span.xrRef a[href]").unwrap();
    let links = Selector::parse("span.xrText a[href]").unwrap();
    let mut out = HashMap::new();
    for row in document.select(&rows) {
        let Some(id) = row.value().attr("id") else {
            continue;
        };
        let Some(verse) = row
            .select(&reference)
            .next()
            .and_then(|anchor| anchor.value().attr("href"))
            .and_then(verse_from_href)
        else {
            continue;
        };
        let mut parsed = Vec::new();
        for link in row.select(&links) {
            if let Some(link) = cross_ref_link(&link) {
                parsed.push(link);
            }
        }
        if parsed.is_empty() {
            continue;
        }
        out.insert(id.to_owned(), CrossRefEntry { verse, links: parsed });
    }
    out
}

/// `ISA_C40.htm#C40V3` → Isaiah 40:3, named from the link's own text.
fn cross_ref_link(anchor: &ElementRef<'_>) -> Option<CrossRefLink> {
    let href = anchor.value().attr("href")?;
    let (path, fragment) = href.split_once('#').unwrap_or((href, ""));
    let file = path.rsplit('/').next().unwrap_or(path);
    let target = file.strip_suffix(".htm").unwrap_or(file);
    let (code, chapter) = target.rsplit_once("_C")?;
    let chapter: u32 = chapter.parse().ok()?;
    let (start, end) = verse_range(fragment)?;
    let text = normalized(&anchor.text().collect::<String>());
    Some(CrossRefLink {
        book: reference_name(&text).unwrap_or_else(|| code.to_owned()),
        chapter,
        verse: if end.is_none() { Some(start) } else { None },
        verse_start: end.map(|_| start),
        verse_end: end,
    })
}

/// `C40V3` → 3; `C40V3-V5` → 3 and 5.
fn verse_range(fragment: &str) -> Option<(u32, Option<u32>)> {
    match fragment.split_once('-') {
        Some((start, end)) => Some((verse_in(start)?, Some(verse_in(end)?))),
        None => Some((verse_in(fragment)?, None)),
    }
}

fn verse_in(fragment: &str) -> Option<u32> {
    let (_, verse) = fragment.rsplit_once('V')?;
    verse.parse().ok()
}

/// `Isaiah 40:3` → `Isaiah`. A link that names no verse keeps its whole text.
fn reference_name(text: &str) -> Option<String> {
    let (name, verses) = text.rsplit_once(' ')?;
    let (chapter, _) = verses.split_once(':')?;
    chapter.parse::<u32>().ok()?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// `0 → a`, `25 → z`, `26 → aa`, `27 → ab` — the scheme the Open English Bible
/// adapter uses for notes the site does not number.
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

// ---------------------------------------------------------------------------
// The chapter itself.
// ---------------------------------------------------------------------------

struct NoteContext<'a> {
    /// The short book code the note ids are keyed by, as the corpora do.
    book: &'a str,
    chapter: u32,
}

struct State<'a, 'b> {
    chapter: &'a ChapterRef<'b>,
    verses: BTreeMap<u32, Verse>,
    pending_headers: Vec<ContentItem>,
    current: Option<u32>,
    buffer: String,
    /// Whether the verse now being read opened its paragraph.
    paragraph_start: bool,
    poetry_indent: Option<u32>,
    /// Whether this paragraph has carried text yet, so a marker after text is
    /// a verse continuing the paragraph rather than starting one.
    text_in_paragraph: bool,
    footnotes: Vec<Footnote>,
    cross_refs: Vec<CrossRef>,
    footnotes_by_id: HashMap<String, FootnoteEntry>,
    cross_refs_by_id: HashMap<String, CrossRefEntry>,
    footnote_markers: HashMap<String, String>,
    crossref_markers: HashMap<String, String>,
    footnote_index: usize,
    crossref_index: usize,
}

/// The verse entry for `number`, created with the headings collected so far.
fn verse_entry<'s>(state: &'s mut State<'_, '_>, number: u32) -> &'s mut Verse {
    if !state.verses.contains_key(&number) {
        let header = std::mem::take(&mut state.pending_headers);
        state.verses.insert(
            number,
            Verse {
                chapter: state.chapter.chapter,
                verse: number,
                content: vec![ContentItem::new("verse_number", None, Some(number), None)],
                header,
                section: String::new(),
                paragraph_start: state.paragraph_start,
                poetry_indent: state.poetry_indent,
            },
        );
    }
    state.verses.get_mut(&number).expect("just inserted")
}

fn flush(state: &mut State<'_, '_>) {
    let text = normalized(&state.buffer);
    state.buffer.clear();
    let Some(number) = state.current else {
        return;
    };
    if text.is_empty() {
        return;
    }
    state.text_in_paragraph = true;
    verse_entry(state, number)
        .content
        .push(ContentItem::text("text", text));
}

/// Starts the verse a marker announces.
///
/// Some versions print a marked verse and then give its text to the next
/// verse: T4T opens Mark and Luke on a verse 1 with no text, because verse 2's
/// text serves both — the site's own parallel view labels it `1-2`. Such a
/// marker joins the empty verse before it, so the text stays in one entry and
/// the range goes in `display`, the field combined verses already use.
fn begin_verse(state: &mut State<'_, '_>, number: u32) {
    let mut start = number;
    if let Some(previous) = state.current
        && previous != number
        && carries_no_text(state, previous)
    {
        let entry = state.verses.remove(&previous).expect("just checked");
        // The headings that empty verse collected belong to the one that
        // carries the text.
        let mut header = entry.header;
        header.append(&mut state.pending_headers);
        state.pending_headers = header;
        start = previous;
    }
    state.current = Some(start);
    let entry = verse_entry(state, start);
    if start != number {
        entry.content[0].display = Some(format!("{start}-{number}"));
    }
}

fn carries_no_text(state: &State<'_, '_>, number: u32) -> bool {
    state
        .verses
        .get(&number)
        .is_some_and(|verse| verse.content.iter().all(|item| item.kind == "verse_number"))
}

fn push_marker(state: &mut State<'_, '_>, item: ContentItem) {
    let Some(number) = state.current else {
        return;
    };
    state.text_in_paragraph = true;
    verse_entry(state, number).content.push(item);
}

fn push_footnote(state: &mut State<'_, '_>, caller: &ElementRef<'_>, context: &NoteContext<'_>) {
    let Some(id) = caller_id(caller) else {
        return;
    };
    let Some(entry) = state.footnotes_by_id.get(&id) else {
        return;
    };
    let verse = entry.verse;
    let text = entry.text.clone();
    let marker = match state.footnote_markers.get(&id) {
        Some(marker) => marker.clone(),
        None => {
            let marker = note_marker(state.footnote_index);
            state.footnote_index += 1;
            let note_id = format!("{}:{}:{verse}-{marker}", context.book, context.chapter);
            state.footnotes.push(Footnote {
                id: note_id,
                text,
                references: BTreeMap::new(),
            });
            state.footnote_markers.insert(id, marker.clone());
            marker
        }
    };
    let note_id = format!("{}:{}:{verse}-{marker}", context.book, context.chapter);
    push_marker(
        state,
        ContentItem::new("footnote", Some(marker), None, Some(note_id)),
    );
}

fn push_cross_ref(state: &mut State<'_, '_>, caller: &ElementRef<'_>, context: &NoteContext<'_>) {
    let Some(id) = caller_id(caller) else {
        return;
    };
    let Some(entry) = state.cross_refs_by_id.get(&id) else {
        return;
    };
    let verse = entry.verse;
    let links = entry.links.clone();
    let marker = match state.crossref_markers.get(&id) {
        Some(marker) => marker.clone(),
        None => {
            let marker = note_marker(state.crossref_index).to_uppercase();
            state.crossref_index += 1;
            let note_id = format!("{}:{}:{verse}-{marker}", context.book, context.chapter);
            state.cross_refs.push(CrossRef {
                id: note_id,
                book: context.book.to_owned(),
                chapter: context.chapter,
                verse,
                links,
            });
            state.crossref_markers.insert(id, marker.clone());
            marker
        }
    };
    let note_id = format!("{}:{}:{verse}-{marker}", context.book, context.chapter);
    push_marker(
        state,
        ContentItem::new("crossref", Some(marker), None, Some(note_id)),
    );
}

fn walk_inline(element: ElementRef<'_>, state: &mut State<'_, '_>, context: &NoteContext<'_>) {
    for child in element.children() {
        match child.value() {
            Node::Text(text) => state.buffer.push_str(text),
            Node::Element(_) => {
                let Some(child) = ElementRef::wrap(child) else {
                    continue;
                };
                // A verse marker is recognised by its id alone.
                if let Some((chapter, verse)) = child.value().attr("id").and_then(marker_of)
                    && chapter == state.chapter.chapter {
                        flush(state);
                        state.paragraph_start = !state.text_in_paragraph;
                        begin_verse(state, verse);
                        continue;
                    }
                if has_class(&child, "fnCaller") {
                    flush(state);
                    push_footnote(state, &child, context);
                    continue;
                }
                if has_class(&child, "xrCaller") {
                    flush(state);
                    push_cross_ref(state, &child, context);
                    continue;
                }
                walk_inline(child, state, context);
            }
            _ => {}
        }
    }
}

fn walk_paragraph(paragraph: ElementRef<'_>, state: &mut State<'_, '_>, context: &NoteContext<'_>) {
    if is_heading(&paragraph) {
        let text = text_of(paragraph);
        if !text.is_empty() {
            state.pending_headers.push(ContentItem::text("text", text));
        }
        return;
    }
    state.text_in_paragraph = false;
    state.poetry_indent = poetry_indent(&paragraph);
    walk_inline(paragraph, state, context);
    flush(state);
}

fn parse_chapter(html: &str, chapter: &ChapterRef<'_>) -> Result<ChapterData> {
    let document = Html::parse_document(html);
    navigation(&document).with_context(|| {
        format!("Missing navigation header for {}", chapter.passage())
    })?;

    let context = NoteContext {
        book: &chapter.book.short,
        chapter: chapter.chapter,
    };
    let mut state = State {
        chapter,
        verses: BTreeMap::new(),
        pending_headers: Vec::new(),
        current: None,
        buffer: String::new(),
        paragraph_start: true,
        poetry_indent: None,
        text_in_paragraph: false,
        footnotes: Vec::new(),
        cross_refs: Vec::new(),
        footnotes_by_id: collect_footnotes(&document),
        cross_refs_by_id: collect_cross_refs(&document),
        footnote_markers: HashMap::new(),
        crossref_markers: HashMap::new(),
        footnote_index: 0,
        crossref_index: 0,
    };

    let elements = Selector::parse("p, span.cPsa[id], span.v[id]").unwrap();
    for element in document.select(&elements) {
        if element.value().name() == "p" {
            if !is_chrome(&element) {
                walk_paragraph(element, &mut state, &context);
            }
        } else if chapter.usfm() == "PSA"
            && element.value().attr("id").and_then(marker_of) == Some((chapter.chapter, 1))
            && !element
                .ancestors()
                .filter_map(ElementRef::wrap)
                .any(|parent| parent.value().name() == "p")
        {
            // ULT and UST place this marker before the verse's paragraph.
            flush(&mut state);
            state.paragraph_start = true;
            begin_verse(&mut state, 1);
            // Some verseText blocks also carry the opening text beside the marker.
            if let Some(parent) = element.parent().and_then(ElementRef::wrap)
                && parent.value().name() == "div"
                && has_class(&parent, "verseText")
            {
                walk_inline(parent, &mut state, &context);
                flush(&mut state);
            }
        }
    }
    flush(&mut state);

    if state.verses.is_empty() {
        bail!("No verses found in {}", chapter.passage());
    }
    if !state.verses.contains_key(&1) {
        bail!("First verse missing from {}", chapter.passage());
    }

    let mut data = ChapterData::new(
        &chapter.translation.code,
        &chapter.translation.name,
        &chapter.book.short,
        &chapter.book.name,
        chapter.chapter,
    );
    data.verses = state.verses.into_values().collect();
    data.footnotes = state.footnotes;
    data.cross_refs = state.cross_refs;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    fn catalog() -> &'static Catalog {
        Box::leak(Box::new(
            Catalog::from_str(include_str!("../../catalog.toml")).unwrap(),
        ))
    }

    fn chapter_ref(code: &str, usfm: &str, chapter: u32) -> ChapterRef<'static> {
        let catalog = catalog();
        ChapterRef {
            translation: Box::leak(Box::new(catalog.resolve("en", code).unwrap())),
            book: catalog.book(usfm).unwrap(),
            chapter,
        }
    }

    /// A trimmed-down chapter page: the furniture the site repeats above and
    /// below the text, two and a half verses, a heading, and one note of each
    /// kind.
    fn fixture(code: &str, book: &str, chapter: u32) -> String {
        format!(
            r##"<html><head><title>{book} {chapter} | {code}</title></head>
<body class="container" data-page-type="chapter">
<div class="topLine"><p class="site"><a href="../../index.htm#Top">Open Bible Data Home</a></p></div>
<div class="header" id="bibleNavigation" data-root="../../" data-version="{code}" data-mode="byC" data-book="{book}" data-chapter="{chapter}" data-verse="1"><p class="wrkLst">OET ULT UST BSB</p><p class="viewLst">By Document By Chapter</p></div>
<p class="bkLst">{code} <span class="selectedBook">{book}</span></p>
<p class="chLst" id="chLst">{code} <a href="./">{book}</a><span><a href="{book}_C2.htm#Top">C2</a></span></p>
<p class="vsLst bibleReferenceChoices" data-nav-verses="1">FBV {book} {chapter}<span><a href="#C{chapter}V1">V1</a></span><span><a href="#C{chapter}V2">V2</a></span></p>
<h1 id="Top">{code} {book} Chapter {chapter}</h1>
<p class="cNav"><a title="Previous (book introduction)" href="{book}_Intro.htm#Top">&#9668;</a></p>
<p class="p">
 <span id="V1"></span><span class="c" id="C{chapter}V1"><a href="#Top">1</a> </span><span class="{code}_verseTextChunk">In the beginning God created the heavens and the earth.</span> <span id="V2"></span><span class="v" id="C{chapter}V2"><a href="#Top">2</a> </span><span class="{code}_verseTextChunk">The earth was formless.</span></p>
<p class="s1">The First Day</p>
<p class="p">
 <span id="V3"></span><span class="v" id="C{chapter}V3"><a href="#Top">3</a> </span><span class="{code}_verseTextChunk">God said, &#8220;Let there be<span class="fnCaller">[<a title="Note: A note about light." href="#fn1">fn</a>]</span> light!&#8221;</span> <span id="V4"></span><span class="v" id="C{chapter}V4"><a href="#Top">4</a> </span><span class="{code}_verseTextChunk">And there was light<span class="xrCaller">[<a title="See also Isaiah 40:3" href="#xr1">ref</a>]</span>.</span></p>
<hr class="line-before-footnotes"><div id="footnotes" class="footnotes">
<p class="fn" id="fn1"><span class="fnRef"><a title="Return to text" href="#C{chapter}V3">{chapter}:3</a></span> <span class="fnText"><span class="ft">A note about light.</span></span></p>
</div><div id="crossRefs" class="crossRefs">
<p class="xr" id="xr1"><span class="xrRef"><a title="Return to text" href="#C{chapter}V4">{chapter}:4</a></span> <span class="xrText"><a data-vref="mapped" href="ISA_C40.htm#C40V3">Isaiah 40:3</a></span></p>
</div>
<p class="cNav"><a href="{book}_C2.htm#Top">&#9658;</a></p>
<p class="chLst">{code} <a href="./">{book}</a><span><a href="{book}_C2.htm#Top">C2</a></span></p>
<div class="footer" id="footer"><p class="copyright" id="Bottom">Open Bible Data site copyright</p></div>
</body></html>"##
        )
    }

    #[test]
    fn parses_verses_and_headings() {
        let chapter = chapter_ref("FBV", "GEN", 1);
        let html = fixture("FBV", "GEN", 1);
        assert!(Obd.validate(&html, &chapter).is_ok());
        assert_eq!(Obd.verse_count(&html, &chapter), 4);

        let ChapterBody::Standard(data) = Obd.parse(&html, &chapter).unwrap() else {
            panic!("expected standard")
        };
        assert_eq!(data.verses.len(), 4);
        assert_eq!(data.verses[0].header.len(), 0);
        // The heading belongs to the verse that follows it.
        assert_eq!(data.verses[2].header[0].text.as_deref(), Some("The First Day"));
        assert_eq!(
            normalized(&data.verses[0].content[1].text.clone().unwrap()),
            "In the beginning God created the heavens and the earth."
        );
        // Verse numbers come from the markers, so the nav lists stay out.
        assert_eq!(data.verses[3].verse, 4);
        assert_eq!(
            crate::model::render_text_chapter(&data),
            "1 In the beginning God created the heavens and the earth.\n\
             2 The earth was formless.\n\
             3 God said, “Let there be light!”\n\
             4 And there was light.\n"
        );
        assert_eq!(data.translation_code, "FBV");
        assert_eq!((data.book_code.as_str(), data.chapter), ("Gen", 1));
    }

    #[test]
    fn lifts_notes_out_of_the_verse_text() {
        let chapter = chapter_ref("FBV", "GEN", 1);
        let ChapterBody::Standard(data) = Obd.parse(&fixture("FBV", "GEN", 1), &chapter).unwrap()
        else {
            panic!("expected standard")
        };

        assert_eq!(data.footnotes.len(), 1);
        assert_eq!(data.footnotes[0].id, "Gen:1:3-a");
        assert_eq!(data.footnotes[0].text, "A note about light.");
        assert!(data.footnotes[0].references.is_empty());

        assert_eq!(data.cross_refs.len(), 1);
        let cross_ref = &data.cross_refs[0];
        assert_eq!(cross_ref.id, "Gen:1:4-A");
        assert_eq!(cross_ref.book, "Gen");
        assert_eq!((cross_ref.chapter, cross_ref.verse), (1, 4));
        assert_eq!(cross_ref.links.len(), 1);
        assert_eq!(
            (
                cross_ref.links[0].book.as_str(),
                cross_ref.links[0].chapter,
                cross_ref.links[0].verse
            ),
            ("Isaiah", 40, Some(3))
        );

        // Both notes are reachable from a marker, and neither leaks into text.
        let markers = |kind: &str| {
            data.verses
                .iter()
                .flat_map(|verse| verse.content.iter())
                .filter(|item| item.kind == kind)
                .count()
        };
        assert_eq!(markers("footnote"), data.footnotes.len());
        assert_eq!(markers("crossref"), data.cross_refs.len());
        let text = crate::model::render_text_chapter(&data);
        assert!(!text.contains("[fn]") && !text.contains("[ref]"), "{text}");
    }

    #[test]
    fn marks_poetry_and_reads_psalm_markers() {
        // A psalm opens on `class="cPsa"`, so the marker id is what counts.
        let chapter = chapter_ref("LSV", "PSA", 1);
        let html = fixture("LSV", "PSA", 1).replace("class=\"c\"", "class=\"cPsa\"");
        let html = html.replace("<p class=\"p\">\n <span id=\"V1\">", "<p class=\"q1\">\n <span id=\"V1\">");
        assert!(Obd.validate(&html, &chapter).is_ok());

        let ChapterBody::Standard(data) = Obd.parse(&html, &chapter).unwrap() else {
            panic!("expected standard")
        };
        assert_eq!(data.verses[0].poetry_indent, Some(1));
        assert_eq!(data.verses.len(), 4);
    }

    #[test]
    fn a_marker_without_text_joins_the_next_verse() {
        // T4T opens Mark 1 on a marked verse 1 with no text: verse 2's text is
        // the text of both, which the site's parallel view labels `1-2`.
        let chapter = chapter_ref("T4T", "MRK", 1);
        let html = r##"<html><body>
<div class="header" id="bibleNavigation" data-version="T4T" data-book="MRK" data-chapter="1"></div>
<h1 id="Top">T4T MRK Chapter 1</h1>
<p class="p"><span id="C1"></span><span class="c" id="C1V1"><a>1</a></span><span id="V2"></span><span class="v" id="C1V2"><a>2</a></span><span class="T4T_verseTextChunk">The good message began.</span></p>
</body></html>"##;
        assert!(Obd.validate(html, &chapter).is_ok());
        assert_eq!(Obd.verse_count(html, &chapter), 2);

        let ChapterBody::Standard(data) = Obd.parse(html, &chapter).unwrap() else {
            panic!("expected standard")
        };
        assert_eq!(data.verses.len(), 1, "one entry carries the pair");
        assert_eq!(data.verses[0].verse, 1);
        assert_eq!(data.verses[0].content[0].display.as_deref(), Some("1-2"));
        assert_eq!(
            crate::model::render_text_chapter(&data),
            "1-2 The good message began.\n"
        );
    }

    #[test]
    fn rejects_a_different_chapter_book_or_version() {
        let chapter = chapter_ref("FBV", "GEN", 1);
        let html = fixture("FBV", "GEN", 1);

        assert!(Obd.validate(&html, &ChapterRef { chapter: 2, ..chapter }).is_err());
        assert!(Obd.validate(&html, &chapter_ref("FBV", "MRK", 1)).is_err());
        assert!(Obd.validate(&html, &chapter_ref("MSB", "GEN", 1)).is_err());
        // The site answers a missing chapter with a short page of its own.
        assert!(Obd.validate("<html><body>404</body></html>", &chapter).is_err());
    }
}
