//! Query Analyzer — extracts entities, references, intent, and key concepts
//! from user questions BEFORE any search runs.
//!
//! This replaces the inline pronoun resolution and Genesis-specific regex
//! scattered across reasoning.rs and planner.rs.

use crate::brain::{error::BrainError, llm::LlmClient, planner::DB_NAME, types::Message};
use regex::Regex;
use serde::{Deserialize, Serialize};

use std::sync::Arc;
use tracing::{debug, info, warn};
use vardadb::{ServerState, bridge::sqlite_resolver::SqliteResolver, storage::codec::Codec};

// ── Public Types ───────────────────────────────────────────────────────────

/// The full result of analyzing a user query.
#[derive(Debug, Clone)]
pub struct QueryAnalysis {
    /// Pronoun-resolved, self-contained query text
    pub resolved_query: String,
    /// People, places, groups detected in the query or resolved from context
    pub detected_entities: Vec<DetectedEntity>,
    /// Book filters extracted from the query (e.g., "Genesis")
    pub book_filters: Option<Vec<String>>,
    /// Chapter filter extracted from the query (e.g., 6 from "Genesis 6")
    pub chapter_filter: Option<i32>,
    /// Explicit verse references (e.g., "Gen 6:5")
    pub verse_refs: Vec<VerseRef>,
    /// Classified intent of the query
    pub query_intent: QueryIntent,
    /// Key concepts for synonym expansion (e.g., "flood", "favor", "worship")
    pub key_concepts: Vec<String>,
    /// Explicit or inferred Strong's identifiers
    pub strong_numbers: Vec<String>,
    /// Lemma terms to use for lexical retrieval
    pub lemma_terms: Vec<String>,
    /// Synset identifiers inferred from query concepts
    pub synset_ids: Vec<String>,
    /// Human-readable synset gloss cues for retrieval/synthesis
    pub synset_glosses: Vec<String>,
    /// Conversation-derived state for multi-turn narrative follow-ups
    pub narrative_state: NarrativeState,
}

#[derive(Debug, Clone)]
pub struct DetectedEntity {
    /// Display name (e.g., "Noah", "God")
    pub name: String,
    /// Entity type from NER (e.g., "PERSON", "DEITY", "PLACE")
    pub entity_type: String,
    /// True if resolved from parent conversation, not the current query
    pub from_context: bool,
    /// UID of the matching `Entities` node in VardaDB, if found
    pub db_entity_uid: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerseRef {
    pub book: String,
    pub chapter: i32,
    pub verse_start: i32,
    pub verse_end: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NarrativeDirection {
    Same,
    Forward,
    Backward,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeState {
    pub books: Vec<String>,
    pub chapter_hint: Option<i32>,
    pub chapter_window: Option<(i32, i32)>,
    pub anchor_refs: Vec<VerseRef>,
    pub anchor_entities: Vec<String>,
    pub anchor_terms: Vec<String>,
    pub temporal_direction: NarrativeDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum QueryIntent {
    /// "What reason…", "Why…"
    FindReason,
    /// "Who…"
    IdentifyPerson,
    /// "What did X do…", "What was the first thing…"
    DescribeAction,
    /// "What happened…", "Where…"
    DescribeEvent,
    /// "How did…"
    DescribeMethod,
    /// Anything else
    Other,
}

// ── Bible Book Patterns ────────────────────────────────────────────────────

/// (lowercase pattern, canonical name, abbreviation for verse refs)
const BIBLE_BOOKS: &[(&str, &str, &str)] = &[
    ("genesis", "Genesis", "gen"),
    ("exodus", "Exodus", "exod"),
    ("leviticus", "Leviticus", "lev"),
    ("numbers", "Numbers", "num"),
    ("deuteronomy", "Deuteronomy", "deut"),
    ("joshua", "Joshua", "josh"),
    ("judges", "Judges", "judg"),
    ("ruth", "Ruth", "ruth"),
    ("1 samuel", "1 Samuel", "1sam"),
    ("2 samuel", "2 Samuel", "2sam"),
    ("1 kings", "1 Kings", "1kgs"),
    ("2 kings", "2 Kings", "2kgs"),
    ("1 chronicles", "1 Chronicles", "1chr"),
    ("2 chronicles", "2 Chronicles", "2chr"),
    ("ezra", "Ezra", "ezra"),
    ("nehemiah", "Nehemiah", "neh"),
    ("esther", "Esther", "esth"),
    ("job", "Job", "job"),
    ("psalm", "Psalms", "ps"),
    ("psalms", "Psalms", "ps"),
    ("proverbs", "Proverbs", "prov"),
    ("ecclesiastes", "Ecclesiastes", "eccl"),
    ("song of solomon", "Song of Solomon", "song"),
    ("song of songs", "Song of Solomon", "song"),
    ("isaiah", "Isaiah", "isa"),
    ("jeremiah", "Jeremiah", "jer"),
    ("lamentations", "Lamentations", "lam"),
    ("ezekiel", "Ezekiel", "ezek"),
    ("daniel", "Daniel", "dan"),
    ("hosea", "Hosea", "hos"),
    ("joel", "Joel", "joel"),
    ("amos", "Amos", "amos"),
    ("obadiah", "Obadiah", "obad"),
    ("jonah", "Jonah", "jonah"),
    ("micah", "Micah", "mic"),
    ("nahum", "Nahum", "nah"),
    ("habakkuk", "Habakkuk", "hab"),
    ("zephaniah", "Zephaniah", "zeph"),
    ("haggai", "Haggai", "hag"),
    ("zechariah", "Zechariah", "zech"),
    ("malachi", "Malachi", "mal"),
    ("matthew", "Matthew", "matt"),
    ("mark", "Mark", "mark"),
    ("luke", "Luke", "luke"),
    ("john", "John", "john"),
    ("acts", "Acts", "acts"),
    ("romans", "Romans", "rom"),
    ("1 corinthians", "1 Corinthians", "1cor"),
    ("2 corinthians", "2 Corinthians", "2cor"),
    ("galatians", "Galatians", "gal"),
    ("ephesians", "Ephesians", "eph"),
    ("philippians", "Philippians", "phil"),
    ("colossians", "Colossians", "col"),
    ("1 thessalonians", "1 Thessalonians", "1thess"),
    ("2 thessalonians", "2 Thessalonians", "2thess"),
    ("1 timothy", "1 Timothy", "1tim"),
    ("2 timothy", "2 Timothy", "2tim"),
    ("titus", "Titus", "titus"),
    ("philemon", "Philemon", "phlm"),
    ("hebrews", "Hebrews", "heb"),
    ("james", "James", "jas"),
    ("1 peter", "1 Peter", "1pet"),
    ("2 peter", "2 Peter", "2pet"),
    ("1 john", "1 John", "1john"),
    ("2 john", "2 John", "2john"),
    ("3 john", "3 John", "3john"),
    ("jude", "Jude", "jude"),
    ("revelation", "Revelation", "rev"),
];

// ── Main API ───────────────────────────────────────────────────────────────

/// Analyze a user query, extracting everything needed for NLP-aware retrieval.
pub async fn analyze_query(
    state: &Arc<ServerState>,
    query: &str,
    history: &[Message],
    llm_client: &LlmClient,
) -> Result<QueryAnalysis, BrainError> {
    info!("Analyzing query: '{}'", query);

    // 1. Resolve pronouns if conversation history exists
    let resolved_query = if !history.is_empty() {
        resolve_pronouns(query, history, llm_client).await?
    } else {
        query.to_string()
    };

    // 2. Detect book references
    let mut book_filters = detect_books(&resolved_query);

    // 3. Extract chapter references
    let mut chapter_filter = extract_chapter(&resolved_query);

    // 4. Extract explicit verse references from the current query only.
    // Pulling verse refs out of prior assistant answers over-anchors follow-up
    // questions to earlier citations and blocks later narrative progression.
    let mut verse_refs = extract_verse_refs(&resolved_query);
    // Dedup
    verse_refs.sort_by(|a, b| {
        (&a.book, a.chapter, a.verse_start).cmp(&(&b.book, b.chapter, b.verse_start))
    });
    verse_refs.dedup_by(|a, b| {
        a.book == b.book && a.chapter == b.chapter && a.verse_start == b.verse_start
    });

    // 5. Classify query intent
    let query_intent = classify_intent(&resolved_query);

    // 6. Detect entities in the query by looking up VardaDB's Entities table
    let mut detected_entities = detect_entities_from_db(state, &resolved_query).await;

    // 7. Also carry entities from conversation history (for child/grandchild prompts)
    if !history.is_empty() {
        let context_entities = extract_entities_from_history(state, history).await;
        for ent in context_entities {
            // Don't duplicate entities already found in the current query
            if !detected_entities
                .iter()
                .any(|e| e.name.to_lowercase() == ent.name.to_lowercase())
            {
                detected_entities.push(ent);
            }
        }
    }

    // 8. Extract key concepts for synonym expansion
    let mut key_concepts = extract_key_concepts(&resolved_query, &detected_entities);
    if let Some(books) = &book_filters {
        let lower_books: Vec<String> = books.iter().map(|book| book.to_lowercase()).collect();
        key_concepts.retain(|concept| !lower_books.iter().any(|book| book.contains(concept)));
    }

    let narrative_state = infer_narrative_state(
        &resolved_query,
        history,
        &verse_refs,
        &book_filters,
        chapter_filter,
    );

    if book_filters.is_none() && !narrative_state.books.is_empty() {
        book_filters = Some(narrative_state.books.clone());
    }
    if chapter_filter.is_none() {
        chapter_filter = narrative_state.chapter_hint;
    }

    let explicit_strongs = extract_explicit_strongs(&resolved_query);
    let (strong_numbers, lemma_terms) =
        expand_lexical_candidates(state, &key_concepts, &explicit_strongs).await;
    let (synset_ids, synset_glosses) =
        expand_synset_candidates(state, &key_concepts, &lemma_terms).await;

    let analysis = QueryAnalysis {
        resolved_query: resolved_query.clone(),
        detected_entities,
        book_filters,
        chapter_filter,
        verse_refs,
        query_intent,
        key_concepts,
        strong_numbers,
        lemma_terms,
        synset_ids,
        synset_glosses,
        narrative_state,
    };

    debug!("Query analysis: {:?}", analysis);
    info!(
        "Analysis: intent={:?}, entities={}, books={:?}, chapter={:?}, concepts={}, strongs={}, synsets={}",
        analysis.query_intent,
        analysis.detected_entities.len(),
        analysis.book_filters,
        analysis.chapter_filter,
        analysis.key_concepts.join(", "),
        analysis.strong_numbers.join(", "),
        analysis.synset_ids.join(", ")
    );

    Ok(analysis)
}

// ── Pronoun Resolution ─────────────────────────────────────────────────────

/// Rewrite a follow-up question into a self-contained query by resolving
/// pronouns from the conversation history.
async fn resolve_pronouns(
    query: &str,
    history: &[Message],
    llm_client: &LlmClient,
) -> Result<String, BrainError> {
    // Build condensed context: full questions, opening snippet of each answer.
    // Answers often open by echoing the question, so take enough of the body
    // to carry the actual names/places needed for pronoun resolution.
    let context_summary = history
        .iter()
        .map(|m| {
            if m.role == "user" {
                format!("Q: {}", m.content)
            } else {
                let plain = m.content.replace("**", "").replace("*", "");
                let body = plain
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let body = body.trim_start_matches("Answer:").trim();
                let snippet: String = body.chars().take(280).collect();
                format!("A: {}", snippet)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    let resolve_prompt = format!(
        "Rewrite this follow-up question as a self-contained question. Replace ALL pronouns with the actual names from the conversation. Output ONLY the rewritten question, do not add explanation.\n\nConversation:\n{}\n\nFollow-up: {}\n\nRewritten:",
        context_summary, query
    );

    let result = llm_client.generate_short(&resolve_prompt, 100, false).await;

    match result {
        Ok(resolved) => {
            // Strip Qwen3 <think>...</think> tags
            let cleaned = if let Some(end_pos) = resolved.find("</think>") {
                resolved[end_pos + 8..].trim().to_string()
            } else {
                let s = resolved.trim();
                if s.starts_with("<think>") {
                    String::new()
                } else {
                    s.to_string()
                }
            };
            let resolved = cleaned
                .trim()
                .trim_matches('"')
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();

            if resolved.is_empty() {
                debug!("Resolution produced no usable output, using original query");
                Ok(query.to_string())
            } else {
                // Hallucination guard: a valid rewrite only swaps pronouns for
                // names/terms already present in the query or conversation.
                // Reject rewrites that introduce new content words (e.g. a
                // hallucinated "John asked Michael") — those poison book
                // scoping downstream.
                let context_corpus = format!("{} {}", context_summary, query).to_lowercase();
                let introduced = resolved
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| w.len() >= 4)
                    .any(|w| !context_corpus.contains(&w.to_lowercase()));
                if introduced {
                    info!(
                        "Rewrite introduced unknown terms ('{}' → '{}'), using original query",
                        query, resolved
                    );
                    return Ok(query.to_string());
                }

                info!("Resolved query: '{}' → '{}'", query, resolved);
                Ok(resolved)
            }
        }
        Err(e) => {
            warn!("Query resolution failed: {}, using original", e);
            Ok(query.to_string())
        }
    }
}

// ── Book Detection ─────────────────────────────────────────────────────────

/// Detect Bible book names in text. Returns canonical names.
pub fn detect_books(text: &str) -> Option<Vec<String>> {
    let lower = text.to_lowercase();
    let mut matches: Vec<String> = BIBLE_BOOKS
        .iter()
        .filter_map(|(needle, canonical, _)| {
            if lower.contains(needle) {
                Some(canonical.to_string())
            } else {
                None
            }
        })
        .collect();
    matches.sort();
    matches.dedup();
    if matches.is_empty() {
        None
    } else {
        Some(matches)
    }
}

// ── Chapter Extraction ─────────────────────────────────────────────────────

/// Extract chapter number from queries like "in Genesis 6" or "Genesis chapter 3"
fn extract_chapter(text: &str) -> Option<i32> {
    // Build a pattern that matches any book name followed by a chapter number
    // e.g., "Genesis 6", "Gen 3", "1 Samuel 17"
    let book_patterns: Vec<String> = BIBLE_BOOKS
        .iter()
        .map(|(needle, _, abbrev)| {
            // Escape special regex characters and create alternatives
            let escaped = regex::escape(needle);
            let abbrev_escaped = regex::escape(abbrev);
            format!("{}|{}", escaped, abbrev_escaped)
        })
        .collect();
    let combined = book_patterns.join("|");
    let pattern = format!(r"(?i)(?:{})\s+(?:chapter\s+)?(\d+)", combined);

    if let Ok(re) = Regex::new(&pattern) {
        if let Some(cap) = re.captures(text) {
            if let Some(ch) = cap.get(1) {
                return ch.as_str().parse::<i32>().ok().filter(|&c| c > 0);
            }
        }
    }
    None
}

// ── Verse Reference Extraction ─────────────────────────────────────────────

/// Extract explicit verse references like "Gen 6:5", "Genesis 37:28", "Gen 6:5-7"
fn extract_verse_refs(text: &str) -> Vec<VerseRef> {
    let mut refs = Vec::new();

    // Build a pattern for all book names/abbreviations
    let book_patterns: Vec<String> = BIBLE_BOOKS
        .iter()
        .flat_map(|(needle, canonical, abbrev)| {
            vec![
                (regex::escape(needle), canonical.to_string()),
                (regex::escape(abbrev), canonical.to_string()),
            ]
        })
        .map(|(pat, _)| pat)
        .collect();

    // Sort by length descending so longer patterns match first
    let mut sorted_patterns = book_patterns.clone();
    sorted_patterns.sort_by(|a, b| b.len().cmp(&a.len()));
    let combined = sorted_patterns.join("|");

    let pattern = format!(r"(?i)({})\s+(\d+):(\d+)(?:[–\-](\d+))?", combined);

    if let Ok(re) = Regex::new(&pattern) {
        for cap in re.captures_iter(text) {
            if let (Some(book_match), Some(ch), Some(vs)) = (cap.get(1), cap.get(2), cap.get(3)) {
                let book_text = book_match.as_str().to_lowercase();
                // Map to canonical name
                let canonical = BIBLE_BOOKS
                    .iter()
                    .find(|(needle, _, abbrev)| book_text == *needle || book_text == *abbrev)
                    .map(|(_, canonical, _)| canonical.to_string())
                    .unwrap_or_else(|| book_match.as_str().to_string());

                let chapter = ch.as_str().parse::<i32>().unwrap_or(0);
                let verse_start = vs.as_str().parse::<i32>().unwrap_or(0);
                let verse_end = cap.get(4).and_then(|ve| ve.as_str().parse::<i32>().ok());

                if chapter > 0 && verse_start > 0 {
                    refs.push(VerseRef {
                        book: canonical,
                        chapter,
                        verse_start,
                        verse_end,
                    });
                }
            }
        }
    }

    refs
}

fn infer_narrative_state(
    current_query: &str,
    history: &[Message],
    current_refs: &[VerseRef],
    _detected_books: &Option<Vec<String>>,
    detected_chapter: Option<i32>,
) -> NarrativeState {
    let temporal_direction = infer_temporal_direction(current_query);
    // Anchor books on the USER's own questions first — assistant answers cite
    // many books and poison the scope of follow-up questions (e.g. a Mark 4
    // follow-up inheriting "John" from citations in the previous answer).
    let mut books = history
        .iter()
        .rev()
        .find_map(|msg| {
            if msg.role == "user" {
                detect_books(&msg.content)
            } else {
                None
            }
        })
        .or_else(|| {
            history.iter().rev().find_map(|msg| {
                if msg.role != "user" {
                    detect_books(&msg.content)
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();
    let mut anchor_refs = current_refs.to_vec();
    let mut anchor_entities = extract_name_candidates(current_query);
    let mut anchor_terms = tokenize_history_terms(current_query);

    for msg in history.iter().rev() {
        if books.is_empty()
            && let Some(history_books) = detect_books(&msg.content)
        {
            books = history_books;
        }

        let refs = extract_verse_refs(&msg.content);
        for verse_ref in refs.into_iter().rev() {
            if !anchor_refs.iter().any(|existing| {
                existing.book == verse_ref.book
                    && existing.chapter == verse_ref.chapter
                    && existing.verse_start == verse_ref.verse_start
                    && existing.verse_end == verse_ref.verse_end
            }) {
                anchor_refs.push(verse_ref);
            }
        }

        for entity in extract_name_candidates(&msg.content) {
            if !anchor_entities.iter().any(|existing| existing == &entity) {
                anchor_entities.push(entity);
            }
        }

        for term in tokenize_history_terms(&msg.content) {
            if !anchor_terms.iter().any(|existing| existing == &term) {
                anchor_terms.push(term);
            }
        }
    }

    let anchor_chapter = match temporal_direction {
        NarrativeDirection::Forward => anchor_refs.iter().map(|verse_ref| verse_ref.chapter).max(),
        NarrativeDirection::Backward => anchor_refs.iter().map(|verse_ref| verse_ref.chapter).min(),
        _ => anchor_refs.last().map(|verse_ref| verse_ref.chapter),
    };

    let chapter_hint = detected_chapter
        .or_else(|| current_refs.last().map(|verse_ref| verse_ref.chapter))
        .or(anchor_chapter)
        .or_else(|| {
            history
                .iter()
                .rev()
                .find_map(|msg| extract_chapter(&msg.content))
        });

    let chapter_window = match (detected_chapter, chapter_hint, &temporal_direction) {
        (Some(chapter), _, _) => Some((chapter, chapter)),
        (None, Some(chapter), NarrativeDirection::Forward) => Some((chapter, chapter + 2)),
        (None, Some(chapter), NarrativeDirection::Backward) => {
            Some(((chapter - 2).max(1), chapter))
        }
        (None, Some(chapter), NarrativeDirection::Same) => Some((chapter, chapter)),
        _ => None,
    };

    NarrativeState {
        books,
        chapter_hint,
        chapter_window,
        anchor_refs,
        anchor_entities,
        anchor_terms,
        temporal_direction,
    }
}

fn infer_temporal_direction(query: &str) -> NarrativeDirection {
    let lower = query.to_lowercase();

    if [
        "after ",
        "afterward",
        "later ",
        "next ",
        "then ",
        "first thing",
        "following ",
        "once ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return NarrativeDirection::Forward;
    }

    if ["before ", "earlier ", "previously", "prior to "]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return NarrativeDirection::Backward;
    }

    if ["same chapter", "in this passage", "in these verses"]
        .iter()
        .any(|needle| lower.contains(needle))
    {
        return NarrativeDirection::Same;
    }

    NarrativeDirection::Unknown
}

fn tokenize_history_terms(text: &str) -> Vec<String> {
    let stop_words: std::collections::HashSet<&str> = [
        "the", "and", "for", "that", "with", "from", "into", "upon", "after", "before", "what",
        "who", "why", "how", "when", "where", "this", "these", "those", "them", "they", "their",
        "his", "her", "was", "were", "did", "does", "have", "has", "had", "not", "you", "your",
        "god", "noah",
    ]
    .iter()
    .copied()
    .collect();

    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| part.len() >= 4 && !stop_words.contains(part.as_str()))
        .fold(Vec::new(), |mut acc, token| {
            if !acc.contains(&token) {
                acc.push(token);
            }
            acc
        })
}

// ── Intent Classification ──────────────────────────────────────────────────

/// Classify the user's query intent based on question words and patterns.
fn classify_intent(query: &str) -> QueryIntent {
    let lower = query.to_lowercase();

    // Check patterns in order of specificity
    if lower.starts_with("why ")
        || lower.contains("what reason")
        || lower.contains("for what reason")
    {
        return QueryIntent::FindReason;
    }

    if lower.starts_with("who ") || lower.contains("who was") || lower.contains("who did") {
        return QueryIntent::IdentifyPerson;
    }

    if lower.starts_with("how did") || lower.starts_with("how was") || lower.starts_with("how does")
    {
        return QueryIntent::DescribeMethod;
    }

    if lower.contains("what did")
        || lower.contains("what was the first")
        || lower.contains("what action")
        || lower.contains("what specific")
    {
        return QueryIntent::DescribeAction;
    }

    if lower.starts_with("what ") || lower.starts_with("where ") || lower.starts_with("when ") {
        return QueryIntent::DescribeEvent;
    }

    QueryIntent::Other
}

// ── Entity Detection from VardaDB ──────────────────────────────────────────

/// Look up known entity names in VardaDB's Entities table.
async fn detect_entities_from_db(state: &Arc<ServerState>, query: &str) -> Vec<DetectedEntity> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut found = Vec::new();

    // Extract capitalized words as potential entity name candidates
    let candidates = extract_name_candidates(query);

    for candidate in &candidates {
        // Search for entity by name using BM25 on Entities.name (term index)
        let results = resolver.search_text_bm25(candidate, "name", "term", 5, true);

        for (uid, _score) in results {
            // Verify this is an Entities node and extract type
            let get_val = |target_uid: u64, field: &str| -> Option<serde_json::Value> {
                let key = Codec::encode_data_key(target_uid, field);
                if let Ok(Some(bytes)) = state.storage.get(DB_NAME, &key) {
                    return decode_storage_value(&bytes);
                }
                None
            };

            if let Some(serde_json::Value::String(name)) = get_val(uid, "name") {
                // Check for exact or close match
                if name.to_lowercase() == candidate.to_lowercase() {
                    let entity_type = match get_val(uid, "entityType") {
                        Some(serde_json::Value::String(t)) => t,
                        _ => "UNKNOWN".to_string(),
                    };

                    // Don't add duplicates
                    if !found.iter().any(|e: &DetectedEntity| e.name == name) {
                        found.push(DetectedEntity {
                            name,
                            entity_type,
                            from_context: false,
                            db_entity_uid: Some(uid),
                        });
                    }
                }
            }
        }
    }

    debug!(
        "Detected {} entities from DB for query: {:?}",
        found.len(),
        found.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
    found
}

/// Extract capitalized words as potential entity name candidates.
fn extract_name_candidates(text: &str) -> Vec<String> {
    let stop_words: std::collections::HashSet<&str> = [
        "What", "Why", "Who", "Where", "When", "How", "Did", "Does", "Was", "Were", "Are", "Is",
        "The", "This", "That", "His", "Her", "Their", "Its", "They", "Them", "Which", "With",
        "From", "Into", "Upon", "After", "Before", "During", "About", "Has", "Have", "Had", "Can",
        "Could", "Would", "Should", "May", "In", "On", "For", "And", "But", "Not", "Given",
        "First",
    ]
    .iter()
    .cloned()
    .collect();

    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|word| {
            let first_char = word.chars().next();
            word.len() > 1
                && first_char.map_or(false, |c| c.is_uppercase())
                && !stop_words.contains(word)
        })
        .map(|w| w.to_string())
        .collect()
}

/// Extract entity names from conversation history answers.
/// Looks for entities mentioned in the AI's previous answers.
async fn extract_entities_from_history(
    state: &Arc<ServerState>,
    history: &[Message],
) -> Vec<DetectedEntity> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut entities = Vec::new();

    for msg in history {
        if msg.role != "assistant" {
            continue;
        }

        // Extract capitalized proper nouns from the answer
        let candidates = extract_name_candidates(&msg.content);

        // Look up each candidate in VardaDB
        for candidate in &candidates {
            let results = resolver.search_text_bm25(candidate, "name", "term", 3, true);

            for (uid, _score) in results {
                let get_val = |target_uid: u64, field: &str| -> Option<serde_json::Value> {
                    let key = Codec::encode_data_key(target_uid, field);
                    if let Ok(Some(bytes)) = state.storage.get(DB_NAME, &key) {
                        return decode_storage_value(&bytes);
                    }
                    None
                };

                if let Some(serde_json::Value::String(name)) = get_val(uid, "name") {
                    if name.to_lowercase() == candidate.to_lowercase() {
                        let entity_type = match get_val(uid, "entityType") {
                            Some(serde_json::Value::String(t)) => t,
                            _ => "UNKNOWN".to_string(),
                        };

                        if !entities.iter().any(|e: &DetectedEntity| e.name == name) {
                            entities.push(DetectedEntity {
                                name,
                                entity_type,
                                from_context: true,
                                db_entity_uid: Some(uid),
                            });
                        }
                    }
                }
            }
        }
    }

    debug!(
        "Extracted {} entities from conversation history",
        entities.len()
    );
    entities
}

// ── Key Concept Extraction ─────────────────────────────────────────────────

/// Extract key concepts from the query that should be expanded with synonyms.
/// Filters out stop words, entity names already detected, and common words.
fn extract_key_concepts(query: &str, entities: &[DetectedEntity]) -> Vec<String> {
    let stop_words: std::collections::HashSet<&str> = [
        "what",
        "why",
        "who",
        "where",
        "when",
        "how",
        "did",
        "does",
        "was",
        "were",
        "are",
        "is",
        "the",
        "this",
        "that",
        "his",
        "her",
        "their",
        "its",
        "they",
        "them",
        "which",
        "with",
        "from",
        "into",
        "upon",
        "after",
        "before",
        "during",
        "about",
        "has",
        "have",
        "had",
        "can",
        "could",
        "would",
        "should",
        "may",
        "in",
        "on",
        "for",
        "and",
        "but",
        "not",
        "given",
        "first",
        "thing",
        "reason",
        "specific",
        "immediately",
        "once",
        "a",
        "an",
        "to",
        "of",
        "it",
        "do",
        "be",
    ]
    .iter()
    .cloned()
    .collect();

    let entity_names: std::collections::HashSet<String> =
        entities.iter().map(|e| e.name.to_lowercase()).collect();

    query
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.to_lowercase())
        .filter(|w| {
            w.len() > 2
                && !stop_words.contains(w.as_str())
                && !entity_names.contains(w)
                && !w.parse::<i32>().is_ok()
        })
        .collect::<Vec<_>>()
        .into_iter()
        .collect::<std::collections::LinkedList<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        // Dedup while preserving order
        .into_iter()
        .fold(Vec::new(), |mut acc, item| {
            if !acc.contains(&item) {
                acc.push(item);
            }
            acc
        })
}

fn extract_explicit_strongs(query: &str) -> Vec<String> {
    let Ok(re) = Regex::new(r"\b([HG]\d{1,5})\b") else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for cap in re.captures_iter(query) {
        if let Some(m) = cap.get(1) {
            let strong = m.as_str().to_uppercase();
            if !out.contains(&strong) {
                out.push(strong);
            }
        }
    }
    out
}

async fn expand_lexical_candidates(
    state: &Arc<ServerState>,
    key_concepts: &[String],
    explicit_strongs: &[String],
) -> (Vec<String>, Vec<String>) {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut strongs = explicit_strongs.to_vec();
    let mut lemmas = Vec::new();

    for concept in key_concepts {
        for (uid, _) in resolver.search_text_bm25(concept, "englishGloss", "fulltext", 12, false) {
            if let Some(lemma_uid) = get_linked_uid(state, uid, "lemma") {
                if let Some(strongs_id) = get_string(state, lemma_uid, "strongsId") {
                    if !strongs.contains(&strongs_id) {
                        strongs.push(strongs_id);
                    }
                }
                if let Some(lemma_text) = get_string(state, lemma_uid, "lemmaText") {
                    if !lemma_text.is_empty() && !lemmas.contains(&lemma_text) {
                        lemmas.push(lemma_text);
                    }
                }
            }
        }

        for (uid, _) in resolver.search_text_bm25(concept, "gloss", "fulltext", 8, false) {
            if let Some(strongs_id) = get_string(state, uid, "strongsId") {
                if !strongs.contains(&strongs_id) {
                    strongs.push(strongs_id);
                }
            }
            if let Some(lemma_text) = get_string(state, uid, "lemmaText") {
                if !lemma_text.is_empty() && !lemmas.contains(&lemma_text) {
                    lemmas.push(lemma_text);
                }
            }
        }

        // Third pass: search the `usage` field on lemma nodes directly.
        // This bridges English translation variants ("grieved", "sorry", "repent") to the
        // underlying Hebrew lemma via the lexicon usage string, enabling lemma/synonym arms
        // to fire for any word a user might use regardless of which translation is stored.
        for (uid, _) in resolver.search_text_bm25(concept, "usage", "fulltext", 8, false) {
            if let Some(strongs_id) = get_string(state, uid, "strongsId") {
                if !strongs.contains(&strongs_id) {
                    strongs.push(strongs_id);
                }
            }
            if let Some(lemma_text) = get_string(state, uid, "lemmaText") {
                if !lemma_text.is_empty() && !lemmas.contains(&lemma_text) {
                    lemmas.push(lemma_text);
                }
            }
        }
    }

    strongs.truncate(8);
    lemmas.truncate(10);
    (strongs, lemmas)
}

async fn expand_synset_candidates(
    state: &Arc<ServerState>,
    key_concepts: &[String],
    lemma_terms: &[String],
) -> (Vec<String>, Vec<String>) {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut synset_ids = Vec::new();
    let mut synset_glosses = Vec::new();

    for term in key_concepts.iter().chain(lemma_terms.iter()) {
        for (uid, _) in resolver.search_text_bm25(term, "gloss", "fulltext", 8, false) {
            if let Some(id) = get_string(state, uid, "id") {
                if !synset_ids.contains(&id) {
                    synset_ids.push(id);
                }
            }
            if let Some(gloss) = get_string(state, uid, "gloss") {
                if !gloss.is_empty() && !synset_glosses.contains(&gloss) {
                    synset_glosses.push(gloss);
                }
            }
        }
    }

    synset_ids.truncate(8);
    synset_glosses.truncate(8);
    (synset_ids, synset_glosses)
}

fn get_linked_uid(state: &Arc<ServerState>, uid: u64, field: &str) -> Option<u64> {
    let key = Codec::encode_data_key(uid, field);
    if let Ok(Some(bytes)) = state.storage.get(DB_NAME, &key) {
        if let Some(val) = decode_storage_value(&bytes) {
            return match val {
                serde_json::Value::String(s) => s.parse::<u64>().ok(),
                serde_json::Value::Number(n) => n.as_u64(),
                _ => None,
            };
        }
    }
    None
}

fn get_string(state: &Arc<ServerState>, uid: u64, field: &str) -> Option<String> {
    let key = Codec::encode_data_key(uid, field);
    if let Ok(Some(bytes)) = state.storage.get(DB_NAME, &key) {
        if let Some(val) = decode_storage_value(&bytes) {
            return match val {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            };
        }
    }
    None
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
