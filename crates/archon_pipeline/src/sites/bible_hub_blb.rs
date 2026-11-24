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
            text: String::new(),
            headings: Vec::new(),
            paragraph_start: true,
            in_notes: false,
            current_note: None,
        };
        parser.walk(body)?;
        parser.flush();
        ensure!(
            !parser.data.verses.is_empty(),
            "No BLB verses found for {}",
            reference.passage()
        );
        Ok(ChapterBody::Standard(parser.data))
    }
}

struct ChapterParser {
    data: ChapterData,
    verse_prefix: String,
    text: String,
    headings: Vec<ContentItem>,
    paragraph_start: bool,
    in_notes: bool,
    current_note: Option<u32>,
}

impl ChapterParser {
    fn walk(&mut self, parent: ElementRef<'_>) -> Result<()> {
        for child in parent.children() {
            match child.value() {
                Node::Text(text) => self.text.push_str(text),
                Node::Element(_) => {
                    let element = ElementRef::wrap(child).unwrap();
                    if element.value().name() == "a" && element.attr("name") == Some("fn") {
                        self.flush();
                        self.in_notes = true;
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
                        self.start_verse(element)?;
                    } else if ["hdg", "ihdg", "subhdg", "suphdg", "pshdg", "acrostic"]
                        .iter()
                        .any(|class| has_class(element, class))
                    {
                        self.flush();
                        let heading = heading_text(element);
                        if !heading.is_empty() {
                            self.headings.push(ContentItem::text("text", heading));
                        }
                    } else if has_class(element, "fn") || has_class(element, "cross") {
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
        let target = element
            .select(&link)
            .next()
            .and_then(|anchor| anchor.attr("href"))
            .and_then(|href| href.strip_prefix(&self.verse_prefix))
            .and_then(|suffix| suffix.strip_suffix(".htm"))
            .and_then(|number| number.parse::<u32>().ok());
        ensure!(
            target.is_some(),
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
                if let Ok(index) = self
                    .data
                    .verses
                    .binary_search_by_key(&number, |verse| verse.verse)
                {
                    self.data.verses[index].content.push(ContentItem::new(
                        "footnote",
                        Some("‡".into()),
                        None,
                        Some(id.clone()),
                    ));
                }
                self.data.footnotes.push(Footnote {
                    id,
                    text,
                    references: Default::default(),
                });
            }
        } else if let Some(verse) = self.data.verses.last_mut().filter(|_| !text.is_empty()) {
            verse.content.push(ContentItem::text("text", text));
        }
    }
}

fn has_class(element: ElementRef<'_>, class: &str) -> bool {
    element.value().classes().any(|name| name == class)
}

fn heading_text(element: ElementRef<'_>) -> String {
    let text = element
        .descendants()
        .filter_map(|node| {
            let Node::Text(text) = node.value() else {
                return None;
            };
            let in_cross_reference = node
                .ancestors()
                .take_while(|ancestor| ancestor.id() != element.id())
                .filter_map(ElementRef::wrap)
                .any(|ancestor| has_class(ancestor, "cross"));
            (!in_cross_reference).then_some(&**text)
        })
        .collect::<String>();
    normalized(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;

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
