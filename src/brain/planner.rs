use crate::brain::{
    error::BrainError,
    llm::LlmClient,
    models::Source,
    query_analyzer::{VerseRef, detect_books},
    types::Message,
};
use regex::Regex;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, warn};
use vardadb::{ServerState, storage::codec::Codec};

/// Return the query-planner metadata generated from the active GraphQL
/// schema. Brain uses this only to invoke VardaDB's generic resolver API;
/// domain-specific retrieval criteria remain in the filter it constructs.
pub async fn query_metadata(
    state: &Arc<ServerState>,
) -> std::collections::HashMap<String, vardadb::engine::resolver::QueryTypeMetadata> {
    let Some(schema) = state
        .schemas
        .get(DB_NAME)
        .map(|entry| entry.value().clone())
    else {
        return std::collections::HashMap::new();
    };
    let schema = schema.read().await;
    schema
        .type_metadata
        .iter()
        .map(|(type_name, metadata)| {
            (
                type_name.clone(),
                vardadb::engine::resolver::QueryTypeMetadata {
                    uniques: metadata.uniques.clone(),
                    inverses: metadata.inverses.clone(),
                    relations: metadata.relations.clone(),
                },
            )
        })
        .collect()
}

pub const DB_NAME: &str = "archondb";

#[derive(Clone, Debug, Default)]
pub struct FallbackPlannerOutput {
    pub resolved_query: String,
    pub entity_names: Vec<String>,
    pub strong_numbers: Vec<String>,
    pub key_concepts: Vec<String>,
}

pub async fn call_planner(
    llm: &LlmClient,
    query: &str,
    history: &[Message],
) -> FallbackPlannerOutput {
    let conv_snippet = history
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|m| {
            let prefix = if m.role == "user" { "Q" } else { "A" };
            format!("{}: {}", prefix, m.content)
        })
        .collect::<Vec<_>>()
        .join("\n");

    let prompt = format!(
        "Recent conversation:\n{}\n\nQuestion: {}\n\nReturn JSON with fields resolved_query, entity_names, strong_numbers, key_concepts. Use [] for empty arrays and never infer schema words or book names as entities.",
        conv_snippet, query
    );

    let raw_result = llm.generate_short(&prompt, 300, true).await;

    let raw = match raw_result {
        Ok(raw) => raw,
        Err(e) => {
            warn!("Fallback planner failed: {}", e);
            return regex_fallback(query);
        }
    };

    lenient_parse_fallback(&raw, query)
}

fn lenient_parse_fallback(json_str: &str, original_query: &str) -> FallbackPlannerOutput {
    let extract_string_field = |field: &str| -> Option<String> {
        let pattern = format!(r#""{}"\s*:\s*"([^"\\]*(?:\\.[^"\\]*)*)""#, field);
        Regex::new(&pattern)
            .ok()
            .and_then(|re| re.captures(json_str))
            .and_then(|caps| caps.get(1))
            .map(|m| m.as_str().replace("\\\"", "\"").replace("\\n", "\n"))
    };

    let extract_array_field = |field: &str| -> Vec<String> {
        let pattern = format!(r#""{}"\s*:\s*\[(.*?)\]"#, field);
        let mut values = Vec::new();
        if let Ok(re) = Regex::new(&pattern) {
            if let Some(caps) = re.captures(json_str) {
                if let Some(m) = caps.get(1) {
                    let item_re = Regex::new(r#""([^"\\]*(?:\\.[^"\\]*)*)""#).unwrap();
                    for cap in item_re.captures_iter(m.as_str()) {
                        if let Some(item) = cap.get(1) {
                            let value = item.as_str().trim().to_string();
                            if !value.is_empty() && !values.contains(&value) {
                                values.push(value);
                            }
                        }
                    }
                }
            }
        }
        values
    };

    FallbackPlannerOutput {
        resolved_query: extract_string_field("resolved_query")
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| original_query.to_string()),
        entity_names: extract_array_field("entity_names"),
        strong_numbers: extract_array_field("strong_numbers"),
        key_concepts: extract_array_field("key_concepts"),
    }
}

fn regex_fallback(query: &str) -> FallbackPlannerOutput {
    FallbackPlannerOutput {
        resolved_query: query.to_string(),
        entity_names: extract_name_candidates(query),
        strong_numbers: extract_strongs(query),
        key_concepts: extract_key_concepts_simple(query),
    }
}

fn extract_name_candidates(text: &str) -> Vec<String> {
    let re = match Regex::new(r"\b([A-Z][a-z]{2,})\b") {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for cap in re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            let name = m.as_str().to_string();
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

fn extract_strongs(text: &str) -> Vec<String> {
    let re = match Regex::new(r"\b([HG]\d{1,5})\b") {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for cap in re.captures_iter(text) {
        if let Some(m) = cap.get(1) {
            let strong = m.as_str().to_uppercase();
            if !out.contains(&strong) {
                out.push(strong);
            }
        }
    }
    out
}

fn extract_key_concepts_simple(query: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "what", "why", "who", "where", "when", "how", "did", "does", "was", "were", "are", "is",
        "the", "this", "that", "his", "her", "their", "its", "they", "them", "which", "with",
        "from", "into", "upon", "after", "before", "about", "has", "have", "had", "can", "could",
        "would", "should", "in", "on", "for", "and", "but", "not", "a", "an", "to", "of", "it",
        "do", "be",
    ];

    query
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() > 2 && !STOP.contains(&w.as_str()))
        .fold(Vec::new(), |mut acc, word| {
            if !acc.contains(&word) {
                acc.push(word);
            }
            acc
        })
}

pub async fn fetch_source_from_storage(
    state: &Arc<ServerState>,
    uid: u64,
) -> Result<Source, BrainError> {
    if has_field(state, uid, "text") && has_field(state, uid, "verse") {
        return fetch_source_from_verse_content(state, uid);
    }
    if has_field(state, uid, "chunkType") && has_field(state, uid, "startVerse") {
        return fetch_source_from_chunk(state, uid);
    }
    if let Some(chunk_uid) = get_linked_uid(state, uid, "chunk") {
        return fetch_source_from_chunk(state, chunk_uid);
    }
    if let Some(verse_uid) = get_linked_uid(state, uid, "verse") {
        return fetch_source_from_verse(state, verse_uid, None).await;
    }
    if let Some(source_verse_uid) = get_linked_uid(state, uid, "sourceVerse") {
        return fetch_source_from_verse(state, source_verse_uid, None).await;
    }

    Err(BrainError::Internal(format!(
        "Could not resolve source uid {} to verse content, chunk, verse, or edge",
        uid
    )))
}

pub async fn fetch_source_from_verse(
    state: &Arc<ServerState>,
    verse_uid: u64,
    preferred_translation: Option<&str>,
) -> Result<Source, BrainError> {
    let verse_content_uid = primary_verse_content_uid(state, verse_uid, preferred_translation)
        .ok_or_else(|| {
            BrainError::Internal(format!(
                "No verse content found for verse uid {}",
                verse_uid
            ))
        })?;
    let mut source = fetch_source_from_verse_content(state, verse_content_uid)?;
    source.verse_uid = Some(verse_uid);
    Ok(source)
}

pub fn primary_verse_content_uid(
    state: &Arc<ServerState>,
    verse_uid: u64,
    preferred_translation: Option<&str>,
) -> Option<u64> {
    let Some((main_ks, _)) = state.storage.get_database(DB_NAME) else {
        return None;
    };

    let prefix = Codec::encode_edge_prefix(verse_uid, "verseContents");
    let mut fallback = None;

    for (key, _) in main_ks.prefix(&prefix) {
        if !key.starts_with(&prefix) {
            break;
        }
        let Some(vc_uid) = Codec::decode_edge_source_uid(&key) else {
            continue;
        };

        if fallback.is_none() {
            fallback = Some(vc_uid);
        }

        if let Some(preferred) = preferred_translation {
            if let Some(translation_uid) = get_linked_uid(state, vc_uid, "translation") {
                let code = get_string(state, translation_uid, "code").unwrap_or_default();
                if code.eq_ignore_ascii_case(preferred) {
                    return Some(vc_uid);
                }
            }
        }
    }

    fallback
}

fn fetch_source_from_verse_content(
    state: &Arc<ServerState>,
    uid: u64,
) -> Result<Source, BrainError> {
    let text = get_string(state, uid, "text").unwrap_or_default();
    let verse_uid = get_linked_uid(state, uid, "verse");
    let translation_uid = get_linked_uid(state, uid, "translation");
    let translation = translation_uid
        .and_then(|t_uid| get_string(state, t_uid, "code"))
        .unwrap_or_default();

    let mut book_name = String::new();
    let mut chapter_num = 0;
    let mut verse_num = 0;

    if let Some(v_uid) = verse_uid {
        verse_num = get_i32(state, v_uid, "number").unwrap_or_default();
        if let Some(chapter_uid) = get_linked_uid(state, v_uid, "chapter") {
            chapter_num = get_i32(state, chapter_uid, "number").unwrap_or_default();
            if let Some(book_translation_uid) =
                get_linked_uid(state, chapter_uid, "bookTranslation")
            {
                if let Some(book_uid) = get_linked_uid(state, book_translation_uid, "book") {
                    book_name = get_string(state, book_uid, "nameEn").unwrap_or_default();
                }
            }
        }
    }

    Ok(Source {
        uid,
        chunk_type: "verse".to_string(),
        book_name: book_name.clone(),
        testament: testament_from_book(&book_name).to_string(),
        chapter: chapter_num,
        verse_start: verse_num,
        verse_end: verse_num,
        chapter_heading: None,
        section_heading: None,
        content: text,
        similarity: 0.0,
        translation,
        verse_uid,
        traces: Vec::new(),
        evidence: Vec::new(),
        is_context: false,
    })
}

fn fetch_source_from_chunk(state: &Arc<ServerState>, uid: u64) -> Result<Source, BrainError> {
    let text = get_string(state, uid, "text").unwrap_or_default();
    let chunk_type = get_string(state, uid, "chunkType").unwrap_or_else(|| "chunk".to_string());
    let start_verse_uid = get_linked_uid(state, uid, "startVerse");
    let end_verse_uid = get_linked_uid(state, uid, "endVerse");
    let verse_uid = start_verse_uid;

    let mut book_name = String::new();
    let mut chapter_num = 0;
    let mut verse_start = 0;
    let verse_end;

    if let Some(v_uid) = start_verse_uid {
        verse_start = get_i32(state, v_uid, "number").unwrap_or_default();
        if let Some(chapter_uid) = get_linked_uid(state, v_uid, "chapter") {
            chapter_num = get_i32(state, chapter_uid, "number").unwrap_or_default();
            if let Some(book_translation_uid) =
                get_linked_uid(state, chapter_uid, "bookTranslation")
            {
                if let Some(book_uid) = get_linked_uid(state, book_translation_uid, "book") {
                    book_name = get_string(state, book_uid, "nameEn").unwrap_or_default();
                }
            }
        }
    }

    if let Some(v_uid) = end_verse_uid {
        verse_end = get_i32(state, v_uid, "number").unwrap_or(verse_start);
    } else {
        verse_end = verse_start;
    }

    Ok(Source {
        uid,
        chunk_type,
        book_name: book_name.clone(),
        testament: testament_from_book(&book_name).to_string(),
        chapter: chapter_num,
        verse_start,
        verse_end,
        chapter_heading: None,
        section_heading: None,
        content: text,
        similarity: 0.0,
        translation: String::new(),
        verse_uid,
        traces: Vec::new(),
        evidence: Vec::new(),
        is_context: false,
    })
}

pub fn should_include_verse_filters(
    state: &Arc<ServerState>,
    verse_uid: u64,
    book_filters: &Option<Vec<String>>,
    chapter_filter: Option<i32>,
) -> bool {
    if book_filters.is_none() && chapter_filter.is_none() {
        return true;
    }

    let mut book_name = String::new();
    let mut chapter = None;

    if let Some(chapter_uid) = get_linked_uid(state, verse_uid, "chapter") {
        chapter = get_i32(state, chapter_uid, "number");
        if let Some(book_translation_uid) = get_linked_uid(state, chapter_uid, "bookTranslation") {
            if let Some(book_uid) = get_linked_uid(state, book_translation_uid, "book") {
                book_name = get_string(state, book_uid, "nameEn").unwrap_or_default();
            }
        }
    }

    if let Some(books) = book_filters {
        let lower = book_name.to_lowercase();
        if !books
            .iter()
            .any(|book| lower.contains(&book.to_lowercase()))
        {
            return false;
        }
    }

    if let Some(chapter_filter) = chapter_filter {
        if chapter.unwrap_or_default() != chapter_filter {
            return false;
        }
    }

    true
}

pub async fn expand_sources_with_context(
    state: &Arc<ServerState>,
    sources: &[Source],
    window: usize,
) -> Vec<Source> {
    let mut seen: HashSet<(String, i32, i32, i32, String, String)> =
        sources.iter().map(Source::dedupe_key).collect();
    let mut expanded = sources.to_vec();

    for source in sources {
        let Some(verse_uid) = source.verse_uid else {
            continue;
        };
        let Some(chapter_uid) = get_linked_uid(state, verse_uid, "chapter") else {
            continue;
        };

        let lo = (source.verse_start - window as i32).max(1);
        let hi = source.verse_end + window as i32;

        for nearby_verse_uid in verses_in_chapter_range(state, chapter_uid, lo, hi) {
            let Ok(mut nearby) =
                fetch_source_from_verse(state, nearby_verse_uid, Some(&source.translation)).await
            else {
                continue;
            };

            let key = nearby.dedupe_key();
            if !seen.insert(key) {
                continue;
            }

            nearby.similarity = 0.0;
            nearby.is_context = true;
            expanded.push(nearby);
        }
    }

    expanded
}

pub async fn fetch_sources_for_verse_refs(
    state: &Arc<ServerState>,
    refs: &[VerseRef],
    preferred_translation: Option<&str>,
) -> Vec<Source> {
    let mut out = Vec::new();
    for verse_ref in refs {
        for verse_uid in find_verse_uids(state, verse_ref) {
            if let Ok(source) =
                fetch_source_from_verse(state, verse_uid, preferred_translation).await
            {
                out.push(source);
            }
        }
    }
    out
}

pub async fn fetch_sources_for_chapter(
    state: &Arc<ServerState>,
    books: &[String],
    chapter: i32,
    preferred_translation: Option<&str>,
) -> Vec<Source> {
    let Some((main_ks, _)) = state.storage.get_database(DB_NAME) else {
        return Vec::new();
    };

    let mut out = Vec::new();

    for book in books {
        for book_uid in find_book_uids(state, book) {
            let book_translation_prefix = Codec::encode_edge_prefix(book_uid, "bookTranslations");
            for (bt_key, _) in main_ks.prefix(&book_translation_prefix) {
                if !bt_key.starts_with(&book_translation_prefix) {
                    break;
                }
                let Some(book_translation_uid) = Codec::decode_edge_source_uid(&bt_key) else {
                    continue;
                };

                let chapter_prefix = Codec::encode_edge_prefix(book_translation_uid, "chapters");
                for (chapter_key, _) in main_ks.prefix(&chapter_prefix) {
                    if !chapter_key.starts_with(&chapter_prefix) {
                        break;
                    }
                    let Some(chapter_uid) = Codec::decode_edge_source_uid(&chapter_key) else {
                        continue;
                    };
                    if get_i32(state, chapter_uid, "number") != Some(chapter) {
                        continue;
                    }

                    for verse_uid in verses_in_chapter_range(state, chapter_uid, 1, i32::MAX) {
                        if let Ok(source) =
                            fetch_source_from_verse(state, verse_uid, preferred_translation).await
                        {
                            out.push(source);
                        }
                    }
                }
            }
        }
    }

    out
}

fn find_verse_uids(state: &Arc<ServerState>, verse_ref: &VerseRef) -> Vec<u64> {
    let Some((main_ks, _)) = state.storage.get_database(DB_NAME) else {
        return Vec::new();
    };

    for book_uid in find_book_uids(state, &verse_ref.book) {
        let book_translation_prefix = Codec::encode_edge_prefix(book_uid, "bookTranslations");
        for (bt_key, _) in main_ks.prefix(&book_translation_prefix) {
            if !bt_key.starts_with(&book_translation_prefix) {
                break;
            }
            let Some(bt_uid) = Codec::decode_edge_source_uid(&bt_key) else {
                continue;
            };

            let chapter_prefix = Codec::encode_edge_prefix(bt_uid, "chapters");
            for (chapter_key, _) in main_ks.prefix(&chapter_prefix) {
                if !chapter_key.starts_with(&chapter_prefix) {
                    break;
                }
                let Some(chapter_uid) = Codec::decode_edge_source_uid(&chapter_key) else {
                    continue;
                };
                if get_i32(state, chapter_uid, "number") != Some(verse_ref.chapter) {
                    continue;
                }

                return verses_in_chapter_range(
                    state,
                    chapter_uid,
                    verse_ref.verse_start,
                    verse_ref.verse_end.unwrap_or(verse_ref.verse_start),
                );
            }
        }
    }

    Vec::new()
}

fn verses_in_chapter_range(
    state: &Arc<ServerState>,
    chapter_uid: u64,
    lo: i32,
    hi: i32,
) -> Vec<u64> {
    let Some((main_ks, _)) = state.storage.get_database(DB_NAME) else {
        return Vec::new();
    };

    let prefix = Codec::encode_edge_prefix(chapter_uid, "verses");
    let mut verses = Vec::new();
    for (key, _) in main_ks.prefix(&prefix) {
        if !key.starts_with(&prefix) {
            break;
        }
        let Some(verse_uid) = Codec::decode_edge_source_uid(&key) else {
            continue;
        };
        let verse_number = get_i32(state, verse_uid, "number").unwrap_or_default();
        if verse_number >= lo && verse_number <= hi {
            verses.push(verse_uid);
        }
    }
    verses.sort_by_key(|uid| get_i32(state, *uid, "number").unwrap_or_default());
    verses
}

pub fn uids_for_type(state: &Arc<ServerState>, type_name: &str) -> Vec<u64> {
    let Some((main_ks, _)) = state.storage.get_database(DB_NAME) else {
        return Vec::new();
    };

    let prefix = Codec::encode_type_prefix(type_name);
    let mut out = Vec::new();
    for (key, _) in main_ks.prefix(&prefix) {
        if !key.starts_with(&prefix) || key.len() < prefix.len() + 8 {
            break;
        }
        if let Some(uid) = Codec::decode_edge_source_uid(&key) {
            out.push(uid);
        }
    }
    out
}

fn find_book_uids(state: &Arc<ServerState>, query: &str) -> Vec<u64> {
    let normalized_query = normalize_book_token(query);
    if normalized_query.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for book_uid in uids_for_type(state, "Book") {
        if book_matches_query(state, book_uid, &normalized_query) && !out.contains(&book_uid) {
            out.push(book_uid);
        }
    }

    out
}

fn book_matches_query(state: &Arc<ServerState>, book_uid: u64, normalized_query: &str) -> bool {
    let candidates = [
        get_string(state, book_uid, "nameEn"),
        get_string(state, book_uid, "nameHe"),
        get_string(state, book_uid, "nameGr"),
        get_string(state, book_uid, "code"),
    ];

    candidates.iter().flatten().any(|candidate| {
        let normalized_candidate = normalize_book_token(candidate);
        normalized_candidate == normalized_query
            || normalized_candidate.starts_with(normalized_query)
            || normalized_query.starts_with(&normalized_candidate)
    })
}

fn normalize_book_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn testament_from_book(book: &str) -> &'static str {
    const NT: &[&str] = &[
        "Matthew",
        "Mark",
        "Luke",
        "John",
        "Acts",
        "Romans",
        "1 Corinthians",
        "2 Corinthians",
        "Galatians",
        "Ephesians",
        "Philippians",
        "Colossians",
        "1 Thessalonians",
        "2 Thessalonians",
        "1 Timothy",
        "2 Timothy",
        "Titus",
        "Philemon",
        "Hebrews",
        "James",
        "1 Peter",
        "2 Peter",
        "1 John",
        "2 John",
        "3 John",
        "Jude",
        "Revelation",
    ];

    if NT.contains(&book) { "NT" } else { "OT" }
}

pub fn has_field(state: &Arc<ServerState>, uid: u64, field: &str) -> bool {
    let key = Codec::encode_data_key(uid, field);
    matches!(state.storage.get(DB_NAME, &key), Ok(Some(_)))
}

pub fn get_linked_uid(state: &Arc<ServerState>, uid: u64, field: &str) -> Option<u64> {
    let key = Codec::encode_data_key(uid, field);
    if let Ok(Some(bytes)) = state.storage.get(DB_NAME, &key) {
        if let Some(value) = decode_storage_value(&bytes) {
            return match value {
                serde_json::Value::String(s) => s.parse::<u64>().ok(),
                serde_json::Value::Number(n) => n.as_u64(),
                _ => None,
            };
        }
    }
    None
}

pub fn get_string(state: &Arc<ServerState>, uid: u64, field: &str) -> Option<String> {
    let key = Codec::encode_data_key(uid, field);
    if let Ok(Some(bytes)) = state.storage.get(DB_NAME, &key) {
        if let Some(value) = decode_storage_value(&bytes) {
            return match value {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            };
        }
    }
    None
}

pub fn get_i32(state: &Arc<ServerState>, uid: u64, field: &str) -> Option<i32> {
    let key = Codec::encode_data_key(uid, field);
    if let Ok(Some(bytes)) = state.storage.get(DB_NAME, &key) {
        if let Some(value) = decode_storage_value(&bytes) {
            return match value {
                serde_json::Value::String(s) => s.parse::<i32>().ok(),
                serde_json::Value::Number(n) => n.as_i64().map(|v| v as i32),
                _ => None,
            };
        }
    }
    None
}

pub fn get_number(state: &Arc<ServerState>, uid: u64, field: &str) -> Option<f64> {
    let key = Codec::encode_data_key(uid, field);
    if let Ok(Some(bytes)) = state.storage.get(DB_NAME, &key) {
        if let Some(value) = decode_storage_value(&bytes) {
            return match value {
                serde_json::Value::String(s) => s.parse::<f64>().ok(),
                serde_json::Value::Number(n) => n.as_f64(),
                _ => None,
            };
        }
    }
    None
}

pub fn edge_targets(state: &Arc<ServerState>, uid: u64, field: &str) -> Vec<u64> {
    let Some((main_ks, _)) = state.storage.get_database(DB_NAME) else {
        return Vec::new();
    };
    let prefix = Codec::encode_edge_prefix(uid, field);
    let mut out = Vec::new();
    for (key, _) in main_ks.prefix(&prefix) {
        if !key.starts_with(&prefix) {
            break;
        }
        if let Some(target_uid) = Codec::decode_edge_source_uid(&key) {
            out.push(target_uid);
        }
    }
    out
}

fn decode_storage_value(bytes: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_slice(bytes).ok().or_else(|| {
        if bytes.len() > 16 {
            serde_json::from_slice(&bytes[16..]).ok()
        } else {
            None
        }
    })
}

pub fn warn_if_query_has_no_book_match(query: &str) {
    if detect_books(query).is_none() {
        debug!("No explicit book filters detected in query '{}'", query);
    }
}
