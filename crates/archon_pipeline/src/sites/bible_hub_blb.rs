use anyhow::{Context, Result, ensure};
use scraper::{ElementRef, Html, Node, Selector};

use crate::model::{ChapterBody, ChapterData, ContentItem, Footnote, Verse, normalized};
use crate::sites::{ChapterRef, Site};

pub struct BibleHubBlb;

impl Site for BibleHubBlb {
    fn validate(&self, html: &str, chapter: &ChapterRef<'_>) -> Result<()> {
        self.parse(html, chapter).map(|_| ())
    }

    fn verse_count(&self, html: &str, _chapter: &ChapterRef<'_>) -> usize {
        let verses = Selector::parse("#leftbox .grk .reftext").unwrap();
        Html::parse_document(html)
            .select(&verses)
            .filter(|element| {
                !element
                    .ancestors()
                    .filter_map(ElementRef::wrap)
                    .any(is_heading)
            })
            .enumerate()
            .filter(|(index, element)| {
                *index == 0 || element.text().any(|text| !text.trim().is_empty())
            })
            .count()
    }

    fn parse(&self, html: &str, reference: &ChapterRef<'_>) -> Result<ChapterBody> {
        let document = Html::parse_document(html);
        let title = Selector::parse("title").unwrap();
        let suffix = format!(" {} Berean Literal Bible", reference.chapter);
        ensure!(
            document
                .select(&title)
                .any(|element| normalized(&element.text().collect::<String>()).ends_with(&suffix)),
            "Unexpected BLB title for {}",
            reference.passage()
        );
        let scope = Selector::parse("#leftbox .grk").unwrap();
        let body = document
            .select(&scope)
            .next()
            .context("Missing BLB scripture text")?;
        let slug = reference.translation.book_slug(reference.book)?;
        let mut parser = ChapterParser {
            data: ChapterData::new(
                &reference.translation.code,
                &reference.translation.name,
                &reference.book.short,
                &reference.book.name,
                reference.chapter,
            ),
            verse_prefix: format!("/{slug}/{}-", reference.chapter),
            is_psalm: reference.usfm() == "PSA",
            has_chapter_link: false,
            text: String::new(),
            headings: Vec::new(),
            paragraph_start: true,
            in_notes: false,
            current_note: None,
            markers: Vec::new(),
            skipped_prefix_notes: 0,
            pending_plain_verse: None,
        };
        parser.walk(body)?;
        parser.flush();
        parser.link_notes()?;
        ensure!(
            !parser.data.verses.is_empty(),
            "No BLB verses found for {}",
            reference.passage()
        );
        ensure!(
            parser.has_chapter_link,
            "Missing BLB verse link for {}",
            reference.passage()
        );
        Ok(ChapterBody::Standard(parser.data))
    }
}

struct ChapterParser {
    data: ChapterData,
    verse_prefix: String,
    is_psalm: bool,
    has_chapter_link: bool,
    text: String,
    headings: Vec<ContentItem>,
    paragraph_start: bool,
    in_notes: bool,
    current_note: Option<u32>,
    markers: Vec<String>,
    skipped_prefix_notes: usize,
    pending_plain_verse: Option<u32>,
}

impl ChapterParser {
    fn walk(&mut self, parent: ElementRef<'_>) -> Result<()> {
        for child in parent.children() {
            match child.value() {
                Node::Text(text) => {
                    let mut text: &str = text;
                    if !text.trim().is_empty() {
                        if let Some(verse) = self.pending_plain_verse.take() {
                            let trimmed = text.trim_start();
                            let digits = trimmed.trim_start_matches(|ch: char| ch.is_ascii_digit());
                            let label = &trimmed[..trimmed.len() - digits.len()];
                            if label.parse::<u32>() == Ok(verse)
                                && self
                                    .data
                                    .verses
                                    .last()
                                    .is_none_or(|previous| previous.verse < verse)
                            {
                                self.begin_verse(verse)?;
                                text = digits;
                            }
                        }
                    }
                    self.text.push_str(text);
                }
                Node::Element(_) => {
                    let element = ElementRef::wrap(child).unwrap();
                    if element.value().name() == "a" && element.attr("name") == Some("fn") {
                        self.flush();
                        self.in_notes = true;
                        self.pending_plain_verse = None;
                        continue;
                    }
                    if self.in_notes {
                        if has_class(element, "fnverse") {
                            self.flush();
                            self.current_note =
                                element.text().collect::<String>().trim().parse().ok();
                        } else if element.value().name() == "br" {
                            self.flush();
                        } else {
                            self.walk(element)?;
                        }
                    } else if has_class(element, "reftext") {
                        self.pending_plain_verse = None;
                        self.start_verse(element)?;
                    } else if element.value().name() == "a" && element.attr("href").is_none() {
                        // BibleHub's legacy anchors name the next verse, even
                        // when the visible number is printed without a link.
                        self.pending_plain_verse = element
                            .attr("name")
                            .and_then(|name| name.parse::<u32>().ok())
                            .and_then(|number| number.checked_sub(1))
                            .filter(|number| *number > 0);
                        self.walk(element)?;
                    } else if is_heading(element) {
                        self.flush();
                        let mut text = String::new();
                        collect_heading(element, &mut text, &mut self.markers);
                        append_annotated_text(&mut self.headings, &text);
                    } else if has_class(element, "fn") {
                        self.markers
                            .push(normalized(&element.text().collect::<String>()));
                        self.text.push(NOTE_ANCHOR);
                    } else if has_class(element, "cross") {
                        continue;
                    } else {
                        if element.value().name() == "p" {
                            self.paragraph_start = true;
                        }
                        if matches!(element.value().name(), "p" | "br") {
                            self.text.push(' ');
                        }
                        self.walk(element)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn start_verse(&mut self, element: ElementRef<'_>) -> Result<()> {
        let label = element.text().collect::<String>();
        let label = label.trim();
        let link = Selector::parse("a[href]").unwrap();
        let href = element
            .select(&link)
            .next()
            .and_then(|anchor| anchor.attr("href"));
        let target = href
            .and_then(|href| href.strip_prefix(&self.verse_prefix))
            .and_then(|suffix| suffix.strip_suffix(".htm"))
            .and_then(|number| number.parse::<u32>().ok());
        // After a Psalm superscription, BibleHub can link the printed verse 1
        // to a placeholder. A numbered verse link still confirms the chapter.
        let placeholder =
            self.is_psalm && self.data.verses.is_empty() && label == "1" && href == Some("#");
        ensure!(
            target.is_some() || placeholder,
            "Unexpected BLB verse link, expected prefix {}",
            self.verse_prefix
        );
        // Some Psalms hide the first verse label. Blank labels later in a
        // chapter mark omissions. Links can drift after an omitted verse.
        let verse = match label {
            "" if self.data.verses.is_empty() && target == Some(1) => 1,
            "" => return Ok(()),
            number => number.parse::<u32>()?,
        };
        self.has_chapter_link |= target.is_some();
        self.begin_verse(verse)
    }

    fn begin_verse(&mut self, verse: u32) -> Result<()> {
        self.flush();
        ensure!(
            self.data
                .verses
                .last()
                .is_none_or(|previous| previous.verse < verse),
            "Duplicate or out-of-order BLB verse {verse}"
        );
        self.data.verses.push(Verse {
            chapter: self.data.chapter,
            verse,
            content: Vec::new(),
            header: std::mem::take(&mut self.headings),
            section: String::new(),
            paragraph_start: std::mem::take(&mut self.paragraph_start),
            poetry_indent: None,
        });
        Ok(())
    }

    fn flush(&mut self) {
        let text = normalized(&std::mem::take(&mut self.text));
        if self.in_notes {
            if let Some(number) = self.current_note.take().filter(|_| !text.is_empty()) {
                let id = format!(
                    "{}:{}:{number}-{}",
                    self.data.book_code,
                    self.data.chapter,
                    self.data.footnotes.len() + 1
                );
                self.data.footnotes.push(Footnote {
                    id,
                    text,
                    references: Default::default(),
                });
            }
        } else if let Some(verse) = self.data.verses.last_mut().filter(|_| !text.is_empty()) {
            append_annotated_text(&mut verse.content, &text);
        } else if !self.in_notes {
            // Legacy inputs can contain unnumbered opening text before the first
            // recognised verse. Keep those footer bodies at chapter level.
            self.skipped_prefix_notes += text.matches(NOTE_ANCHOR).count();
        }
    }

    fn link_notes(&mut self) -> Result<()> {
        ensure!(
            self.markers.len() == self.data.footnotes.len(),
            "BLB {} {}: {} anchors for {} notes",
            self.data.book_code,
            self.data.chapter,
            self.markers.len(),
            self.data.footnotes.len()
        );
        let mut notes = self
            .markers
            .iter()
            .zip(&self.data.footnotes)
            .skip(self.skipped_prefix_notes);
        for verse in &mut self.data.verses {
            for item in verse.header.iter_mut().chain(&mut verse.content) {
                if item.kind == "footnote" {
                    let (marker, note) = notes.next().context("Missing BLB note")?;
                    item.id = Some(note.id.clone());
                    item.text = Some(marker.clone());
                }
            }
        }
        ensure!(
            notes.next().is_none(),
            "Unplaced BLB footnote in {} {}",
            self.data.book_code,
            self.data.chapter
        );
        Ok(())
    }
}

fn has_class(element: ElementRef<'_>, class: &str) -> bool {
    element.value().classes().any(|name| name == class)
}

fn is_heading(element: ElementRef<'_>) -> bool {
    ["hdg", "ihdg", "subhdg", "suphdg", "pshdg", "acrostic"]
        .iter()
        .any(|class| has_class(element, class))
}

// A temporary boundary in the DOM text buffer. Never written to chapter JSON.
const NOTE_ANCHOR: char = '\u{fdd0}';

fn collect_heading(element: ElementRef<'_>, text: &mut String, markers: &mut Vec<String>) {
    for child in element.children() {
        match child.value() {
            Node::Text(value) => text.push_str(value),
            Node::Element(_) => {
                let node = ElementRef::wrap(child).unwrap();
                if has_class(node, "fn") {
                    markers.push(normalized(&node.text().collect::<String>()));
                    text.push(NOTE_ANCHOR);
                } else if !has_class(node, "cross") {
                    collect_heading(node, text, markers);
                }
            }
            _ => {}
        }
    }
}

fn append_annotated_text(items: &mut Vec<ContentItem>, raw: &str) {
    // Offsets use UTF-16 to match browser text-selection offsets.
    let previous_length = items
        .iter()
        .filter(|item| item.kind == "text")
        .filter_map(|item| item.text.as_ref())
        .map(|s| s.encode_utf16().count() + 1)
        .sum::<usize>();
    let mut offset = previous_length;
    let mut text = String::new();
    let mut annotations = Vec::new();
    let mut space = false;
    for ch in raw.chars() {
        match ch {
            NOTE_ANCHOR | '⁺' => {
                let kind = if ch == NOTE_ANCHOR {
                    "footnote"
                } else {
                    "plural"
                };
                annotations.push(ContentItem::new(kind, None, Some(offset as u32), None));
            }
            ch if ch.is_whitespace() => space = !text.is_empty(),
            ch => {
                if space {
                    text.push(' ');
                    offset += 1;
                    space = false;
                }
                text.push(ch);
                offset += ch.len_utf16();
            }
        }
    }
    if !text.is_empty() {
        items.push(ContentItem::text("text", text));
    }
    items.extend(annotations);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

    #[test]
    fn parses_a_psalm_superscription_before_a_placeholder_verse_link() {
        let catalog = Catalog::from_str(crate::CATALOG).unwrap();
        let translation = catalog.resolve("en", "BLB").unwrap();
        let chapter = ChapterRef {
            translation: &translation,
            book: catalog.book("PSA").unwrap(),
            chapter: 3,
        };
        let html = r##"
            <title>Psalm 3 Berean Literal Bible</title>
            <div id="leftbox"><div class="grk">
            <p class="pshdg"><span class="reftext"><a href="/psalms/3-1.htm"><b></b></a></span>Superscription.
            <p><span class="reftext"><a href="#"><b>1</b></a></span>First verse.
            <span class="reftext"><a href="/psalms/3-2.htm"><b>2</b></a></span>Second verse.
            </div></div>
        "##;
        let ChapterBody::Standard(data) = BibleHubBlb.parse(html, &chapter).unwrap() else {
            panic!("BLB must use the standard chapter schema");
        };
        assert_eq!(BibleHubBlb.verse_count(html, &chapter), 2);
        assert_eq!(data.verses.len(), 2);
        assert_eq!(data.verses[0].verse, 1);
        assert_eq!(
            data.verses[0].header[0].text.as_deref(),
            Some("Superscription.")
        );
        assert_eq!(
            data.verses[0].content[0].text.as_deref(),
            Some("First verse.")
        );
        assert!(
            BibleHubBlb
                .validate(&html.replace("/psalms/3-2.htm", "#"), &chapter)
                .is_err()
        );
    }

    #[test]
    fn uses_printed_labels_after_omissions_and_recovers_a_hidden_first_label() {
        let catalog = Catalog::from_str(crate::CATALOG).unwrap();
        let translation = catalog.resolve("en", "BLB").unwrap();
        let chapter = ChapterRef {
            translation: &translation,
            book: catalog.book("PSA").unwrap(),
            chapter: 1,
        };
        let html = r#"
            <title>Psalms 1 Berean Literal Bible</title>
            <div id="leftbox"><div class="grk">
            <p class="suphdg">Book heading
            <p class="hdg">Psalm heading
            <p><span class="reftext"><a href="/psalms/1-1.htm"><b></b></a></span>First verse.
            <span class="reftext"><a href="/psalms/1-2.htm"><b></b></a></span>
            <span class="reftext"><a href="/psalms/1-2.htm"><b>3</b></a></span>Third verse.
            </div></div>
        "#;
        let ChapterBody::Standard(data) = BibleHubBlb.parse(html, &chapter).unwrap() else {
            panic!("BLB must use the standard chapter schema");
        };
        assert_eq!(
            data.verses
                .iter()
                .map(|verse| verse.verse)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(data.verses[0].header.len(), 2);
        assert_eq!(BibleHubBlb.verse_count(html, &chapter), 2);
        assert_eq!(
            data.verses[0].content[0].text.as_deref(),
            Some("First verse.")
        );
        assert!(
            BibleHubBlb
                .validate(&html.replace("/psalms/", "/proverbs/"), &chapter)
                .is_err()
        );
    }
}
