//! The canonical chapter schema shared by every site and every workflow.
//!
//! Field-for-field the schema the existing corpora already use, so the JSON
//! this produces is interchangeable with what the old builders wrote.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentItem {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
    pub number: Option<u32>,
    pub id: Option<String>,
    pub display: Option<String>,
}

impl ContentItem {
    pub fn new(
        kind: &str,
        text: Option<String>,
        number: Option<u32>,
        id: Option<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            text,
            number,
            id,
            display: None,
        }
    }

    pub fn text(kind: &str, text: impl Into<String>) -> Self {
        Self::new(kind, Some(text.into()), None, None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verse {
    pub chapter: u32,
    pub verse: u32,
    pub content: Vec<ContentItem>,
    pub header: Vec<ContentItem>,
    pub section: String,
    pub paragraph_start: bool,
    pub poetry_indent: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceCitation {
    pub book: String,
    pub verses: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceBlock {
    pub display: String,
    pub citations: Vec<ReferenceCitation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Footnote {
    pub id: String,
    pub text: String,
    pub references: BTreeMap<String, ReferenceBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossRefLink {
    pub book: String,
    pub chapter: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verse_end: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossRef {
    pub id: String,
    pub book: String,
    pub chapter: u32,
    pub verse: u32,
    pub links: Vec<CrossRefLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterData {
    pub translation_code: String,
    pub translation_name: String,
    pub book_code: String,
    pub book_name: String,
    pub chapter: u32,
    pub verses: Vec<Verse>,
    pub footnotes: Vec<Footnote>,
    #[serde(rename = "cross_refs")]
    pub cross_refs: Vec<CrossRef>,
}

impl ChapterData {
    pub fn new(translation_code: &str, translation_name: &str, book_code: &str, book_name: &str, chapter: u32) -> Self {
        Self {
            translation_code: translation_code.into(),
            translation_name: translation_name.into(),
            book_code: book_code.into(),
            book_name: book_name.into(),
            chapter,
            verses: Vec::new(),
            footnotes: Vec::new(),
            cross_refs: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Interlinear (BibleHub): a different shape, not a variant of the above.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterlinearWord {
    pub strongs: String,
    pub reference: String,
    pub transliteration: String,
    pub original: String,
    pub english: String,
    pub grammar: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterlinearVerse {
    pub chapter: u32,
    pub verse: u32,
    pub header: String,
    pub words: Vec<InterlinearWord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterlinearChapterData {
    pub translation_code: String,
    pub translation_name: String,
    pub book_code: String,
    pub book_name: String,
    pub chapter: u32,
    pub verses: Vec<InterlinearVerse>,
    pub crossrefs: Vec<CrossRef>,
}

/// What a site produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChapterBody {
    Standard(ChapterData),
    Interlinear(InterlinearChapterData),
}

impl ChapterBody {
    pub fn verses(&self) -> usize {
        match self {
            Self::Standard(data) => data.verses.len(),
            Self::Interlinear(data) => data.verses.len(),
        }
    }

    pub fn book_code(&self) -> &str {
        match self {
            Self::Standard(data) => &data.book_code,
            Self::Interlinear(data) => &data.book_code,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared text helpers, used by every site.
// ---------------------------------------------------------------------------

/// Collapse whitespace runs to single spaces and trim.
pub fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One verse per line, text only — no headings, notes or verse markers.
pub fn render_text_chapter(data: &ChapterData) -> String {
    let mut output = String::new();
    for verse in &data.verses {
        let mut line = String::new();
        for item in &verse.content {
            if item.kind != "text" {
                continue;
            }
            let Some(text) = item.text.as_deref() else {
                continue;
            };
            let text = normalized(text);
            if text.is_empty() {
                continue;
            }
            if !line.is_empty() {
                let starts_with_closer =
                    text.starts_with([',', '.', ';', ':', '!', '?', '»', '”', '’', ')', ']']);
                let ends_with_opener = line.ends_with(['«', '“', '‘', '(', '[']);
                if !starts_with_closer && !ends_with_opener {
                    line.push(' ');
                }
            }
            line.push_str(&text);
        }
        if !line.is_empty() {
            let number = verse
                .content
                .first()
                .and_then(|item| item.display.as_deref())
                .map(str::to_owned)
                .unwrap_or_else(|| verse.verse.to_string());
            output.push_str(&format!("{number} {line}\n"));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verse(number: u32, parts: &[&str]) -> Verse {
        Verse {
            chapter: 1,
            verse: number,
            content: std::iter::once(ContentItem::new("verse_number", None, Some(number), None))
                .chain(parts.iter().map(|text| ContentItem::text("text", *text)))
                .collect(),
            header: Vec::new(),
            section: String::new(),
            paragraph_start: number == 1,
            poetry_indent: None,
        }
    }

    #[test]
    fn text_output_is_one_verse_per_line() {
        let mut data = ChapterData::new("NVI", "Nueva Versión Internacional", "MRK", "Marcos", 1);
        data.verses = vec![verse(1, &["Primera", "parte"]), verse(2, &["Segunda."])];
        assert_eq!(render_text_chapter(&data), "1 Primera parte\n2 Segunda.\n");
    }

    #[test]
    fn text_output_respects_punctuation_spacing() {
        let mut data = ChapterData::new("BB", "BasisBibel", "JHN", "Johannes", 1);
        data.verses = vec![verse(1, &["Ein Wort", ".", "Noch eins"])];
        assert_eq!(render_text_chapter(&data), "1 Ein Wort. Noch eins\n");
    }

    #[test]
    fn combined_verse_uses_its_display_range() {
        let mut data = ChapterData::new("NFC", "Nouvelle Français Courant", "JHN", "Jean", 4);
        let mut first = verse(1, &["Texte commun."]);
        first.content[0].display = Some("1-3".into());
        data.verses = vec![first];
        assert_eq!(render_text_chapter(&data), "1-3 Texte commun.\n");
    }
}
