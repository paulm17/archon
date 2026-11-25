//! Stage 3a — Scripture
//!
//! Reads chapter JSON from language/translation/book or legacy book/translation folders.
//! Auto-discovers all translations from directory structure.
//! Emits scripture graph JSONL files for the Bible schema:
//! - books.jsonl
//! - languages.jsonl
//! - translations.jsonl
//! - categories.jsonl
//! - book_translations.jsonl
//! - book_categories.jsonl
//! - chapters.jsonl
//! - verses.jsonl
//! - verse_contents.jsonl
//! - interlinear_tokens.jsonl
//! - chunks.jsonl

use crate::ids::make_id;
use crate::jsonl::{open_writer, write_record};
use crate::progress::{count_bar, finish_ok, spinner};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;

use std::{
    io::Write,
    path::{Path, PathBuf},
};

/// Infer language code from translation code (matches insert_book.rs detect_language)
fn detect_language_code(translation_code: &str) -> &'static str {
    match translation_code {
        "SBLGNT" | "TR1894" | "greek" => "gr",
        "WLC" => "he",
        "coptic" => "cop",
        _ => "en",
    }
}

fn language_name(language_code: &str) -> &'static str {
    match language_code {
        "he" => "Hebrew",
        "gr" => "Greek",
        "cop" => "Coptic",
        _ => "English",
    }
}

// ── Source JSON deserialization ──

#[allow(dead_code)]
#[derive(Deserialize)]
struct ChapterFile {
    translation_code: String,
    translation_name: String,
    book_code: String,
    book_name: String,
    chapter: u32,
    #[serde(default)]
    verses: Vec<VerseEntry>,
    #[serde(default)]
    footnotes: Vec<serde_json::Value>,
    #[serde(default, rename = "cross_refs")]
    crossrefs: Vec<serde_json::Value>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct VerseEntry {
    #[serde(default)]
    chapter: u32,
    verse: u32,
    #[serde(default)]
    content: Vec<ContentItem>,
    #[serde(default)]
    words: Vec<InterlinearWord>,
    #[serde(default)]
    header: Option<serde_json::Value>,
    #[serde(default)]
    section: String,
    #[serde(default)]
    paragraph_start: bool,
    #[serde(default)]
    poetry_indent: Option<i32>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ThomasTextFile {
    translation_code: String,
    translation_name: String,
    book_code: String,
    book_name: String,
    #[serde(default)]
    logia: Vec<ThomasLogionEntry>,
    #[serde(default)]
    verses: Vec<ThomasVerseEntry>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ThomasLogionEntry {
    logion: u32,
    text: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ThomasVerseEntry {
    verse: u32,
    #[serde(default)]
    header: Option<serde_json::Value>,
    #[serde(default)]
    words: Vec<InterlinearWord>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ContentItem {
    #[serde(rename = "type")]
    item_type: String,
    text: Option<String>,
    #[serde(default)]
    number: Option<i32>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    lemma: Option<String>,
    #[serde(default)]
    strong: Option<String>,
    #[serde(default)]
    morph: Option<String>,
    #[serde(default)]
    gloss: Option<String>,
    #[serde(default)]
    token_id: Option<String>,
    #[serde(default)]
    token_index: Option<i32>,
}

#[allow(dead_code)]
#[derive(Clone, Deserialize)]
struct InterlinearWord {
    strongs: Option<String>,
    reference: Option<String>,
    transliteration: Option<String>,
    original: Option<String>,
    english: Option<String>,
    grammar: Option<String>,
}

// ── Output record types ──

#[derive(Serialize)]
struct BookRecord {
    id: String,
    code: String,
    name_en: String,
    book_corpus: String,
    testament: Option<String>,
}

#[derive(Serialize)]
struct LanguageRecord {
    id: String,
    code: String,
    name: String,
}

#[derive(Serialize)]
struct TranslationRecord {
    id: String,
    code: String,
    name: String,
    language_id: String,
}

#[derive(Serialize)]
struct CategoryRecord {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct BookTranslationRecord {
    id: String,
    book_id: String,
    translation_id: String,
}

#[derive(Serialize)]
struct BookCategoryRecord {
    id: String,
    book_id: String,
    category_id: String,
}

#[derive(Serialize)]
struct ChapterRecord {
    id: String,
    book_id: String,
    book_translation_id: String,
    number: u32,
}

#[derive(Serialize)]
struct VerseRecord {
    id: String,
    chapter_id: String,
    number: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    logion_number: Option<u32>,
    chunk_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    chapter_heading: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    section_heading: Option<String>,
    paragraph_start: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    poetry_indent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<String>,
}

#[derive(Serialize)]
struct VerseContentRecord {
    id: String,
    verse_id: String,
    translation_id: String,
    language_code: String,
    #[serde(rename = "type")]
    content_type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    normalized_text: Option<String>,
}

#[derive(Serialize)]
struct InterlinearTokenRecord {
    id: String,
    verse_id: String,
    translation_id: String,
    language_code: String,
    token_index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    strong: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transliteration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    english_gloss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grammar: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<String>,
}

#[derive(Serialize)]
struct ChunkRecord {
    id: String,
    chunk_type: String,
    start_verse_id: String,
    end_verse_id: String,
    verse_ids: Vec<String>,
    text: String,
}

struct VerseText {
    translation_code: String,
    text: String,
    words: Vec<InterlinearWord>,
    logion_number: Option<u32>,
    chapter_heading: Option<String>,
    section_heading: Option<String>,
    paragraph_start: bool,
    poetry_indent: Option<i32>,
    metadata: Option<String>,
}

struct BookData {
    code: String,
    name_en: String,
    // chapter -> verse_num -> [VerseText per translation]
    chapters: BTreeMap<u32, BTreeMap<u32, Vec<VerseText>>>,
}

struct TranslationData {
    code: String,
    name: String,
    language_code: String,
}

fn canonical_testament(code: &str) -> &'static str {
    match code {
        "Gen" | "Exo" | "Lev" | "Num" | "Deu" | "Jos" | "Jdg" | "Rut" | "1Sa" | "2Sa" | "1Ki"
        | "2Ki" | "1Ch" | "2Ch" | "Ezr" | "Neh" | "Est" | "Job" | "Psa" | "Pro" | "Ecc" | "Sng"
        | "Isa" | "Jer" | "Lam" | "Ezk" | "Dan" | "Hos" | "Jol" | "Amo" | "Oba" | "Jon" | "Mic"
        | "Nam" | "Hab" | "Zep" | "Hag" | "Zec" | "Mal" => "OT",
        "Mat" | "Matt" | "Mrk" | "Mark" | "Luk" | "Luke" | "Jhn" | "John" | "Act" | "Acts"
        | "Rom" | "1Co" | "2Co" | "Gal" | "Eph" | "Php" | "Col" | "1Th" | "2Th" | "1Ti" | "2Ti"
        | "Tit" | "Phm" | "Heb" | "Jas" | "1Pe" | "2Pe" | "1Jo" | "2Jo" | "3Jo" | "Jud" | "Rev" => {
            "NT"
        }
        _ => "APOCRYPHA",
    }
}

fn testament(code: &str) -> Option<&'static str> {
    if book_corpus(code) == "GNOSTIC" {
        None
    } else {
        Some(canonical_testament(code))
    }
}

fn category_name_for_book(code: &str) -> &'static str {
    if book_corpus(code) == "GNOSTIC" {
        return "Gnostic Texts";
    }
    match canonical_testament(code) {
        "OT" => "Old Testament",
        "NT" => "New Testament",
        _ => "Apocrypha",
    }
}

fn book_corpus(code: &str) -> &'static str {
    if code == "Thomas" {
        "GNOSTIC"
    } else {
        "CANONICAL"
    }
}

fn extract_header_text(value: &serde_json::Value, positioned_notes: bool) -> Option<String> {
    match value {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        serde_json::Value::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|part| extract_header_text(part, positioned_notes))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" ").trim().to_string())
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(item_type) = map.get("type").and_then(|v| v.as_str()) {
                match item_type {
                    "text" | "surface" => {
                        if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                return Some(trimmed.to_string());
                            }
                        }
                    }
                    "footnote"
                        if positioned_notes
                            && map.get("number").and_then(|n| n.as_u64()).is_some() =>
                    {
                        return None;
                    }
                    "footnote" => {
                        if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                return Some(format!("[fn:{trimmed}]"));
                            }
                        }
                    }
                    "cross_reference" | "crossref" => {
                        if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                return Some(format!("[cr:{trimmed}]"));
                            }
                        }
                    }
                    "verse_number" => {
                        if let Some(num) = map.get("number").and_then(|v| v.as_i64()) {
                            return Some(format!("[vs:{num}]"));
                        }
                    }
                    "inline_h3" => {
                        if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                return Some(format!("[inline:{trimmed}]"));
                            }
                        }
                    }
                    _ => {}
                }
            }

            for key in ["text", "title", "heading", "content", "label", "value"] {
                if let Some(text) = map
                    .get(key)
                    .and_then(|part| extract_header_text(part, positioned_notes))
                {
                    return Some(text);
                }
            }
            None
        }
        _ => None,
    }
}

fn id_suffix(id: &str) -> &str {
    id.rsplit('-').next().unwrap_or(id)
}

fn extract_inline_heading(verse: &VerseEntry) -> Option<String> {
    verse
        .content
        .iter()
        .find(|item| item.item_type == "inline_h3")
        .and_then(|item| item.text.as_deref())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(|text| text.to_string())
}

fn collect_markers_from_header_value(
    value: &serde_json::Value,
    footnote_markers: &mut Vec<(String, String)>,
    crossref_markers: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_markers_from_header_value(item, footnote_markers, crossref_markers);
            }
        }
        serde_json::Value::Object(map) => {
            let item_type = map.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let marker = map
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("");
            let id = map.get("id").and_then(|v| v.as_str()).unwrap_or("");

            if !marker.is_empty() && !id.is_empty() {
                match item_type {
                    "footnote" => footnote_markers.push((id.to_string(), marker.to_string())),
                    "cross_reference" | "crossref" => {
                        crossref_markers.push((id.to_string(), marker.to_string()))
                    }
                    _ => {}
                }
            }

            for nested in map.values() {
                collect_markers_from_header_value(nested, footnote_markers, crossref_markers);
            }
        }
        _ => {}
    }
}

fn metadata_footnotes_for_verse(
    chapter_file: &ChapterFile,
    verse: &VerseEntry,
) -> Vec<serde_json::Value> {
    let mut footnote_markers: Vec<(String, String)> = Vec::new();
    let mut header_crossref_markers: Vec<(String, String)> = Vec::new();

    if let Some(header) = verse.header.as_ref() {
        collect_markers_from_header_value(
            header,
            &mut footnote_markers,
            &mut header_crossref_markers,
        );
    }

    for item in &verse.content {
        if item.item_type == "footnote" {
            if let (Some(id), Some(marker)) = (item.id.as_deref(), item.text.as_deref()) {
                let marker = marker.trim();
                if !id.is_empty() && !marker.is_empty() {
                    footnote_markers.push((id.to_string(), marker.to_string()));
                }
            }
        }
    }

    let mut out = Vec::new();
    for (id, marker) in footnote_markers {
        let source = chapter_file
            .footnotes
            .iter()
            .find(|f| f.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
            .or_else(|| {
                let suffix = id_suffix(&id);
                chapter_file.footnotes.iter().find(|f| {
                    f.get("id")
                        .and_then(|v| v.as_str())
                        .map(|fid| id_suffix(fid) == suffix)
                        .unwrap_or(false)
                })
            });

        let mut row = serde_json::Map::new();
        row.insert("marker".to_string(), serde_json::Value::String(marker));
        row.insert(
            "id".to_string(),
            serde_json::Value::String(
                source
                    .and_then(|f| f.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string(),
            ),
        );
        row.insert(
            "text".to_string(),
            serde_json::Value::String(
                source
                    .and_then(|f| f.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            ),
        );
        row.insert(
            "references".to_string(),
            source
                .and_then(|f| f.get("references"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
        );

        out.push(serde_json::Value::Object(row));
    }

    out
}

fn metadata_crossrefs_for_verse(
    chapter_file: &ChapterFile,
    verse: &VerseEntry,
) -> Vec<serde_json::Value> {
    let mut header_footnote_markers: Vec<(String, String)> = Vec::new();
    let mut crossref_markers: Vec<(String, String)> = Vec::new();

    if let Some(header) = verse.header.as_ref() {
        collect_markers_from_header_value(
            header,
            &mut header_footnote_markers,
            &mut crossref_markers,
        );
    }

    for item in &verse.content {
        if item.item_type == "crossref" || item.item_type == "cross_reference" {
            if let (Some(id), Some(marker)) = (item.id.as_deref(), item.text.as_deref()) {
                let marker = marker.trim();
                if !id.is_empty() && !marker.is_empty() {
                    crossref_markers.push((id.to_string(), marker.to_string()));
                }
            }
        }
    }

    let mut out = Vec::new();
    for (id, marker) in crossref_markers {
        let source = chapter_file
            .crossrefs
            .iter()
            .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
            .or_else(|| {
                let suffix = id_suffix(&id);
                chapter_file.crossrefs.iter().find(|c| {
                    c.get("id")
                        .and_then(|v| v.as_str())
                        .map(|cid| id_suffix(cid) == suffix)
                        .unwrap_or(false)
                })
            });

        let mut row = serde_json::Map::new();
        row.insert("marker".to_string(), serde_json::Value::String(marker));
        row.insert(
            "id".to_string(),
            serde_json::Value::String(
                source
                    .and_then(|c| c.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string(),
            ),
        );

        let refs = source
            .and_then(|c| c.get("links").cloned())
            .or_else(|| source.and_then(|c| c.get("references").cloned()))
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new()));
        row.insert("references".to_string(), refs);

        out.push(serde_json::Value::Object(row));
    }

    out
}

fn metadata_for_verse(chapter_file: &ChapterFile, verse: &VerseEntry) -> Option<String> {
    let mut meta = serde_json::Map::new();

    if let Some(text) = extract_inline_heading(verse) {
        meta.insert("inlineHeading".to_string(), serde_json::Value::String(text));
    }

    let mut footnotes = metadata_footnotes_for_verse(chapter_file, verse);
    let mut plural = Vec::new();
    let mut annotate = |kind: &str, id: Option<&str>, offset: u64, target: &str| {
        if kind == "plural" {
            plural.push(serde_json::json!({ "offset": offset, "target": target }));
        } else if kind == "footnote" {
            if let Some(note) = footnotes.iter_mut().find(|n| n["id"].as_str() == id) {
                note["offset"] = offset.into();
                note["target"] = target.into();
            }
        }
    };
    if chapter_file.translation_code == "BLB" {
        for item in &verse.content {
            if let Some(offset) = item.number.and_then(|n| u64::try_from(n).ok()) {
                annotate(&item.item_type, item.id.as_deref(), offset, "verse");
            }
        }
        if let Some(items) = verse.header.as_ref().and_then(|h| h.as_array()) {
            for item in items {
                if let Some(offset) = item["number"].as_u64() {
                    annotate(
                        item["type"].as_str().unwrap_or(""),
                        item["id"].as_str(),
                        offset,
                        "heading",
                    );
                }
            }
        }
    }
    if !plural.is_empty() {
        meta.insert("pluralMarks".into(), plural.into());
    }
    if !footnotes.is_empty() {
        meta.insert("footnotes".to_string(), serde_json::Value::Array(footnotes));
    }

    let crossrefs = metadata_crossrefs_for_verse(chapter_file, verse);
    if !crossrefs.is_empty() {
        meta.insert(
            "cross_refs".to_string(),
            serde_json::Value::Array(crossrefs),
        );
    }

    if meta.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(meta).to_string())
    }
}

/// Extract plain text from verse: uses content items for translations, words for interlinear.
fn extract_verse_text(verse: &VerseEntry, positioned_notes: bool) -> String {
    // If we have content items, use text/surface items (normal translations)
    if !verse.content.is_empty() {
        let mut result = String::new();

        for item in &verse.content {
            match item.item_type.as_str() {
                "text" | "surface" => {
                    if let Some(text) = item.text.as_deref() {
                        if !result.is_empty() && !result.ends_with(' ') {
                            result.push(' ');
                        }
                        result.push_str(text);
                    }
                }
                "verse_number" => {
                    if let Some(num) = item.number {
                        result.push_str(&format!("[vs:{num}] "));
                    }
                }
                "inline_h3" => {
                    if let Some(text) = item.text.as_deref() {
                        result.push_str("[inline]");
                        if !result.ends_with(' ') {
                            result.push(' ');
                        }
                        result.push_str(text);
                    }
                }
                "footnote" if positioned_notes && item.number.is_some() => {}
                "footnote" => {
                    if let Some(text) = item.text.as_deref() {
                        result.push_str(&format!("[fn:{text}] "));
                    }
                }
                "cross_reference" | "crossref" => {
                    if let Some(text) = item.text.as_deref() {
                        result.push_str(&format!("[cr:{text}] "));
                    }
                }
                _ => {}
            }
        }

        return result.trim().to_string();
    }
    // Fall back to interlinear words (english glosses)
    if !verse.words.is_empty() {
        return verse
            .words
            .iter()
            .filter_map(|w| w.english.as_deref())
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
    }
    String::new()
}

fn subdirectories(root: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for entry in fs::read_dir(root).with_context(|| format!("Failed to read {}", root.display()))? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && !entry.file_name().to_string_lossy().starts_with('.') {
            directories.push(entry.path());
        }
    }
    directories.sort();
    Ok(directories)
}

pub(crate) fn export(translations_dir: &Path, ingest_dir: &Path) -> Result<()> {
    let sp = spinner("Scanning translations");

    // Multi-translation data model:
    // - Chapter/verse rows are translation-scoped
    // - VerseContents store text per translation-specific verse
    let mut books: BTreeMap<String, BookData> = BTreeMap::new();
    let mut translations: BTreeMap<String, TranslationData> = BTreeMap::new();
    let mut book_translation_pairs: BTreeSet<(String, String)> = BTreeSet::new();

    for directory in subdirectories(translations_dir)? {
        let name = directory.file_name().and_then(|name| name.to_str());
        let mut chapter_dirs = Vec::new();
        if matches!(name, Some("en" | "gr" | "he")) {
            // Current output is grouped by language, then translation, then book.
            for translation in subdirectories(&directory)? {
                chapter_dirs.extend(subdirectories(&translation)?);
            }
        } else {
            // Legacy output is grouped by book, then translation.
            chapter_dirs = subdirectories(&directory)?;
        }

        for trans_dir in chapter_dirs {
            if name == Some("Thomas") {
                let thomas_path = trans_dir.join("text.json");
                let data = fs::read_to_string(&thomas_path)
                    .with_context(|| format!("Failed to read {}", thomas_path.display()))?;
                let file: ThomasTextFile = serde_json::from_str(&data)
                    .with_context(|| format!("Failed to parse {}", thomas_path.display()))?;

                let lang_code = detect_language_code(&file.translation_code).to_string();
                translations
                    .entry(file.translation_code.clone())
                    .or_insert_with(|| TranslationData {
                        code: file.translation_code.clone(),
                        name: file.translation_name.clone(),
                        language_code: lang_code.clone(),
                    });
                book_translation_pairs
                    .insert((file.book_code.clone(), file.translation_code.clone()));

                let book = books
                    .entry(file.book_code.clone())
                    .or_insert_with(|| BookData {
                        code: file.book_code.clone(),
                        name_en: file.book_name.clone(),
                        chapters: BTreeMap::new(),
                    });
                let chapter_verses = book.chapters.entry(1).or_default();

                if !file.logia.is_empty() {
                    for logion in &file.logia {
                        let text = logion.text.trim().to_string();
                        if !text.is_empty() {
                            chapter_verses
                                .entry(logion.logion)
                                .or_default()
                                .push(VerseText {
                                    translation_code: file.translation_code.clone(),
                                    text,
                                    words: Vec::new(),
                                    logion_number: Some(logion.logion),
                                    chapter_heading: None,
                                    section_heading: None,
                                    paragraph_start: false,
                                    poetry_indent: None,
                                    metadata: None,
                                });
                        }
                    }
                } else {
                    for v in &file.verses {
                        let text = if !v.words.is_empty() {
                            v.words
                                .iter()
                                .filter_map(|w| w.english.as_deref())
                                .collect::<Vec<_>>()
                                .join(" ")
                                .trim()
                                .to_string()
                        } else {
                            String::new()
                        };
                        if !text.is_empty() {
                            chapter_verses.entry(v.verse).or_default().push(VerseText {
                                translation_code: file.translation_code.clone(),
                                text,
                                words: v.words.clone(),
                                logion_number: Some(v.verse),
                                chapter_heading: v
                                    .header
                                    .as_ref()
                                    .and_then(|h| extract_header_text(h, false)),
                                section_heading: None,
                                paragraph_start: false,
                                poetry_indent: None,
                                metadata: None,
                            });
                        }
                    }
                }
                continue;
            }

            // Read all chapter_N.json files
            let mut chapter_files: Vec<String> = Vec::new();
            for entry in fs::read_dir(&trans_dir)
                .with_context(|| format!("Failed to read {}", trans_dir.display()))?
            {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("chapter_") && name.ends_with(".json") {
                    chapter_files.push(name);
                }
            }
            chapter_files.sort_by(|a, b| {
                let n_a = a
                    .trim_start_matches("chapter_")
                    .trim_end_matches(".json")
                    .parse::<u32>()
                    .unwrap_or(0);
                let n_b = b
                    .trim_start_matches("chapter_")
                    .trim_end_matches(".json")
                    .parse::<u32>()
                    .unwrap_or(0);
                n_a.cmp(&n_b)
            });

            if chapter_files.is_empty() {
                continue;
            }

            for file_name in &chapter_files {
                let path = trans_dir.join(file_name);
                let data = fs::read_to_string(&path)
                    .with_context(|| format!("Failed to read {}", path.display()))?;
                let mut chapter_file: ChapterFile = serde_json::from_str(&data)
                    .with_context(|| format!("Failed to parse {}", path.display()))?;

                // BibleHub can write chapter 1 in the payload even when the file
                // contains another chapter. The scraper's filename carries the chapter.
                if chapter_file.translation_code == "interlinear" {
                    chapter_file.chapter = file_name
                        .trim_start_matches("chapter_")
                        .trim_end_matches(".json")
                        .parse()
                        .with_context(|| format!("Invalid chapter filename {}", path.display()))?;
                }

                let lang_code = detect_language_code(&chapter_file.translation_code).to_string();
                translations
                    .entry(chapter_file.translation_code.clone())
                    .or_insert_with(|| TranslationData {
                        code: chapter_file.translation_code.clone(),
                        name: chapter_file.translation_name.clone(),
                        language_code: lang_code.clone(),
                    });
                book_translation_pairs.insert((
                    chapter_file.book_code.clone(),
                    chapter_file.translation_code.clone(),
                ));

                let book = books
                    .entry(chapter_file.book_code.clone())
                    .or_insert_with(|| BookData {
                        code: chapter_file.book_code.clone(),
                        name_en: chapter_file.book_name.clone(),
                        chapters: BTreeMap::new(),
                    });

                let chapter_verses = book.chapters.entry(chapter_file.chapter).or_default();
                for v in &chapter_file.verses {
                    let positioned_notes = chapter_file.translation_code == "BLB";
                    let text = extract_verse_text(v, positioned_notes);
                    if !text.is_empty() {
                        chapter_verses.entry(v.verse).or_default().push(VerseText {
                            translation_code: chapter_file.translation_code.clone(),
                            text,
                            words: v.words.clone(),
                            logion_number: None,
                            chapter_heading: v
                                .header
                                .as_ref()
                                .and_then(|h| extract_header_text(h, positioned_notes)),
                            section_heading: if v.section.trim().is_empty() {
                                None
                            } else {
                                Some(v.section.trim().to_string())
                            },
                            paragraph_start: v.paragraph_start,
                            poetry_indent: v.poetry_indent,
                            metadata: metadata_for_verse(&chapter_file, v),
                        });
                    }
                }
            }
        }
    }

    sp.finish_and_clear();

    // Count totals
    let total_book_translations = book_translation_pairs.len();
    let total_chapters: usize = books
        .values()
        .map(|b| {
            b.chapters
                .values()
                .map(|chapter| {
                    chapter
                        .values()
                        .flat_map(|translations| {
                            translations.iter().map(|vt| vt.translation_code.clone())
                        })
                        .collect::<BTreeSet<_>>()
                        .len()
                })
                .sum::<usize>()
        })
        .sum();
    let total_verses: usize = books
        .values()
        .map(|b| {
            b.chapters
                .values()
                .map(|c| c.values().map(|v| v.len()).sum::<usize>())
                .sum::<usize>()
        })
        .sum();
    let total_verse_contents: usize = books
        .values()
        .map(|b| {
            b.chapters
                .values()
                .map(|c| c.values().map(|v| v.len()).sum::<usize>())
                .sum::<usize>()
        })
        .sum();

    eprintln!(
        "   Found {} books, {} translations, {} book_translations, {} chapters, {} verses, {} verse_contents",
        books.len(),
        translations.len(),
        total_book_translations,
        total_chapters,
        total_verses,
        total_verse_contents
    );

    // Write books.jsonl
    let pb = count_bar(books.len() as u64, "books.jsonl");
    let mut w = open_writer(&ingest_dir.join("books.jsonl"))?;
    for book in books.values() {
        write_record(
            &mut w,
            &BookRecord {
                id: make_id("book", &[&book.code]),
                code: book.code.clone(),
                name_en: book.name_en.clone(),
                book_corpus: book_corpus(&book.code).to_string(),
                testament: testament(&book.code).map(|t| t.to_string()),
            },
        )?;
        pb.inc(1);
    }
    w.flush()?;
    finish_ok(&pb, &format!("books.jsonl ({} books)", books.len()));

    let pb = count_bar(
        translations.len() as u64,
        "languages.jsonl + translations.jsonl",
    );
    let mut lw = open_writer(&ingest_dir.join("languages.jsonl"))?;
    let mut tw = open_writer(&ingest_dir.join("translations.jsonl"))?;
    let mut emitted_languages: BTreeSet<String> = BTreeSet::new();
    for translation in translations.values() {
        if emitted_languages.insert(translation.language_code.clone()) {
            write_record(
                &mut lw,
                &LanguageRecord {
                    id: make_id("language", &[&translation.language_code]),
                    code: translation.language_code.clone(),
                    name: language_name(&translation.language_code).to_string(),
                },
            )?;
        }
        write_record(
            &mut tw,
            &TranslationRecord {
                id: make_id("translation", &[&translation.code]),
                code: translation.code.clone(),
                name: translation.name.clone(),
                language_id: make_id("language", &[&translation.language_code]),
            },
        )?;
        pb.inc(1);
    }
    lw.flush()?;
    tw.flush()?;
    finish_ok(
        &pb,
        &format!(
            "languages.jsonl ({} languages) + translations.jsonl ({} translations)",
            emitted_languages.len(),
            translations.len()
        ),
    );

    let mut category_names: BTreeSet<String> = BTreeSet::new();
    for book in books.values() {
        category_names.insert(category_name_for_book(&book.code).to_string());
    }
    let pb = count_bar(category_names.len() as u64, "categories.jsonl");
    let mut catw = open_writer(&ingest_dir.join("categories.jsonl"))?;
    for category_name in &category_names {
        write_record(
            &mut catw,
            &CategoryRecord {
                id: make_id("category", &[category_name]),
                name: category_name.clone(),
            },
        )?;
        pb.inc(1);
    }
    catw.flush()?;
    finish_ok(
        &pb,
        &format!("categories.jsonl ({} categories)", category_names.len()),
    );

    let pb = count_bar(total_book_translations as u64, "book_translations.jsonl");
    let mut btw = open_writer(&ingest_dir.join("book_translations.jsonl"))?;
    for (book_code, translation_code) in &book_translation_pairs {
        write_record(
            &mut btw,
            &BookTranslationRecord {
                id: make_id("book-translation", &[book_code, translation_code]),
                book_id: make_id("book", &[book_code]),
                translation_id: make_id("translation", &[translation_code]),
            },
        )?;
        pb.inc(1);
    }
    btw.flush()?;
    finish_ok(
        &pb,
        &format!("book_translations.jsonl ({total_book_translations} rows)"),
    );

    let pb = count_bar(books.len() as u64, "book_categories.jsonl");
    let mut bcw = open_writer(&ingest_dir.join("book_categories.jsonl"))?;
    for book in books.values() {
        let category_name = category_name_for_book(&book.code);
        write_record(
            &mut bcw,
            &BookCategoryRecord {
                id: make_id("book-category", &[&book.code]),
                book_id: make_id("book", &[&book.code]),
                category_id: make_id("category", &[category_name]),
            },
        )?;
        pb.inc(1);
    }
    bcw.flush()?;
    finish_ok(
        &pb,
        &format!("book_categories.jsonl ({} rows)", books.len()),
    );

    // Write chapters.jsonl
    let pb = count_bar(total_chapters as u64, "chapters.jsonl");
    let mut w = open_writer(&ingest_dir.join("chapters.jsonl"))?;
    for book in books.values() {
        for (&chap_num, verses) in &book.chapters {
            let translation_codes: BTreeSet<String> = verses
                .values()
                .flat_map(|items| items.iter().map(|vt| vt.translation_code.clone()))
                .collect();
            for translation_code in translation_codes {
                write_record(
                    &mut w,
                    &ChapterRecord {
                        id: make_id(
                            "chapter",
                            &[&book.code, &translation_code, &chap_num.to_string()],
                        ),
                        book_id: make_id("book", &[&book.code]),
                        book_translation_id: make_id(
                            "book-translation",
                            &[&book.code, &translation_code],
                        ),
                        number: chap_num,
                    },
                )?;
                pb.inc(1);
            }
        }
    }
    w.flush()?;
    finish_ok(&pb, &format!("chapters.jsonl ({total_chapters} chapters)"));

    // Write verses.jsonl (translation-scoped) + verse_contents.jsonl + interlinear_tokens.jsonl
    struct VerseForChunk {
        verse_id: String,
        text: String,
        book_code: String,
        translation_code: String,
    }
    let mut ordered_verses: Vec<VerseForChunk> = Vec::new();
    let mut verse_count: u64 = 0;
    let mut verse_content_count: u64 = 0;
    let mut interlinear_token_count: u64 = 0;

    let mut vw = open_writer(&ingest_dir.join("verses.jsonl"))?;
    let mut vcw = open_writer(&ingest_dir.join("verse_contents.jsonl"))?;
    let mut itw = open_writer(&ingest_dir.join("interlinear_tokens.jsonl"))?;
    let pb = count_bar(
        total_verse_contents as u64,
        "verses.jsonl + verse_contents.jsonl + interlinear_tokens.jsonl",
    );

    for book in books.values() {
        for (&chap_num, chapter_verses) in &book.chapters {
            for (&verse_num, translations) in chapter_verses {
                let mut occurrences = HashMap::new();
                for vt in translations {
                    let occurrence = occurrences.entry(vt.translation_code.as_str()).or_insert(0);
                    *occurrence += 1;
                    // Some sources repeat a verse number for alternate text.
                    // Keep the first ID and give subsequent records distinct IDs.
                    let verse_key = if *occurrence == 1 {
                        verse_num.to_string()
                    } else {
                        format!("{verse_num}::{occurrence}")
                    };
                    let vid = make_id(
                        "verse",
                        &[
                            &book.code,
                            &vt.translation_code,
                            &chap_num.to_string(),
                            &verse_key,
                        ],
                    );
                    let cid = make_id(
                        "chapter",
                        &[&book.code, &vt.translation_code, &chap_num.to_string()],
                    );

                    write_record(
                        &mut vw,
                        &VerseRecord {
                            id: vid.clone(),
                            chapter_id: cid.clone(),
                            number: verse_num,
                            logion_number: vt.logion_number,
                            chunk_type: "single_verse".to_string(),
                            chapter_heading: vt.chapter_heading.clone(),
                            section_heading: vt.section_heading.clone(),
                            paragraph_start: vt.paragraph_start,
                            poetry_indent: vt.poetry_indent,
                            metadata: vt.metadata.clone(),
                        },
                    )?;
                    verse_count += 1;

                    write_record(
                        &mut vcw,
                        &VerseContentRecord {
                            id: make_id(
                                "verse-content",
                                &[
                                    &book.code,
                                    &vt.translation_code,
                                    &chap_num.to_string(),
                                    &verse_key,
                                ],
                            ),
                            verse_id: vid.clone(),
                            translation_id: make_id("translation", &[&vt.translation_code]),
                            language_code: detect_language_code(&vt.translation_code).to_string(),
                            content_type: "text".to_string(),
                            text: vt.text.clone(),
                            normalized_text: None,
                        },
                    )?;
                    verse_content_count += 1;

                    for (idx, word) in vt.words.iter().enumerate() {
                        write_record(
                            &mut itw,
                            &InterlinearTokenRecord {
                                id: make_id(
                                    "interlinear-token",
                                    &[
                                        &book.code,
                                        &vt.translation_code,
                                        &chap_num.to_string(),
                                        &verse_key,
                                        &(idx as u32 + 1).to_string(),
                                    ],
                                ),
                                verse_id: vid.clone(),
                                translation_id: make_id("translation", &[&vt.translation_code]),
                                language_code: detect_language_code(&vt.translation_code)
                                    .to_string(),
                                token_index: idx as u32 + 1,
                                strong: word.strongs.clone(),
                                reference: word.reference.clone(),
                                transliteration: word.transliteration.clone(),
                                original: word.original.clone(),
                                english_gloss: word.english.clone(),
                                grammar: word.grammar.clone(),
                                metadata: None,
                            },
                        )?;
                        interlinear_token_count += 1;
                    }

                    pb.inc(1);

                    ordered_verses.push(VerseForChunk {
                        verse_id: vid,
                        text: vt.text.clone(),
                        book_code: book.code.clone(),
                        translation_code: vt.translation_code.clone(),
                    });
                }
            }
        }
    }
    vw.flush()?;
    vcw.flush()?;
    itw.flush()?;
    finish_ok(
        &pb,
        &format!(
            "verses.jsonl ({verse_count} rows) + verse_contents.jsonl ({verse_content_count} rows) + interlinear_tokens.jsonl ({interlinear_token_count} rows)"
        ),
    );

    // Write chunks.jsonl — sliding window, never crossing book/translation boundaries
    let window = 5;
    let stride = 3;
    let mut actual_chunks: u64 = 0;

    let mut verse_groups: BTreeMap<(String, String), Vec<&VerseForChunk>> = BTreeMap::new();
    for v in &ordered_verses {
        verse_groups
            .entry((v.book_code.clone(), v.translation_code.clone()))
            .or_default()
            .push(v);
    }

    let mut chunk_count: u64 = 0;
    for verses in verse_groups.values() {
        if verses.is_empty() {
            continue;
        }
        let mut i = 0;
        while i < verses.len() {
            chunk_count += 1;
            i += stride;
        }
    }
    let pb = count_bar(chunk_count, "chunks.jsonl");
    let mut cw = open_writer(&ingest_dir.join("chunks.jsonl"))?;

    for ((book_code, translation_code), verses) in &verse_groups {
        if verses.is_empty() {
            continue;
        }
        let mut i = 0;
        let mut chunk_idx = 0u32;
        while i < verses.len() {
            let end = (i + window).min(verses.len());
            let chunk_verses = &verses[i..end];

            let combined_text: String = chunk_verses
                .iter()
                .map(|v| v.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let start_vid = &chunk_verses[0].verse_id;
            let end_vid = &chunk_verses[chunk_verses.len() - 1].verse_id;
            let verse_ids: Vec<String> = chunk_verses.iter().map(|v| v.verse_id.clone()).collect();

            chunk_idx += 1;
            write_record(
                &mut cw,
                &ChunkRecord {
                    id: make_id(
                        "chunk",
                        &[book_code, translation_code, &chunk_idx.to_string()],
                    ),
                    chunk_type: format!("{window}verse"),
                    start_verse_id: start_vid.clone(),
                    end_verse_id: end_vid.clone(),
                    verse_ids,
                    text: combined_text,
                },
            )?;
            actual_chunks += 1;
            pb.inc(1);

            i += stride;
        }
    }
    cw.flush()?;
    finish_ok(&pb, &format!("chunks.jsonl ({actual_chunks} chunks)"));

    eprintln!(
        "✅ Stage 3a complete: {} books, {} translations, {} book_translations, {} chapters, {} verses, {} verse_contents, {} interlinear_tokens, {} chunks",
        books.len(),
        translations.len(),
        total_book_translations,
        total_chapters,
        verse_count,
        verse_content_count,
        interlinear_token_count,
        actual_chunks
    );

    anyhow::ensure!(
        verse_count > 0,
        "No scripture verses found in {}",
        translations_dir.display()
    );
    Ok(())
}
