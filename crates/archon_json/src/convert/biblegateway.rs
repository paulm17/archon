use regex::Regex;
use scraper::{Html, Selector, ElementRef};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;


#[allow(dead_code)]
#[derive(Debug, Serialize, Deserialize, Clone)]
struct Book {
    id: String,
    name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ContentItem {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
    number: Option<u32>,
    id: Option<String>,
    display: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Verse {
    chapter: u32,
    verse: u32,
    content: Vec<ContentItem>,
    header: Vec<ContentItem>,
    section: String,
    paragraph_start: bool,
    poetry_indent: Option<u32>,
}

#[derive(Debug, Default)]
struct WalkContext {
    poetry_mode: bool,
    pending_poetry_line: bool,
    current_indent: Option<u32>,
    indent_history: Vec<Option<u32>>,
}

impl WalkContext {
    fn push_indent(&mut self, indent: u32) {
        let previous = self.current_indent;
        self.indent_history.push(previous);
        let merged = previous.map(|val| val.max(indent)).unwrap_or(indent);
        self.current_indent = Some(merged);
    }

    fn pop_indent(&mut self) {
        self.current_indent = self.indent_history.pop().unwrap_or(None);
    }

    fn ensure_poetry_line_started<F>(
        &mut self,
        text_buffer: &mut String,
        content_items: &mut Vec<ContentItem>,
        flush_text: &mut F,
    ) where
        F: FnMut(&mut String, &mut Vec<ContentItem>),
    {
        if self.poetry_mode && self.pending_poetry_line {
            flush_text(text_buffer, content_items);
            content_items.push(ContentItem {
                kind: "poetry_line".into(),
                text: None,
                number: self.current_indent,
                id: None,
                display: None,
            });
            self.pending_poetry_line = false;
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ReferenceCitation {
    book: String,
    verses: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReferenceBlock {
    display: String,
    citations: Vec<ReferenceCitation>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Footnote {
    id: String,
    text: String,
    references: HashMap<String, ReferenceBlock>,
}

fn table_element_to_markdown(table_el: ElementRef) -> String {
    let tr_sel = Selector::parse("tr").unwrap();
    let cell_sel = Selector::parse("th, td").unwrap();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for tr in table_el.select(&tr_sel) {
        let mut row: Vec<String> = Vec::new();
        for cell in tr.select(&cell_sel) {
            let text = cell.text().collect::<String>().trim().to_string();
            row.push(text);
        }
        if !row.is_empty() {
            rows.push(row);
        }
    }

    if rows.is_empty() { return String::new(); }

    // Determine column count from the widest row
    let cols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 { return String::new(); }

    // Build Markdown table: use first row as header, then a separator, then the rest
    let mut out = String::new();
    let header = &rows[0];
    let mut header_line = String::new();
    for (i, cell) in header.iter().enumerate() {
        if i == 0 { header_line.push('|'); }
        header_line.push(' ');
        header_line.push_str(cell);
        header_line.push(' ');
        header_line.push('|');
    }
    out.push_str(&header_line);
    out.push('\n');

    let mut sep = String::new();
    for i in 0..cols {
        if i == 0 { sep.push('|'); }
        sep.push(' ');
        sep.push_str("---");
        sep.push(' ');
        sep.push('|');
    }
    out.push_str(&sep);
    out.push('\n');

    for row in rows.iter().skip(1) {
        let mut line = String::new();
        for i in 0..cols {
            if i == 0 { line.push('|'); }
            line.push(' ');
            if let Some(cell) = row.get(i) { line.push_str(cell); }
            line.push(' ');
            line.push('|');
        }
        out.push_str(&line);
        out.push('\n');
    }

    out.trim_end().to_string()
}

#[derive(Debug, Serialize, Deserialize)]
struct CrossRefLink {
    book: String,
    chapter: u32,
    // If a single verse, set `verse`; if a range, set `verse_start` and `verse_end`.
    #[serde(skip_serializing_if = "Option::is_none")]
    verse: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verse_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verse_end: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CrossRef {
    id: String,
    book: String,
    chapter: u32,
    verse: u32,
    links: Vec<CrossRefLink>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChapterData {
    translation_code: String,
    translation_name: String,
    book_code: String,
    book_name: String,
    chapter: u32,
    verses: Vec<Verse>,
    footnotes: Vec<Footnote>,
    #[serde(rename = "cross_refs")]
    crossrefs: Vec<CrossRef>,
}

fn extract_book_name_from_meta(document: &Html) -> Option<String> {
    let selector = Selector::parse(r#"meta[property="og:title"]"#).unwrap();

    if let Some(meta) = document.select(&selector).next() {
        if let Some(content) = meta.value().attr("content") {
            // Remove "Bible Gateway passage: " and chapter info
            let re = Regex::new(r"^Bible Gateway passage:\s*(.+?)\s+\d+\s*-\s*.+$").unwrap();
            if let Some(caps) = re.captures(content) {
                return Some(caps[1].to_string());
            }
        }
    }

    None
}

fn extract_translation_name(document: &Html) -> Option<(String, String)> {
    let selector = Selector::parse(r#"option[selected="selected"]"#).unwrap();

    for option in document.select(&selector) {
        if let Some(value) = option.value().attr("value") {
            let text = option.text().collect::<String>();
            // Remove the parenthetical ID
            let re = Regex::new(r"^(.+?)\s*\([^)]+\)\s*$").unwrap();
            let name = if let Some(caps) = re.captures(&text) {
                caps[1].to_string()
            } else {
                text.trim().to_string()
            };

            return Some((value.to_string(), name));
        }
    }

    None
}

fn parse_indent_from_classes(classes: &[&str]) -> Option<u32> {
    let mut indent: Option<u32> = None;
    for class in classes {
        let value = match *class {
            "indent-1" | "q1" => Some(1),
            "indent-2" | "q2" => Some(2),
            "indent-3" | "q3" => Some(3),
            "indent-4" | "q4" => Some(4),
            _ => class
                .strip_prefix("indent-")
                .and_then(|n| n.parse::<u32>().ok()),
        };

        if let Some(val) = value {
            indent = Some(indent.map(|current| current.max(val)).unwrap_or(val));
        }
    }
    indent
}

// Helper function to walk the DOM tree in document order
fn walk_dom_for_content<F>(
    node: ElementRef,
    content_items: &mut Vec<ContentItem>,
    text_buffer: &mut String,
    flush_text: &mut F,
    book_id: &str,
    chapter: u32,
    verse: u32,
    context: &mut WalkContext,
) where
    F: FnMut(&mut String, &mut Vec<ContentItem>),
{
    let element = node.value();
    let tag_name = element.name();
    let classes: Vec<_> = element.classes().collect();
    let mut pushed_indent = false;

    if let Some(indent) = parse_indent_from_classes(&classes) {
        context.push_indent(indent);
        pushed_indent = true;
    }

    match tag_name {
        "br" => {
            if context.poetry_mode {
                flush_text(text_buffer, content_items);
                context.pending_poetry_line = true;
            } else {
                text_buffer.push(' ');
            }
        }
        "div" if classes.contains(&"poetry") => {
            flush_text(text_buffer, content_items);
            for child in node.children() {
                if let Some(elem_ref) = ElementRef::wrap(child) {
                    walk_dom_for_content(
                        elem_ref,
                        content_items,
                        text_buffer,
                        flush_text,
                        book_id,
                        chapter,
                        verse,
                        context,
                    );
                }
            }
        }
        "p" if classes.contains(&"line") => {
            flush_text(text_buffer, content_items);
            let previous_poetry = context.poetry_mode;
            let previous_pending = context.pending_poetry_line;
            context.poetry_mode = true;
            context.pending_poetry_line = true;

            for child in node.children() {
                if let Some(elem_ref) = ElementRef::wrap(child) {
                    walk_dom_for_content(
                        elem_ref,
                        content_items,
                        text_buffer,
                        flush_text,
                        book_id,
                        chapter,
                        verse,
                        context,
                    );
                } else if let Some(text) = child.value().as_text() {
                    let t = text.to_string();
                    if t == "[" || t == "]" || t == "(" || t == ")" {
                        continue;
                    }
                    if !t.trim().is_empty() {
                        context.ensure_poetry_line_started(text_buffer, content_items, flush_text);
                        text_buffer.push_str(&t);
                    }
                }
            }

            context.poetry_mode = previous_poetry;
            context.pending_poetry_line = previous_pending;
        }
        "p" => {
            flush_text(text_buffer, content_items);
            for child in node.children() {
                if let Some(elem_ref) = ElementRef::wrap(child) {
                    walk_dom_for_content(
                        elem_ref,
                        content_items,
                        text_buffer,
                        flush_text,
                        book_id,
                        chapter,
                        verse,
                        context,
                    );
                } else if let Some(text) = child.value().as_text() {
                    let t = text.to_string();
                    if t == "[" || t == "]" || t == "(" || t == ")" {
                        continue;
                    }
                    if !t.trim().is_empty() {
                        text_buffer.push_str(&t);
                    }
                }
            }
            flush_text(text_buffer, content_items);
            text_buffer.push_str("\n\n");
        }
        "table" => {
            flush_text(text_buffer, content_items);
            let table_markdown = table_element_to_markdown(node);
            if !table_markdown.is_empty() {
                content_items.push(ContentItem {
                    kind: "text".into(),
                    text: Some(format!("\n\n{}\n\n", table_markdown)),
                    number: None,
                    id: None,
                    display: None,
                });
            }
        }
        "b" if classes.contains(&"inline-h3") => {
            flush_text(text_buffer, content_items);
            let text = node.text().collect::<String>().trim().to_string();
            if !text.is_empty() {
                content_items.push(ContentItem {
                    kind: "inline_h3".into(),
                    text: Some(text),
                    number: None,
                    id: None,
                    display: None,
                });
            }
        }
        "span" if classes.contains(&"chapternum") => {}
        "sup" if classes.contains(&"versenum") => {
            flush_text(text_buffer, content_items);
            let already_has_verse_number =
                content_items.iter().any(|ci| ci.kind == "verse_number");
            if !already_has_verse_number {
                content_items.push(ContentItem {
                    kind: "verse_number".into(),
                    text: None,
                    number: Some(verse),
                    id: None,
                    display: None,
                });
            }
        }
        "sup" if classes.contains(&"footnote") => {
            context.ensure_poetry_line_started(text_buffer, content_items, flush_text);
            flush_text(text_buffer, content_items);
            if let Some(a_el) = node.select(&Selector::parse("a").unwrap()).next() {
                let marker = a_el.text().collect::<String>().trim().to_string();
                if !marker.is_empty() {
                    let id = format!("{}:{}:{}-{}", book_id, chapter, verse, marker);
                    content_items.push(ContentItem {
                        kind: "footnote".into(),
                        text: Some(marker),
                        number: None,
                        id: Some(id),
                        display: None,
                    });
                }
            }
        }
        "sup" if classes.contains(&"crossreference") => {
            context.ensure_poetry_line_started(text_buffer, content_items, flush_text);
            flush_text(text_buffer, content_items);
            if let Some(a_el) = node.select(&Selector::parse("a").unwrap()).next() {
                let marker = a_el.text().collect::<String>().trim().to_string();
                if !marker.is_empty() {
                    let id = format!("{}:{}:{}-{}", book_id, chapter, verse, marker);
                    content_items.push(ContentItem {
                        kind: "crossref".into(),
                        text: Some(marker),
                        number: None,
                        id: Some(id),
                        display: None,
                    });
                }
            }
        }
        _ => {
            for child in node.children() {
                if let Some(elem_ref) = ElementRef::wrap(child) {
                    walk_dom_for_content(
                        elem_ref,
                        content_items,
                        text_buffer,
                        flush_text,
                        book_id,
                        chapter,
                        verse,
                        context,
                    );
                } else if let Some(text) = child.value().as_text() {
                    let t = text.to_string();
                    if t == "[" || t == "]" || t == "(" || t == ")" {
                        continue;
                    }
                    if !t.trim().is_empty() {
                        context.ensure_poetry_line_started(text_buffer, content_items, flush_text);
                        text_buffer.push_str(&t);
                    }
                }
            }
        }
    }

    if pushed_indent {
        context.pop_indent();
    }
}

fn tokenize_greek_text(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current_word = String::new();

    for ch in text.chars() {
        if ch.is_whitespace() {
            if !current_word.is_empty() {
                words.push(current_word.clone());
                current_word.clear();
            }
        } else {
            current_word.push(ch);
        }
    }

    if !current_word.is_empty() {
        words.push(current_word);
    }

    words
}

fn parse_header_with_crossrefs(header_text: &str, book_id: &str, chapter: u32, verse: u32) -> Vec<ContentItem> {
    if header_text.is_empty() {
        return Vec::new();
    }

    let mut items = Vec::new();
    let crossref_re = Regex::new(r"\(([A-Z])\)").unwrap();

    let mut last_end = 0;
    let matches: Vec<_> = crossref_re.find_iter(header_text).collect();

    for m in matches.iter() {
        // Add text before this cross-ref
        if m.start() > last_end {
            let text = header_text[last_end..m.start()].to_string();
            if !text.is_empty() {
                items.push(ContentItem {
                    kind: "text".into(),
                    text: Some(text),
                    number: None,
                    id: None,
                    display: None,
                });
            }
        }

        // Extract the letter from (A), (B), etc.
        let letter = &header_text[m.start()+1..m.end()-1];

        // Add cross-ref item
        items.push(ContentItem {
            kind: "crossref".into(),
            text: Some(letter.to_string()),
            number: None,
            id: Some(format!("{}:{}:{}-{}", book_id, chapter, verse, letter)),
            display: None,
        });

        last_end = m.end();
    }

    // Add remaining text after last cross-ref
    if last_end < header_text.len() {
        let text = header_text[last_end..].to_string();
        if !text.is_empty() {
            items.push(ContentItem {
                kind: "text".into(),
                text: Some(text),
                number: None,
                id: None,
                display: None,
            });
        }
    }

    // If no cross-refs were found, just return the text as-is
    if items.is_empty() {
        items.push(ContentItem {
            kind: "text".into(),
            text: Some(header_text.to_string()),
            number: None,
            id: None,
            display: None,
        });
    }

    items
}

fn extract_verses(document: &Html, book_id: &str, translation_id: &str, chapter: u32) -> Vec<Verse> {
    let mut verses = Vec::new();
    let passage_selector = Selector::parse("div.passage-content").unwrap();
    let inline_header_selector = Selector::parse("b.inline-h3").unwrap();
    let verse_number_selector = Selector::parse("sup.versenum").unwrap();

    let mut current_header = String::new();
    let mut current_section = String::new();

    if let Some(passage_content) = document.select(&passage_selector).next() {
        // First pass: collect all verse spans with metadata across all paragraphs/poetry blocks
        let verse_pattern = Regex::new(&format!(r"{}-(\d+)-(\d+)", regex::escape(book_id))).unwrap();
        let all_spans_selector = Selector::parse("span").unwrap();

        struct SpanInfo<'a> {
            verse_num: u32,
            span: ElementRef<'a>,
            poetry_indent: Option<u32>,
            is_poetry_line: bool,
            is_first_in_paragraph: bool,
            header: String,
            section: String,
        }

        let mut all_verse_spans: Vec<SpanInfo> = Vec::new();

        for elem in passage_content.select(&Selector::parse("h2, h3, p:not(.line), div.poetry").unwrap()) {
            let elem_name = elem.value().name();

            if elem_name == "h2" {
                let span_selector = Selector::parse("span.text").unwrap();
                if let Some(span) = elem.select(&span_selector).next() {
                    current_section = span.text().collect::<String>().trim().to_string();
                }
            }
            else if elem_name == "h3" {
                let span_selector = Selector::parse("span.text").unwrap();
                if let Some(span) = elem.select(&span_selector).next() {
                    current_header = span.text().collect::<String>().trim().to_string();
                }
            }
            else if elem_name == "p" || elem_name == "div" {
                let class_list: Vec<_> = elem.value().classes().collect();
                let poetry_indent = if class_list.contains(&"q1") { Some(1) }
                else if class_list.contains(&"q2") { Some(2) }
                else if class_list.contains(&"q3") { Some(3) }
                else { None };

                let mut first_in_paragraph = true;

                for span in elem.select(&all_spans_selector) {
                    let mut verse_num: Option<u32> = None;
                    let mut span_chapter: Option<u32> = None;

                    for class in span.value().classes() {
                        if let Some(caps) = verse_pattern.captures(class) {
                            span_chapter = caps[1].parse::<u32>().ok();
                            verse_num = caps[2].parse::<u32>().ok();
                            break;
                        }
                    }

                    if verse_num.is_none() { continue; }
                    if span_chapter != Some(chapter) { continue; }

                    let mut verse_num = verse_num.unwrap();
                    let has_inline_header = span.select(&inline_header_selector).next().is_some();
                    let has_verse_number = span.select(&verse_number_selector).next().is_some();
                    if has_inline_header && !has_verse_number {
                        verse_num += 1;
                    }

                    let mut verse_poetry_indent: Option<u32> = None;
                    let mut is_poetry_line = false;

                    for ancestor in span.ancestors() {
                        if let Some(elem_ref) = ElementRef::wrap(ancestor) {
                            let classes: Vec<_> = elem_ref.value().classes().collect();
                            let tag_name = elem_ref.value().name();

                            if (tag_name == "p" && classes.contains(&"line"))
                                || (tag_name == "div" && classes.contains(&"poetry"))
                            {
                                is_poetry_line = true;
                            }

                            for class in &classes {
                                if *class == "indent-1" || *class == "q1" {
                                    verse_poetry_indent = Some(verse_poetry_indent.unwrap_or(0).max(1));
                                } else if *class == "indent-2" || *class == "q2" {
                                    verse_poetry_indent = Some(verse_poetry_indent.unwrap_or(0).max(2));
                                } else if *class == "indent-3" || *class == "q3" {
                                    verse_poetry_indent = Some(verse_poetry_indent.unwrap_or(0).max(3));
                                } else if *class == "indent-4" || *class == "q4" {
                                    verse_poetry_indent = Some(verse_poetry_indent.unwrap_or(0).max(4));
                                } else if let Some(num) = class.strip_prefix("indent-").and_then(|n| n.parse::<u32>().ok()) {
                                    verse_poetry_indent = Some(verse_poetry_indent.unwrap_or(0).max(num));
                                }
                            }
                        }
                    }

                    let final_poetry_indent = verse_poetry_indent.or(poetry_indent);

                    all_verse_spans.push(SpanInfo {
                        verse_num,
                        span,
                        poetry_indent: final_poetry_indent,
                        is_poetry_line,
                        is_first_in_paragraph: first_in_paragraph,
                        header: if first_in_paragraph { current_header.clone() } else { String::new() },
                        section: if first_in_paragraph { current_section.clone() } else { String::new() },
                    });

                    if first_in_paragraph {
                        current_header.clear();
                        first_in_paragraph = false;
                    }
                }
            }
        }

        // Second pass: group spans by verse number and process them
        let mut current_verse: Option<u32> = None;
        let mut combined_content: Vec<ContentItem> = Vec::new();
        let mut combined_text_buffer = String::new();
        let mut verse_header: Vec<ContentItem> = Vec::new();
        let mut verse_section = String::new();
        let mut verse_paragraph_start = false;
        let mut verse_indent: Option<u32> = None;

        let ensure_verse_number = |items: &mut Vec<ContentItem>, verse_number: u32| {
            if !items.iter().any(|ci| ci.kind == "verse_number") {
                items.insert(0, ContentItem {
                    kind: "verse_number".into(),
                    text: None,
                    number: Some(verse_number),
                    id: None,
                    display: None,
                });
            }
        };

        for span_info in all_verse_spans {
            let verse_num = span_info.verse_num;

            if current_verse.is_none() {
                current_verse = Some(verse_num);
                if !span_info.header.is_empty() {
                    verse_header = parse_header_with_crossrefs(&span_info.header, book_id, chapter, verse_num);
                }
                verse_section = span_info.section;
                verse_paragraph_start = span_info.is_first_in_paragraph;
                verse_indent = span_info.poetry_indent;
            } else if current_verse != Some(verse_num) {
                // Flush current verse
                let flush_text = |buffer: &mut String, items: &mut Vec<ContentItem>| {
                    let trimmed = buffer.trim().to_string();
                    if !trimmed.is_empty() {
                        if translation_id == "SBLGNT" || translation_id == "TR1894" || translation_id == "WLC" {
                            for word in tokenize_greek_text(&trimmed) {
                                items.push(ContentItem {
                                    kind: "surface".into(),
                                    text: Some(word),
                                    number: None,
                                    id: None,
                                    display: None,
                                });
                            }
                        } else {
                            items.push(ContentItem {
                                kind: "text".into(),
                                text: Some(trimmed),
                                number: None,
                                id: None,
                                display: None,
                            });
                        }
                        buffer.clear();
                    }
                };
                flush_text(&mut combined_text_buffer, &mut combined_content);

                if !combined_content.is_empty() {
                    ensure_verse_number(&mut combined_content, current_verse.unwrap());
                    verses.push(Verse {
                        chapter: chapter,
                        verse: current_verse.unwrap(),
                        content: combined_content.clone(),
                        header: verse_header.clone(),
                        section: verse_section.clone(),
                        paragraph_start: verse_paragraph_start,
                        poetry_indent: verse_indent,
                    });
                }

                // Start new verse
                current_verse = Some(verse_num);
                combined_content.clear();
                combined_text_buffer.clear();
                if !span_info.header.is_empty() {
                    verse_header = parse_header_with_crossrefs(&span_info.header, book_id, chapter, verse_num);
                } else {
                    verse_header = Vec::new();
                }
                verse_section = span_info.section;
                verse_paragraph_start = span_info.is_first_in_paragraph;
                verse_indent = span_info.poetry_indent;
            }

            // Process the span content
            let mut flush_text = |buffer: &mut String, items: &mut Vec<ContentItem>| {
                let mut trimmed = buffer.trim().to_string();
                if !trimmed.is_empty() {
                    if items.is_empty() {
                        let s = trimmed.replace('\u{a0}', " ");
                        let s_trimmed = s.trim_start();
                        if let Some(space_pos) = s_trimmed.find(' ') {
                            let (num_str, rest) = s_trimmed.split_at(space_pos);
                            if let Ok(n) = num_str.parse::<u32>() {
                                if n == verse_num {
                                    trimmed = rest.trim_start().to_string();
                                }
                            }
                        }
                    }
                    if !trimmed.is_empty() {
                        if translation_id == "SBLGNT" || translation_id == "TR1894" || translation_id == "WLC" {
                            for word in tokenize_greek_text(&trimmed) {
                                items.push(ContentItem {
                                    kind: "surface".into(),
                                    text: Some(word),
                                    number: None,
                                    id: None,
                                    display: None,
                                });
                            }
                        } else {
                            items.push(ContentItem {
                                kind: "text".into(),
                                text: Some(trimmed),
                                number: None,
                                id: None,
                                display: None,
                            });
                        }
                        buffer.clear();
                    }
                }
            };

            let mut walk_context = WalkContext::default();
            walk_context.current_indent = span_info.poetry_indent;
            if span_info.is_poetry_line {
                walk_context.poetry_mode = true;
                walk_context.pending_poetry_line = true;
            }
            walk_dom_for_content(
                span_info.span,
                &mut combined_content,
                &mut combined_text_buffer,
                &mut flush_text,
                book_id,
                chapter,
                verse_num,
                &mut walk_context,
            );
        }

        // Flush final verse
        if current_verse.is_some() {
            let flush_text = |buffer: &mut String, items: &mut Vec<ContentItem>| {
                let trimmed = buffer.trim().to_string();
                if !trimmed.is_empty() {
                    if translation_id == "SBLGNT" || translation_id == "TR1894" || translation_id == "WLC" {
                        for word in tokenize_greek_text(&trimmed) {
                            items.push(ContentItem {
                                kind: "surface".into(),
                                text: Some(word),
                                number: None,
                                id: None,
                                display: None,
                            });
                        }
                    } else {
                        items.push(ContentItem {
                            kind: "text".into(),
                            text: Some(trimmed),
                            number: None,
                            id: None,
                            display: None,
                        });
                    }
                    buffer.clear();
                }
            };
            flush_text(&mut combined_text_buffer, &mut combined_content);

            if !combined_content.is_empty() {
                ensure_verse_number(&mut combined_content, current_verse.unwrap());
                verses.push(Verse {
                    chapter: chapter,
                    verse: current_verse.unwrap(),
                    content: combined_content,
                    header: verse_header,
                    section: verse_section,
                    paragraph_start: verse_paragraph_start,
                    poetry_indent: verse_indent,
                });
            }
        }
    }

    verses
}

fn get_abbreviation_map() -> HashMap<&'static str, (&'static str, &'static str)> {
    let mut map = HashMap::new();

    // Old Testament
    map.insert("Ge", ("Gen", "Genesis"));
    map.insert("Gen", ("Gen", "Genesis"));
    map.insert("Gn", ("Gen", "Genesis"));
    map.insert("Genesis", ("Gen", "Genesis"));

    map.insert("Ex", ("Exod", "Exodus"));
    map.insert("Exod", ("Exod", "Exodus"));
    map.insert("Exodus", ("Exod", "Exodus"));

    map.insert("Lev", ("Lev", "Leviticus"));
    map.insert("Lv", ("Lev", "Leviticus"));
    map.insert("Leviticus", ("Lev", "Leviticus"));

    map.insert("Nm", ("Num", "Numbers"));
    map.insert("Nu", ("Num", "Numbers"));
    map.insert("Num", ("Num", "Numbers"));
    map.insert("Numbers", ("Num", "Numbers"));

    map.insert("Deut", ("Deut", "Deuteronomy"));
    map.insert("Dt", ("Deut", "Deuteronomy"));
    map.insert("Deuteronomy", ("Deut", "Deuteronomy"));

    map.insert("Jos", ("Josh", "Joshua"));
    map.insert("Josh", ("Josh", "Joshua"));
    map.insert("Joshua", ("Josh", "Joshua"));

    map.insert("Jdg", ("Judg", "Judges"));
    map.insert("Jgs", ("Judg", "Judges"));
    map.insert("Judg", ("Judg", "Judges"));
    map.insert("Judges", ("Judg", "Judges"));

    map.insert("Ru", ("Ruth", "Ruth"));
    map.insert("Ruth", ("Ruth", "Ruth"));

    map.insert("1Sa", ("1Sam", "1 Samuel"));
    map.insert("1Sam", ("1Sam", "1 Samuel"));
    map.insert("1Sm", ("1Sam", "1 Samuel"));
    map.insert("1Samuel", ("1Sam", "1 Samuel"));

    map.insert("2Sa", ("2Sam", "2 Samuel"));
    map.insert("2Sam", ("2Sam", "2 Samuel"));
    map.insert("2Sm", ("2Sam", "2 Samuel"));
    map.insert("2Samuel", ("2Sam", "2 Samuel"));

    map.insert("1Kg", ("1Kgs", "1 Kings"));
    map.insert("1Kgs", ("1Kgs", "1 Kings"));
    map.insert("1Ki", ("1Kgs", "1 Kings"));
    map.insert("1Kings", ("1Kgs", "1 Kings"));

    map.insert("2Kg", ("2Kgs", "2 Kings"));
    map.insert("2Kgs", ("2Kgs", "2 Kings"));
    map.insert("2Ki", ("2Kgs", "2 Kings"));
    map.insert("2Kings", ("2Kgs", "2 Kings"));

    map.insert("1Ch", ("1Chr", "1 Chronicles"));
    map.insert("1Chr", ("1Chr", "1 Chronicles"));
    map.insert("1Chron", ("1Chr", "1 Chronicles"));
    map.insert("1Chronicles", ("1Chr", "1 Chronicles"));

    map.insert("2Ch", ("2Chr", "2 Chronicles"));
    map.insert("2Chr", ("2Chr", "2 Chronicles"));
    map.insert("2Chronicles", ("2Chr", "2 Chronicles"));

    map.insert("Ezr", ("Ezra", "Ezra"));
    map.insert("Ezra", ("Ezra", "Ezra"));

    map.insert("Ne", ("Neh", "Nehemiah"));
    map.insert("Neh", ("Neh", "Nehemiah"));
    map.insert("Nehemiah", ("Neh", "Nehemiah"));

    map.insert("Est", ("Esth", "Esther"));
    map.insert("Esth", ("Esth", "Esther"));
    map.insert("Esther", ("Esth", "Esther"));

    map.insert("Jb", ("Job", "Job"));
    map.insert("Job", ("Job", "Job"));

    map.insert("Ps", ("Ps", "Psalms"));
    map.insert("Psalm", ("Ps", "Psalms"));
    map.insert("Psalms", ("Ps", "Psalms"));

    map.insert("Pr", ("Prov", "Proverbs"));
    map.insert("Prov", ("Prov", "Proverbs"));
    map.insert("Prv", ("Prov", "Proverbs"));
    map.insert("Proverbs", ("Prov", "Proverbs"));

    map.insert("Ec", ("Eccl", "Ecclesiastes"));
    map.insert("Ecc", ("Eccl", "Ecclesiastes"));
    map.insert("Eccl", ("Eccl", "Ecclesiastes"));
    map.insert("Ecclesiastes", ("Eccl", "Ecclesiastes"));

    map.insert("Sg", ("Song", "Song of Solomon"));
    map.insert("Song", ("Song", "Song of Solomon"));
    map.insert("SS", ("Song", "Song of Solomon"));
    map.insert("SongofSolomon", ("Song", "Song of Solomon"));

    map.insert("Is", ("Isa", "Isaiah"));
    map.insert("Isa", ("Isa", "Isaiah"));
    map.insert("Isaiah", ("Isa", "Isaiah"));

    map.insert("Jer", ("Jer", "Jeremiah"));
    map.insert("Jr", ("Jer", "Jeremiah"));
    map.insert("Jeremiah", ("Jer", "Jeremiah"));

    map.insert("La", ("Lam", "Lamentations"));
    map.insert("Lam", ("Lam", "Lamentations"));
    map.insert("Lm", ("Lam", "Lamentations"));
    map.insert("Lamentations", ("Lam", "Lamentations"));

    map.insert("Ez", ("Ezek", "Ezekiel"));
    map.insert("Eze", ("Ezek", "Ezekiel"));
    map.insert("Ezek", ("Ezek", "Ezekiel"));
    map.insert("Ezk", ("Ezek", "Ezekiel"));
    map.insert("Ezekiel", ("Ezek", "Ezekiel"));

    map.insert("Da", ("Dan", "Daniel"));
    map.insert("Dan", ("Dan", "Daniel"));
    map.insert("Dn", ("Dan", "Daniel"));
    map.insert("Daniel", ("Dan", "Daniel"));

    map.insert("Hos", ("Hos", "Hosea"));
    map.insert("Hs", ("Hos", "Hosea"));
    map.insert("Hosea", ("Hos", "Hosea"));

    map.insert("Joe", ("Joel", "Joel"));
    map.insert("Jl", ("Joel", "Joel"));
    map.insert("Joel", ("Joel", "Joel"));

    map.insert("Am", ("Amos", "Amos"));
    map.insert("Amos", ("Amos", "Amos"));

    map.insert("Ob", ("Obad", "Obadiah"));
    map.insert("Obad", ("Obad", "Obadiah"));
    map.insert("Obadiah", ("Obad", "Obadiah"));

    map.insert("Jon", ("Jonah", "Jonah"));
    map.insert("Jnh", ("Jonah", "Jonah"));
    map.insert("Jonah", ("Jonah", "Jonah"));

    map.insert("Mc", ("Mic", "Micah"));
    map.insert("Mi", ("Mic", "Micah"));
    map.insert("Mic", ("Mic", "Micah"));
    map.insert("Micah", ("Mic", "Micah"));

    map.insert("Na", ("Nah", "Nahum"));
    map.insert("Nah", ("Nah", "Nahum"));
    map.insert("Nahum", ("Nah", "Nahum"));

    map.insert("Hab", ("Hab", "Habakkuk"));
    map.insert("Hb", ("Hab", "Habakkuk"));
    map.insert("Habakkuk", ("Hab", "Habakkuk"));

    map.insert("Zep", ("Zeph", "Zephaniah"));
    map.insert("Zeph", ("Zeph", "Zephaniah"));
    map.insert("Zph", ("Zeph", "Zephaniah"));
    map.insert("Zephaniah", ("Zeph", "Zephaniah"));

    map.insert("Hag", ("Hag", "Haggai"));
    map.insert("Hg", ("Hag", "Haggai"));
    map.insert("Haggai", ("Hag", "Haggai"));

    map.insert("Zc", ("Zech", "Zechariah"));
    map.insert("Zch", ("Zech", "Zechariah"));
    map.insert("Zec", ("Zech", "Zechariah"));
    map.insert("Zech", ("Zech", "Zechariah"));
    map.insert("Zechariah", ("Zech", "Zechariah"));

    map.insert("Mal", ("Mal", "Malachi"));
    map.insert("Malachi", ("Mal", "Malachi"));

    // Deuterocanonical/Apocrypha
    map.insert("Tb", ("Tob", "Tobit"));
    map.insert("Tob", ("Tob", "Tobit"));
    map.insert("Tobit", ("Tob", "Tobit"));

    map.insert("Jdt", ("Jdt", "Judith"));
    map.insert("Judith", ("Jdt", "Judith"));

    map.insert("Wis", ("Wis", "Wisdom of Solomon"));
    map.insert("WisdomofSolomon", ("Wis", "Wisdom of Solomon"));

    map.insert("Sir", ("Sir", "Sirach"));
    map.insert("Sirach", ("Sir", "Sirach"));

    map.insert("Bar", ("Bar", "Baruch"));
    map.insert("Baruch", ("Bar", "Baruch"));

    map.insert("EpJer", ("EpJer", "Letter of Jeremiah"));
    map.insert("LetterofJeremiah", ("EpJer", "Letter of Jeremiah"));

    map.insert("1Macc", ("1Macc", "1 Maccabees"));
    map.insert("1Maccabees", ("1Macc", "1 Maccabees"));

    map.insert("2Macc", ("2Macc", "2 Maccabees"));
    map.insert("2Maccabees", ("2Macc", "2 Maccabees"));

    map.insert("PrMan", ("PrMan", "Prayer of Manasseh"));
    map.insert("PrayerofManasseh", ("PrMan", "Prayer of Manasseh"));

    map.insert("1Esd", ("1Esd", "1 Esdras"));
    map.insert("1Esdras", ("1Esd", "1 Esdras"));

    map.insert("2Esd", ("2Esd", "2 Esdras"));
    map.insert("2Esdras", ("2Esd", "2 Esdras"));

    map.insert("AddEsth", ("AddEsth", "Additions to Esther"));
    map.insert("AdditionstoEsther", ("AddEsth", "Additions to Esther"));

    map.insert("Sus", ("Sus", "Susanna"));
    map.insert("Susanna", ("Sus", "Susanna"));

    map.insert("Bel", ("Bel", "Bel and the Dragon"));
    map.insert("BelandtheDragon", ("Bel", "Bel and the Dragon"));

    // New Testament
    map.insert("Mt", ("Matt", "Matthew"));
    map.insert("Matt", ("Matt", "Matthew"));
    map.insert("Matthew", ("Matt", "Matthew"));

    map.insert("Mk", ("Mark", "Mark"));
    map.insert("Mark", ("Mark", "Mark"));

    map.insert("Lk", ("Luke", "Luke"));
    map.insert("Luke", ("Luke", "Luke"));

    map.insert("Jn", ("John", "John"));
    map.insert("John", ("John", "John"));

    map.insert("Ac", ("Acts", "Acts"));
    map.insert("Acts", ("Acts", "Acts"));

    map.insert("Rm", ("Rom", "Romans"));
    map.insert("Ro", ("Rom", "Romans"));
    map.insert("Rom", ("Rom", "Romans"));
    map.insert("Romans", ("Rom", "Romans"));

    map.insert("1Co", ("1Cor", "1 Corinthians"));
    map.insert("1Cor", ("1Cor", "1 Corinthians"));
    map.insert("1Corinthians", ("1Cor", "1 Corinthians"));

    map.insert("2Co", ("2Cor", "2 Corinthians"));
    map.insert("2Cor", ("2Cor", "2 Corinthians"));
    map.insert("2Corinthians", ("2Cor", "2 Corinthians"));

    map.insert("Gal", ("Gal", "Galatians"));
    map.insert("Gl", ("Gal", "Galatians"));
    map.insert("Galatians", ("Gal", "Galatians"));

    map.insert("Eph", ("Eph", "Ephesians"));
    map.insert("Ephesians", ("Eph", "Ephesians"));

    map.insert("Phil", ("Phil", "Philippians"));
    map.insert("Php", ("Phil", "Philippians"));
    map.insert("Philippians", ("Phil", "Philippians"));

    map.insert("Col", ("Col", "Colossians"));
    map.insert("Colossians", ("Col", "Colossians"));

    map.insert("1Th", ("1Thess", "1 Thessalonians"));
    map.insert("1Thess", ("1Thess", "1 Thessalonians"));
    map.insert("1Thessalonians", ("1Thess", "1 Thessalonians"));

    map.insert("2Th", ("2Thess", "2 Thessalonians"));
    map.insert("2Thess", ("2Thess", "2 Thessalonians"));
    map.insert("2Thessalonians", ("2Thess", "2 Thessalonians"));

    map.insert("1Ti", ("1Tim", "1 Timothy"));
    map.insert("1Tim", ("1Tim", "1 Timothy"));
    map.insert("1Tm", ("1Tim", "1 Timothy"));
    map.insert("1Timothy", ("1Tim", "1 Timothy"));

    map.insert("2Ti", ("2Tim", "2 Timothy"));
    map.insert("2Tim", ("2Tim", "2 Timothy"));
    map.insert("2Tm", ("2Tim", "2 Timothy"));
    map.insert("2Timothy", ("2Tim", "2 Timothy"));

    map.insert("Tit", ("Titus", "Titus"));
    map.insert("Titus", ("Titus", "Titus"));

    map.insert("Phm", ("Phlm", "Philemon"));
    map.insert("Phlm", ("Phlm", "Philemon"));
    map.insert("Philemon", ("Phlm", "Philemon"));

    map.insert("Heb", ("Heb", "Hebrews"));
    map.insert("Hebrews", ("Heb", "Hebrews"));

    map.insert("Jas", ("Jas", "James"));
    map.insert("Jms", ("Jas", "James"));
    map.insert("James", ("Jas", "James"));

    map.insert("1Pe", ("1Pet", "1 Peter"));
    map.insert("1Pet", ("1Pet", "1 Peter"));
    map.insert("1Pt", ("1Pet", "1 Peter"));
    map.insert("1Peter", ("1Pet", "1 Peter"));

    map.insert("2Pe", ("2Pet", "2 Peter"));
    map.insert("2Pet", ("2Pet", "2 Peter"));
    map.insert("2Pt", ("2Pet", "2 Peter"));
    map.insert("2Peter", ("2Pet", "2 Peter"));

    map.insert("1Jn", ("1John", "1 John"));
    map.insert("1John", ("1John", "1 John"));

    map.insert("2Jn", ("2John", "2 John"));
    map.insert("2John", ("2John", "2 John"));

    map.insert("3Jn", ("3John", "3 John"));
    map.insert("3John", ("3John", "3 John"));

    map.insert("Jude", ("Jude", "Jude"));

    map.insert("Rev", ("Rev", "Revelation"));
    map.insert("Rv", ("Rev", "Revelation"));
    map.insert("Revelation", ("Rev", "Revelation"));

    map
}

fn parse_book_abbreviation(abbr: &str) -> String {
    let map = get_abbreviation_map();
    if let Some((_, full_name)) = map.get(abbr) {
        full_name.to_string()
    } else {
        abbr.to_string()
    }
}

fn extract_footnotes_placeholders(document: &Html, book_name: &str, current_chapter: u32) -> Vec<Footnote> {
    let mut footnotes = Vec::new();
    let footnotes_selector = Selector::parse("div.footnotes ol li").unwrap();
    let text_selector = Selector::parse("span.footnote-text").unwrap();
    let link_selector = Selector::parse("a[href^='#en-']").unwrap();

    let abbr_map = get_abbreviation_map();
    let (ctx_short, _ctx_full) = abbr_map
        .get(book_name)
        .cloned()
        .unwrap_or((book_name, book_name));
    let ctx_short_s = ctx_short.to_string();

    for li in document.select(&footnotes_selector) {
        if let Some(text_span) = li.select(&text_selector).next() {
            // Read the leading anchor (e.g., "1:26")
            let link_text_opt = li
                .select(&link_selector)
                .next()
                .map(|a| a.text().collect::<String>().trim().to_string());

            let mut text_out = String::new();
            let mut references: HashMap<String, ReferenceBlock> = HashMap::new();
            let mut ref_counter: u32 = 1;

            // If we have a leading verse reference (like "1:26"), add it as REF1 at the start
            if let Some(ref link_text) = link_text_opt {
                let re_cv_simple = Regex::new(r"^(\d+):(\d+)$").unwrap();
                if let Some(c) = re_cv_simple.captures(link_text) {
                    let chap = c.get(1).unwrap().as_str();
                    let v = c.get(2).unwrap().as_str();
                    let verses = format!("{}:{}", chap, v);

                    let key = format!("REF{}", ref_counter);
                    ref_counter += 1;

                    references.insert(
                        key.clone(),
                        ReferenceBlock {
                            display: link_text.clone(),
                            citations: vec![ReferenceCitation {
                                book: ctx_short_s.clone(),
                                verses: verses.clone()
                            }]
                        },
                    );

                    text_out.push_str(&format!("{{{{{key}}}}} "));
                }
            }

            // State machine for parsing parentheses and citations
            let mut in_paren = false;
            let mut paren_display = String::new();
            let mut paren_citations: Vec<ReferenceCitation> = Vec::new();

            // Walk through all descendants of the text span
            for node in text_span.descendants() {
                if let Some(element) = ElementRef::wrap(node) {
                    let elem = element.value();

                    // Check for <a class="bibleref">
                    if elem.name() == "a" && elem.classes().any(|c| c == "bibleref") {
                        if let Some(data_ref) = elem.attr("data-bibleref") {
                            // Extract the display text
                            let display_text = element.text().collect::<String>();
                            if in_paren {
                                paren_display.push_str(&display_text);
                            } else {
                                text_out.push_str(&display_text);
                            }

                            // Parse data-bibleref attribute
                            let refs: Vec<&str> = data_ref.split('-').collect();

                            if refs.len() == 1 {
                                let parts: Vec<&str> = refs[0].split('.').collect();
                                if parts.len() == 3 {
                                    let b = parts[0];
                                    let ch = parts[1];
                                    let vs = parts[2];
                                    let (short, _) = abbr_map.get(b).cloned().unwrap_or((b, b));
                                    paren_citations.push(ReferenceCitation {
                                        book: short.to_string(),
                                        verses: format!("{}:{}", ch, vs)
                                    });
                                } else if parts.len() == 2 {
                                    let b = parts[0];
                                    let ch = parts[1];
                                    let (short, _) = abbr_map.get(b).cloned().unwrap_or((b, b));
                                    paren_citations.push(ReferenceCitation {
                                        book: short.to_string(),
                                        verses: ch.to_string()
                                    });
                                }
                            } else if refs.len() == 2 {
                                let start_parts: Vec<&str> = refs[0].split('.').collect();
                                let end_parts: Vec<&str> = refs[1].split('.').collect();

                                if start_parts.len() == 3 && end_parts.len() == 3 {
                                    let b = start_parts[0];
                                    let ch1 = start_parts[1];
                                    let vs1 = start_parts[2];
                                    let ch2 = end_parts[1];
                                    let vs2 = end_parts[2];
                                    let (short, _) = abbr_map.get(b).cloned().unwrap_or((b, b));

                                    if ch1 == ch2 {
                                        paren_citations.push(ReferenceCitation {
                                            book: short.to_string(),
                                            verses: format!("{}:{}-{}", ch1, vs1, vs2)
                                        });
                                    } else {
                                        paren_citations.push(ReferenceCitation {
                                            book: short.to_string(),
                                            verses: format!("{}:{}-{}:{}", ch1, vs1, ch2, vs2)
                                        });
                                    }
                                } else if start_parts.len() == 2 && end_parts.len() == 2 {
                                    let b = start_parts[0];
                                    let ch1 = start_parts[1];
                                    let ch2 = end_parts[1];
                                    let (short, _) = abbr_map.get(b).cloned().unwrap_or((b, b));
                                    paren_citations.push(ReferenceCitation {
                                        book: short.to_string(),
                                        verses: format!("{}-{}", ch1, ch2)
                                    });
                                }
                            }
                        }
                    }
                } else if let Some(text) = node.value().as_text() {
                    let t: &str = text.as_ref();

                    // Process character by character to handle parentheses
                    for ch in t.chars() {
                        if ch == '(' {
                            in_paren = true;
                            paren_display.clear();
                            paren_display.push(ch);
                            paren_citations.clear();
                        } else if ch == ')' && in_paren {
                            paren_display.push(ch);

                            // If no citations from anchors, try parsing the text
                            if paren_citations.is_empty() {
                                let text_only = paren_display.trim_matches(|c| c == '(' || c == ')');

                                // Try common patterns
                                let re_vv_comma = Regex::new(r"vv?\.?\s*(\d+)\s*,\s*(\d+)").unwrap();
                                let re_vv_range = Regex::new(r"vv?\.?\s*(\d+)\s*[–-]\s*(\d+)").unwrap();
                                let re_v_single = Regex::new(r"v\.?\s*(\d+)").unwrap();
                                let re_chaps = Regex::new(r"(?i)chaps?\.?\s*(\d+)\s*[–-]\s*(\d+)").unwrap();

                                if let Some(c) = re_vv_comma.captures(text_only) {
                                    let v1 = c.get(1).unwrap().as_str();
                                    let v2 = c.get(2).unwrap().as_str();
                                    paren_citations.push(ReferenceCitation {
                                        book: ctx_short_s.clone(),
                                        verses: format!("{}:{},{}", current_chapter, v1, v2)
                                    });
                                } else if let Some(c) = re_vv_range.captures(text_only) {
                                    let v1 = c.get(1).unwrap().as_str();
                                    let v2 = c.get(2).unwrap().as_str();
                                    paren_citations.push(ReferenceCitation {
                                        book: ctx_short_s.clone(),
                                        verses: format!("{}:{}-{}", current_chapter, v1, v2)
                                    });
                                } else if let Some(c) = re_v_single.captures(text_only) {
                                    let v = c.get(1).unwrap().as_str();
                                    paren_citations.push(ReferenceCitation {
                                        book: ctx_short_s.clone(),
                                        verses: format!("{}:{}", current_chapter, v)
                                    });
                                } else if let Some(c) = re_chaps.captures(text_only) {
                                    let ch1 = c.get(1).unwrap().as_str();
                                    let ch2 = c.get(2).unwrap().as_str();
                                    paren_citations.push(ReferenceCitation {
                                        book: ctx_short_s.clone(),
                                        verses: format!("{}-{}", ch1, ch2)
                                    });
                                }
                            }

                            // Create reference if we have citations
                            if !paren_citations.is_empty() {
                                let key = format!("REF{}", ref_counter);
                                ref_counter += 1;

                                // Build a normalized display from citations, de-duplicating entries
                                let mut seen: HashSet<String> = HashSet::new();
                                let mut parts: Vec<String> = Vec::new();
                                for cit in &paren_citations {
                                    let part = if cit.book == ctx_short_s {
                                        cit.verses.clone()
                                    } else {
                                        format!("{} {}", cit.book, cit.verses)
                                    };
                                    if seen.insert(part.clone()) {
                                        parts.push(part);
                                    }
                                }
                                let display = format!("({})", parts.join("; "));

                                references.insert(
                                    key.clone(),
                                    ReferenceBlock {
                                        display,
                                        citations: paren_citations.clone()
                                    },
                                );

                                text_out.push_str(&format!("{{{{{key}}}}}"));
                            } else {
                                text_out.push_str(&paren_display);
                            }

                            in_paren = false;
                            paren_display.clear();
                            paren_citations.clear();
                        } else {
                            if in_paren {
                                paren_display.push(ch);
                            } else {
                                text_out.push(ch);
                            }
                        }
                    }
                }
            }

            // If we ended with an open parenthesis (shouldn't happen but handle it)
            if in_paren {
                text_out.push_str(&paren_display);
            }

            // If there's an HTML table in this footnote, append it as Markdown below the text
            if let Some(table_el) = li.select(&Selector::parse("table").unwrap()).next() {
                // Build flattened table text to locate where it appeared in the composed text
                let tr_sel = Selector::parse("tr").unwrap();
                let cell_sel = Selector::parse("th, td").unwrap();
                let mut flat_no_sep = String::new();
                for tr in table_el.select(&tr_sel) {
                    for cell in tr.select(&cell_sel) {
                        let t = cell.text().collect::<String>().trim().to_string();
                        flat_no_sep.push_str(&t);
                    }
                }

                let md = table_element_to_markdown(table_el);
                if !md.is_empty() {
                    let md_block = format!("\n\n{}\n\n", md);
                    let mut inserted = false;
                    if !flat_no_sep.is_empty() {
                        if let Some(_) = text_out.find(&flat_no_sep) {
                            text_out = text_out.replacen(&flat_no_sep, &md_block, 1);
                            inserted = true;
                        }
                    }
                    if !inserted {
                        if !text_out.ends_with('\n') { text_out.push('\n'); }
                        text_out.push('\n');
                        text_out.push_str(&md);
                    }
                }
            }

            // Build footnote id in format: ShortBook:Chapter:Verse-marker (e.g., "Gen:1:1-a")
            let short_book = ctx_short_s.clone();

            // Prefer parsing verse + marker from the first anchor's href (e.g., "#en-NABRE-1A")
            let (mut href_verse_opt, mut href_marker_opt) = (None::<String>, None::<String>);
            if let Some(a) = li.select(&link_selector).next() {
                if let Some(href) = a.value().attr("href") {
                    // Capture trailing -<digits><letters> at end
                    let re_href = Regex::new(r"-(\d+)([A-Za-z]+)$").unwrap();
                    if let Some(c) = re_href.captures(href) {
                        href_verse_opt = Some(c.get(1).unwrap().as_str().to_string());
                        href_marker_opt = Some(c.get(2).unwrap().as_str().to_lowercase());
                    }
                }
            }

            // Fallback marker and verse from the li id attribute segment (e.g., last seg of id like "1A")
            let li_id = li.value().attr("id").unwrap_or("");
            let li_last_seg_opt = li_id.rsplit('-').next();
            let (li_verse_opt, li_marker_opt) = if let Some(seg) = li_last_seg_opt {
                let mut digits = String::new();
                let mut letters = String::new();
                for ch in seg.chars() {
                    if ch.is_ascii_digit() && letters.is_empty() { digits.push(ch); } else { letters.push(ch); }
                }
                (if digits.is_empty() { None } else { Some(digits) }, if letters.is_empty() { None } else { Some(letters.to_lowercase()) })
            } else { (None, None) };

            let marker_opt = href_marker_opt.clone().or(li_marker_opt.clone());

            // Determine chapter:verse — prefer explicit link text; support C:V, C:V–V and V-only
            let (v_chap, v_verse) = if let Some(ref link_text) = link_text_opt {
                let re_cv_simple = Regex::new(r"^(\d+):(\d+)$").unwrap();
                let re_cv_range_same_ch = Regex::new(r"^(\d+):(\d+)[–-](\d+)$").unwrap();
                let re_cv_range_cross_ch = Regex::new(r"^(\d+):(\d+)[–-](\d+):(\d+)$").unwrap();
                let re_v_only = Regex::new(r"^(\d+)$").unwrap();
                if let Some(c) = re_cv_simple.captures(link_text) {
                    (c.get(1).unwrap().as_str().to_string(), c.get(2).unwrap().as_str().to_string())
                } else if let Some(c) = re_cv_range_same_ch.captures(link_text) {
                    // Use the first verse of the range for the id
                    (c.get(1).unwrap().as_str().to_string(), c.get(2).unwrap().as_str().to_string())
                } else if let Some(c) = re_cv_range_cross_ch.captures(link_text) {
                    // Use the starting chapter and verse for the id
                    (c.get(1).unwrap().as_str().to_string(), c.get(2).unwrap().as_str().to_string())
                } else if let Some(c) = re_v_only.captures(link_text) {
                    (current_chapter.to_string(), c.get(1).unwrap().as_str().to_string())
                } else if let Some(v) = href_verse_opt.clone() {
                    (current_chapter.to_string(), v)
                } else if let Some(v) = li_verse_opt.clone() {
                    (current_chapter.to_string(), v)
                } else {
                    (current_chapter.to_string(), String::from("0"))
                }
            } else if let Some(v) = href_verse_opt.clone() {
                (current_chapter.to_string(), v)
            } else if let Some(v) = li_verse_opt.clone() {
                (current_chapter.to_string(), v)
            } else {
                (current_chapter.to_string(), String::from("0"))
            };

            let id = if let Some(ref marker) = marker_opt {
                format!("{}:{}:{}-{}", short_book, v_chap, v_verse, marker)
            } else {
                format!("{}:{}:{}", short_book, v_chap, v_verse)
            };

            footnotes.push(Footnote {
                id,
                text: text_out.trim().to_string(),
                references
            });
        }
    }

    footnotes
}

fn extract_crossrefs(document: &Html, book_name: &str, book_id: &str) -> Vec<CrossRef> {
    let mut crossrefs = Vec::new();
    let crossref_selector = Selector::parse("div.crossrefs ol li").unwrap();
    let link_selector = Selector::parse("a[href^='#en-']").unwrap();
    let crossref_link_selector = Selector::parse("a.crossref-link").unwrap();

    for li in document.select(&crossref_selector) {
        // Extract source reference from the first link (e.g., "11:1" or "Genesis 1:1" or "1:26–27")
        if let Some(source_link) = li.select(&link_selector).next() {
            let mut source_text = source_link.text().collect::<String>();
            // Normalize non-breaking space to regular space and trim
            source_text = source_text.replace('\u{00A0}', " ").trim().to_string();

            // Accept the following formats for the source reference text:
            //  - "11:1" (chapter:verse)
            //  - "Genesis 1:1" (optional book + chapter:verse)
            //  - "1:26–27" (chapter:verseStart–verseEnd) -> use verseStart for the id
            //  - "Genesis 1:26–27" -> use verseStart for the id
            // First try an optional-book + chapter:verse or chapter:verse–verseEnd
            let source_with_optional_book = Regex::new(
                r"^(?:[A-Za-z0-9 .]+\s+)?(\d+):(\d+)(?:[\u{2013}\u{2014}-](\d+))?$"
            ).unwrap();

            if let Some(caps) = source_with_optional_book.captures(&source_text) {
                let source_chapter = caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok()).unwrap_or(0);
                let source_verse = caps.get(2).and_then(|m| m.as_str().parse::<u32>().ok()).unwrap_or(0);

                let mut links = Vec::new();

                // Extract all crossref-link anchors with carryover of book/chapter across comma-separated refs
                // Maintain carryover across ALL anchors within this list item
                let mut current_book_name: String = book_name.to_string();
                let mut current_chapter: u32 = source_chapter;
                for crossref_link in li.select(&crossref_link_selector) {
                    // Check if this anchor has a data-bibleref attribute
                    if let Some(bibleref) = crossref_link.value().attr("data-bibleref") {
                        // Parse data-bibleref which can be comma-separated list:
                        // "Genesis 22:18, Galatians 3:16"
                        // "2 Samuel 7:12-2 Samuel 7:16, Isaiah 9:6, Isaiah 9:7"
                        for ref_part in bibleref.split(',') {
                            let ref_part = ref_part.trim();
                            if ref_part.is_empty() { continue; }

                            // Match patterns for different reference formats
                            let bibleref_range_re = Regex::new(r"^([A-Za-z0-9 ]+)\s+(\d+):(\d+)-([A-Za-z0-9 ]+)\s+(\d+):(\d+)$").unwrap();
                            let bibleref_simple_re = Regex::new(r"^([A-Za-z0-9 ]+)\s+(\d+):(\d+)-(\d+)$").unwrap();
                            let bibleref_single_re = Regex::new(r"^([A-Za-z0-9 ]+)\s+(\d+):(\d+)$").unwrap();

                            if let Some(caps) = bibleref_range_re.captures(ref_part) {
                                // Cross-chapter range: "Luke 3:23-Luke 3:38"
                                let book1 = caps[1].trim();
                                let ch1: u32 = caps[2].parse().unwrap_or(0);
                                let v1: u32 = caps[3].parse().unwrap_or(0);
                                // caps[4] is book2 (not used - we only use starting reference)
                                let ch2: u32 = caps[5].parse().unwrap_or(0);
                                let v2: u32 = caps[6].parse().unwrap_or(0);

                                let full_book = parse_book_abbreviation(&book1.replace(' ', ""));
                                if ch1 == ch2 {
                                    // Same chapter range
                                    links.push(CrossRefLink {
                                        book: full_book,
                                        chapter: ch1,
                                        verse: None,
                                        verse_start: Some(v1),
                                        verse_end: Some(v2)
                                    });
                                } else {
                                    // Cross-chapter - just use start verse
                                    links.push(CrossRefLink {
                                        book: full_book,
                                        chapter: ch1,
                                        verse: Some(v1),
                                        verse_start: None,
                                        verse_end: None
                                    });
                                }
                            } else if let Some(caps) = bibleref_simple_re.captures(ref_part) {
                                // Same chapter range: "Ruth 4:18-22"
                                let book = caps[1].trim();
                                let ch: u32 = caps[2].parse().unwrap_or(0);
                                let v1: u32 = caps[3].parse().unwrap_or(0);
                                let v2: u32 = caps[4].parse().unwrap_or(0);
                                let full_book = parse_book_abbreviation(&book.replace(' ', ""));
                                links.push(CrossRefLink {
                                    book: full_book,
                                    chapter: ch,
                                    verse: None,
                                    verse_start: Some(v1),
                                    verse_end: Some(v2)
                                });
                            } else if let Some(caps) = bibleref_single_re.captures(ref_part) {
                                // Single verse: "Luke 3:23"
                                let book = caps[1].trim();
                                let ch: u32 = caps[2].parse().unwrap_or(0);
                                let v: u32 = caps[3].parse().unwrap_or(0);
                                let full_book = parse_book_abbreviation(&book.replace(' ', ""));
                                links.push(CrossRefLink {
                                    book: full_book,
                                    chapter: ch,
                                    verse: Some(v),
                                    verse_start: None,
                                    verse_end: None
                                });
                            }
                        }
                        // Successfully parsed data-bibleref, continue to next anchor
                        continue;
                    }

                    // Fall back to parsing the text content if no data-bibleref
                    let ref_text = crossref_link.text().collect::<String>();
                    for group in ref_text.split(';') {

                        for token in group.split(',') {
                            let mut t = token.trim().trim_end_matches('.').to_string();
                            if t.is_empty() { continue; }
                            if t.starts_with("See ") { t = t[4..].to_string(); }
                            else if t.starts_with("S ") { t = t[2..].to_string(); }
                            t = Regex::new(r"(?i)^(?:ver\.?|verses?)\s+").unwrap().replace(&t, "").to_string();
                            t = t.replace('\u{A0}', " ");

                            // Some references include trailing markers like "*" or "†" after the verse
                            // number (e.g., "2Co 4:6*"), which break the matchers below. Strip any
                            // trailing non-alphanumeric characters from the token before parsing.
                            while t.chars().last().map(|c| !c.is_ascii_alphanumeric()).unwrap_or(false) {
                                t.pop();
                            }

                            let book_re = Regex::new(r"^([A-Za-z0-9]+(?:[\s\u{A0}]+[A-Za-z0-9]+)?)\s+(\d+):(\d+)(?:[\u{2013}\u{2014}-](\d+))?$").unwrap();
                            let chap_verse_re = Regex::new(r"^(\d+):(\d+)(?:[\u{2013}\u{2014}-](\d+))?$").unwrap();
                            let verse_only_re = Regex::new(r"^(\d+)(?:[\u{2013}\u{2014}-](\d+))?$").unwrap();

                            if let Some(caps) = book_re.captures(&t) {
                                let abbr = caps[1].to_string().replace('\u{A0}', "").replace(' ', "");
                                let chapter = caps[2].parse::<u32>().unwrap_or(0);
                                let v1 = caps[3].parse::<u32>().unwrap_or(0);
                                let v2 = caps.get(4).and_then(|m| m.as_str().parse::<u32>().ok());
                                let full = parse_book_abbreviation(&abbr);
                                current_book_name = full.clone();
                                current_chapter = chapter;
                                if let Some(endv) = v2 {
                                    links.push(CrossRefLink { book: current_book_name.clone(), chapter: current_chapter, verse: None, verse_start: Some(v1), verse_end: Some(endv) });
                                } else {
                                    links.push(CrossRefLink { book: current_book_name.clone(), chapter: current_chapter, verse: Some(v1), verse_start: None, verse_end: None });
                                }
                            } else if let Some(caps) = chap_verse_re.captures(&t) {
                                let chapter = caps[1].parse::<u32>().unwrap_or(0);
                                let v1 = caps[2].parse::<u32>().unwrap_or(0);
                                let v2 = caps.get(3).and_then(|m| m.as_str().parse::<u32>().ok());
                                current_chapter = chapter;
                                if let Some(endv) = v2 {
                                    links.push(CrossRefLink { book: current_book_name.clone(), chapter: current_chapter, verse: None, verse_start: Some(v1), verse_end: Some(endv) });
                                } else {
                                    links.push(CrossRefLink { book: current_book_name.clone(), chapter: current_chapter, verse: Some(v1), verse_start: None, verse_end: None });
                                }
                            } else if let Some(caps) = verse_only_re.captures(&t) {
                                let v1 = caps[1].parse::<u32>().unwrap_or(0);
                                let v2 = caps.get(2).and_then(|m| m.as_str().parse::<u32>().ok());
                                if let Some(endv) = v2 {
                                    links.push(CrossRefLink { book: current_book_name.clone(), chapter: current_chapter, verse: None, verse_start: Some(v1), verse_end: Some(endv) });
                                } else {
                                    links.push(CrossRefLink { book: current_book_name.clone(), chapter: current_chapter, verse: Some(v1), verse_start: None, verse_end: None });
                                }
                            }
                        }
                    }
                }

                // Fallback: if no anchors were parsed (some entries are plain text like "ver 8, 13, 19"),
                // parse simple numeric verse lists within the same book/chapter.
                if links.is_empty() {
                    let li_text = li.text().collect::<String>().replace('\u{00A0}', " ");
                    let simple_re = Regex::new(r"(?i)\b(?:S\s+|See\s+)?ver\.?(?:se|ses)?\s+([0-9 ,\-–]+)").unwrap();
                    if let Some(caps) = simple_re.captures(&li_text) {
                        if let Some(nums) = caps.get(1) {
                            for part in nums.as_str().split(',') {
                                let p = part.trim();
                                if p.is_empty() { continue; }
                                let range_re = Regex::new(r"^(\d+)\s*[\-–]\s*(\d+)$").unwrap();
                                if let Some(rc) = range_re.captures(p) {
                                    let start: u32 = rc[1].parse().unwrap_or(0);
                                    let end: u32 = rc[2].parse().unwrap_or(0);
                                    if start > 0 && end >= start {
                                        links.push(CrossRefLink { book: book_name.to_string(), chapter: source_chapter, verse: None, verse_start: Some(start), verse_end: Some(end) });
                                    }
                                } else if let Ok(v) = p.parse::<u32>() {
                                    if v > 0 {
                                        links.push(CrossRefLink { book: book_name.to_string(), chapter: source_chapter, verse: Some(v), verse_start: None, verse_end: None });
                                    }
                                }
                            }
                        }
                    }
                }

                // Extract the marker from the li id attribute (e.g., "cen-NABRE-1A" -> "A")
                let li_id = li.value().attr("id").unwrap_or("");
                let display_marker = li_id
                    .rsplit('-')
                    .next()
                    .and_then(|s| {
                        // Extract just the letter at the end (e.g., "1A" -> "A")
                        let marker: String = s.chars()
                            .skip_while(|c| c.is_numeric())
                            .collect();
                        if marker.is_empty() {
                            None
                        } else {
                            Some(marker)
                        }
                    });

                let crossref_id = if let Some(marker) = display_marker {
                    format!("{}:{}:{}-{}", book_id, source_chapter, source_verse, marker)
                } else {
                    format!("{}:{}:{}", book_id, source_chapter, source_verse)
                };

                crossrefs.push(CrossRef {
                    id: crossref_id,
                    book: book_name.to_string(),
                    chapter: source_chapter,
                    verse: source_verse,
                    links,
                });
            }
        }
    }

    crossrefs
}

fn process_chapter_file(file_path: &Path, book_code: &str, aliases: &[&str]) -> Result<ChapterData, Box<dyn std::error::Error>> {
    let html_content = fs::read_to_string(file_path)?;
    let document = Html::parse_document(&html_content);

    let book_name_extracted = extract_book_name_from_meta(&document).unwrap_or_else(|| book_code.to_string());
    let (translation_code, translation_name) = extract_translation_name(&document).unwrap_or((String::from("UNKNOWN"), String::new()));
    // Derive chapter strictly from filename: chapter_<N>.html
    let fname = file_path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let chapter = Regex::new(r"^chapter_(\d+)\.html$")
        .unwrap()
        .captures(fname)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<u32>().ok())
        .expect("Filename must be chapter_<N>.html");

    // Some archive folders and verse classes use different aliases for the same book.
    let mut verse_code = book_code;
    if !aliases.is_empty() {
        let spans = Selector::parse("div.passage-content span.text").unwrap();
        for span in document.select(&spans) {
            if let Some(code) = span.value().classes().filter_map(|class| class.split_once('-'))
                .map(|(code, _)| code).find(|code| *code == book_code || aliases.contains(code)) {
                verse_code = code;
                break;
            }
        }
    }
    let verses = extract_verses(&document, verse_code, &translation_code, chapter);
    let footnotes = extract_footnotes_placeholders(&document, &book_name_extracted, chapter);
    let crossrefs = extract_crossrefs(&document, &book_name_extracted, book_code);

    Ok(ChapterData {
        translation_code,
        translation_name,
        book_code: book_code.to_string(),
        book_name: book_name_extracted,
        chapter,
        verses,
        footnotes,
        crossrefs,
    })
}


pub(super) fn convert(path: &Path, book: &str, aliases: &[&str], pretty: bool) -> anyhow::Result<Vec<super::Chapter>> {
    let data = process_chapter_file(path, book, aliases).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(vec![super::render(data.chapter, &data, pretty)?])
}
