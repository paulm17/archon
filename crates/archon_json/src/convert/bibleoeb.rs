use regex::Regex;
use scraper::{Html, Selector, ElementRef};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;


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

fn normalize_oeb_text(text: &str) -> String {
    text.replace('\u{a0}', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

fn build_text_content_item(translation_id: &str, text: &str) -> Vec<ContentItem> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if translation_id == "SBLGNT" || translation_id == "TR1894" || translation_id == "WLC" {
        tokenize_greek_text(trimmed)
            .into_iter()
            .map(|word| ContentItem {
                kind: "surface".into(),
                text: Some(word),
                number: None,
                id: None,
                display: None,
            })
            .collect()
    } else {
        vec![ContentItem {
            kind: "text".into(),
            text: Some(trimmed.to_string()),
            number: None,
            id: None,
            display: None,
        }]
    }
}

fn oeb_note_marker(index: usize) -> String {
    let mut index = index;
    let mut chars = Vec::new();

    loop {
        chars.push((b'a' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = (index / 26) - 1;
    }

    chars.iter().rev().collect()
}

fn extract_oeb_book_name(book_id: &str) -> String {
    get_abbreviation_map()
        .get(book_id)
        .map(|(_, full_name)| (*full_name).to_string())
        .unwrap_or_else(|| book_id.to_string())
}

fn extract_oeb_book_section(html: &str, book_name: &str) -> Option<String> {
    let toc_re = Regex::new(&format!(
        r##"<a href="#(\d+)">{}</a>"##,
        regex::escape(book_name).replace(" ", "(?: |&nbsp;|&#160;|&#x[Aa]0;| )")
    ))
    .ok()?;
    let anchor_id = toc_re.captures(html)?.get(1)?.as_str().to_string();
    let start_marker = format!(r#"<h1 id="{}"></h1>"#, anchor_id);
    let start_index = html.find(&start_marker)? + start_marker.len();
    let remaining = &html[start_index..];
    let next_book_re = Regex::new(r#"<h1 id="\d+"></h1>"#).ok()?;

    if let Some(next_match) = next_book_re.find(remaining) {
        Some(remaining[..next_match.start()].to_string())
    } else if let Some(body_end) = remaining.find("</body>") {
        Some(remaining[..body_end].to_string())
    } else {
        Some(remaining.to_string())
    }
}

fn next_oeb_block_index(html: &str) -> usize {
    ["<p", "<h1", "<h2", "<h5"]
        .iter()
        .filter_map(|marker| html.find(marker))
        .min()
        .unwrap_or(html.len())
}

fn normalize_oeb_section_html(section_html: &str) -> String {
    let mut normalized = String::with_capacity(section_html.len());
    let mut cursor = 0usize;

    while let Some(close_rel) = section_html[cursor..].find("</p>") {
        let close_end = cursor + close_rel + 4;
        normalized.push_str(&section_html[cursor..close_end]);

        let after_close = &section_html[close_end..];
        let next_block = next_oeb_block_index(after_close);
        let orphan_content = &after_close[..next_block];

        if orphan_content.contains(r#"<span class="v-num">"#) && !orphan_content.trim().is_empty() {
            normalized.push_str("<p>");
            normalized.push_str(orphan_content.trim());
            normalized.push_str("</p>");
        } else {
            normalized.push_str(orphan_content);
        }

        cursor = close_end + next_block;
    }

    normalized.push_str(&section_html[cursor..]);
    normalized
}

enum OebEvent {
    Verse(u32),
    Text(String),
    Note(String),
}

fn oeb_html_to_text(html: &str) -> String {
    Html::parse_fragment(html)
        .root_element()
        .text()
        .collect::<String>()
}

fn push_oeb_text_with_notes(text: &str, events: &mut Vec<OebEvent>) {
    let note_re = Regex::new(r#"(?s)\[Note:\s*(.*?)\s*\]"#).unwrap();
    let mut last = 0;

    for caps in note_re.captures_iter(text) {
        let m = caps.get(0).unwrap();
        let before = &text[last..m.start()];
        if !before.is_empty() {
            events.push(OebEvent::Text(before.to_string()));
        }

        let note_text = caps.get(1).map(|x| x.as_str()).unwrap_or("").trim();
        if !note_text.is_empty() {
            events.push(OebEvent::Note(note_text.to_string()));
        }

        last = m.end();
    }

    let tail = &text[last..];
    if !tail.is_empty() {
        events.push(OebEvent::Text(tail.to_string()));
    }
}

fn extract_oeb_paragraph_events(paragraph: ElementRef) -> Vec<OebEvent> {
    let mut events = Vec::new();
    let html = paragraph.inner_html();
    let verse_re = Regex::new(r#"(?s)<span class="v-num">\s*\[(\d+)\]\s*</span>"#).unwrap();
    let mut last = 0;

    for caps in verse_re.captures_iter(&html) {
        let m = caps.get(0).unwrap();
        let before_html = &html[last..m.start()];
        let before_text = oeb_html_to_text(before_html);
        if !before_text.is_empty() {
            push_oeb_text_with_notes(&before_text, &mut events);
        }

        if let Some(verse_num) = caps.get(1).and_then(|x| x.as_str().parse::<u32>().ok()) {
            events.push(OebEvent::Verse(verse_num));
        }

        last = m.end();
    }

    let tail_html = &html[last..];
    let tail_text = oeb_html_to_text(tail_html);
    if !tail_text.is_empty() {
        push_oeb_text_with_notes(&tail_text, &mut events);
    }
    events
}

fn process_oeb_file(
    translation: &str,
    file_path: &Path,
    book_code: &str,
) -> Result<Vec<ChapterData>, Box<dyn std::error::Error>> {
    let html_content = fs::read_to_string(file_path)?;
    let translation_code = translation.to_owned();
    let translation_name = if translation_code.eq_ignore_ascii_case("OEB") {
        "Open English Bible".to_string()
    } else {
        translation_code.clone()
    };
    let book_name = extract_oeb_book_name(book_code);
    let section_html = extract_oeb_book_section(&html_content, &book_name)
        .ok_or_else(|| format!("Unable to locate OEB section for {}", book_name))?;
    let normalized_section_html = normalize_oeb_section_html(&section_html);
    let fragment = Html::parse_fragment(&normalized_section_html);
    let selector = Selector::parse("h5, p").unwrap();
    let chapter_re =
        Regex::new(&format!(r#"^\[\s*{}\s+(\d+)\s*\]$"#, regex::escape(&book_name))).unwrap();

    let mut chapters = Vec::new();
    let mut current_chapter: Option<u32> = None;
    let mut current_verse_num: Option<u32> = None;
    let mut current_content: Vec<ContentItem> = Vec::new();
    let mut current_text = String::new();
    let mut current_header: Vec<ContentItem> = Vec::new();
    let mut current_section = String::new();
    let mut current_paragraph_start = false;
    let mut current_indent: Option<u32> = None;
    let mut pending_header: Option<String> = None;
    let mut chapter_verses: Vec<Verse> = Vec::new();
    let mut chapter_footnotes: Vec<Footnote> = Vec::new();
    let mut note_index = 0usize;

    let flush_current_text = |content: &mut Vec<ContentItem>, text_buffer: &mut String| {
        let normalized = normalize_oeb_text(text_buffer);
        if !normalized.is_empty() {
            content.extend(build_text_content_item(&translation_code, &normalized));
        }
        text_buffer.clear();
    };

    let flush_current_verse = |
        chapter_num: Option<u32>,
        verse_num: &mut Option<u32>,
        content: &mut Vec<ContentItem>,
        text_buffer: &mut String,
        header: &mut Vec<ContentItem>,
        section: &mut String,
        paragraph_start: &mut bool,
        indent: &mut Option<u32>,
        verses: &mut Vec<Verse>
    | {
        if let Some(verse) = *verse_num {
            flush_current_text(content, text_buffer);
            if !content.is_empty() {
                verses.push(Verse {
                    chapter: chapter_num.unwrap_or_default(),
                    verse,
                    content: std::mem::take(content),
                    header: std::mem::take(header),
                    section: section.clone(),
                    paragraph_start: *paragraph_start,
                    poetry_indent: *indent,
                });
            }
            *verse_num = None;
            section.clear();
            *paragraph_start = false;
            *indent = None;
        }
    };

    let flush_current_chapter = |
        chapter_num: &mut Option<u32>,
        verse_num: &mut Option<u32>,
        content: &mut Vec<ContentItem>,
        text_buffer: &mut String,
        header: &mut Vec<ContentItem>,
        section: &mut String,
        paragraph_start: &mut bool,
        indent: &mut Option<u32>,
        verses: &mut Vec<Verse>,
        footnotes: &mut Vec<Footnote>,
        chapters_out: &mut Vec<ChapterData>
    | {
        flush_current_verse(
            *chapter_num,
            verse_num,
            content,
            text_buffer,
            header,
            section,
            paragraph_start,
            indent,
            verses,
        );

        if let Some(chapter_num) = *chapter_num {
            chapters_out.push(ChapterData {
                translation_code: translation_code.clone(),
                translation_name: translation_name.clone(),
                book_code: book_code.to_string(),
                book_name: book_name.clone(),
                chapter: chapter_num,
                verses: std::mem::take(verses),
                footnotes: std::mem::take(footnotes),
                crossrefs: Vec::new(),
            });
        }
    };

    for element in fragment.select(&selector) {
        match element.value().name() {
            "h5" => {
                let heading = normalize_oeb_text(&element.text().collect::<String>());
                if !heading.is_empty() {
                    pending_header = Some(heading);
                }
            }
            "p" => {
                let class_list: Vec<_> = element.value().classes().collect();

                if class_list.contains(&"c-num") {
                    let marker_text = normalize_oeb_text(&element.text().collect::<String>());
                    if let Some(caps) = chapter_re.captures(&marker_text) {
                        flush_current_chapter(
                            &mut current_chapter,
                            &mut current_verse_num,
                            &mut current_content,
                            &mut current_text,
                            &mut current_header,
                            &mut current_section,
                            &mut current_paragraph_start,
                            &mut current_indent,
                            &mut chapter_verses,
                            &mut chapter_footnotes,
                            &mut chapters,
                        );
                        current_chapter = caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok());
                        pending_header = None;
                        note_index = 0;
                    }
                    continue;
                }

                if current_chapter.is_none() {
                    continue;
                }

                let paragraph_indent = parse_indent_from_classes(&class_list);
                let events = extract_oeb_paragraph_events(element);
                let mut verse_started_in_paragraph = false;

                for event in events {
                    match event {
                        OebEvent::Verse(verse_num) => {
                            flush_current_verse(
                                current_chapter,
                                &mut current_verse_num,
                                &mut current_content,
                                &mut current_text,
                                &mut current_header,
                                &mut current_section,
                                &mut current_paragraph_start,
                                &mut current_indent,
                                &mut chapter_verses,
                            );

                            current_verse_num = Some(verse_num);
                            current_content.push(ContentItem {
                                kind: "verse_number".into(),
                                text: None,
                                number: Some(verse_num),
                                id: None,
                                display: None,
                            });
                            current_paragraph_start = !verse_started_in_paragraph;
                            current_indent = paragraph_indent;

                            if let Some(header_text) = pending_header.take() {
                                current_header =
                                    parse_header_with_crossrefs(&header_text, book_code, current_chapter.unwrap_or_default(), verse_num);
                            } else {
                                current_header = Vec::new();
                            }

                            verse_started_in_paragraph = true;
                        }
                        OebEvent::Text(text) => {
                            if current_verse_num.is_some() {
                                current_text.push_str(&text);
                                current_text.push(' ');
                            }
                        }
                        OebEvent::Note(note_text) => {
                            if let Some(verse_num) = current_verse_num {
                                flush_current_text(&mut current_content, &mut current_text);
                                let marker = oeb_note_marker(note_index);
                                let note_id = format!(
                                    "{}:{}:{}-{}",
                                    book_code,
                                    current_chapter.unwrap_or_default(),
                                    verse_num,
                                    marker
                                );
                                current_content.push(ContentItem {
                                    kind: "footnote".into(),
                                    text: Some(marker.clone()),
                                    number: None,
                                    id: Some(note_id.clone()),
                                    display: None,
                                });
                                chapter_footnotes.push(Footnote {
                                    id: note_id,
                                    text: normalize_oeb_text(&note_text),
                                    references: HashMap::new(),
                                });
                                note_index += 1;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    flush_current_chapter(
        &mut current_chapter,
        &mut current_verse_num,
        &mut current_content,
        &mut current_text,
        &mut current_header,
        &mut current_section,
        &mut current_paragraph_start,
        &mut current_indent,
        &mut chapter_verses,
        &mut chapter_footnotes,
        &mut chapters,
    );

    Ok(chapters)
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



pub(super) fn convert(path: &Path, book: &str, translation: &str, pretty: bool) -> anyhow::Result<Vec<super::Chapter>> {
    let data = process_oeb_file(translation, path, book).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    data.iter().map(|item| super::render(item.chapter, item, pretty)).collect()
}
