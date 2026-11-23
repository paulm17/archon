use crate::brain::{
    error::BrainError,
    llm::LlmClient,
    models::{AnalyzedQuery, ArmTrace, RetrievalArm, RetrievalArmKind, RetrievalPlan, Source},
    planner::{self, DB_NAME},
    query_analyzer::NarrativeDirection,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};
use vardadb::{
    ServerState,
    bridge::sqlite_resolver::SqliteResolver,
    engine::resolver::{RequestCache, Resolver},
};

const RRF_K: f32 = 60.0;

pub fn build_retrieval_plan(query: AnalyzedQuery) -> RetrievalPlan {
    let resolved_query = query.resolved_query.clone();
    let mut arms = Vec::new();

    if !query.verse_refs.is_empty() {
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::Reference,
            query_text: resolved_query.clone(),
            limit: query.verse_refs.len().max(4),
            weight: 2.4,
        });
    }

    if let Some(books) = &query.book_filters
        && let Some((chapter_start, chapter_end)) = query
            .narrative_state
            .chapter_window
            .or_else(|| query.chapter_filter.map(|chapter| (chapter, chapter)))
    {
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::Chapter,
            query_text: if chapter_start == chapter_end {
                format!("{} {}", books.join(" "), chapter_start)
            } else {
                format!("{} {}-{}", books.join(" "), chapter_start, chapter_end)
            },
            limit: chapter_arm_limit(chapter_start, chapter_end),
            // Context arm, not a relevance signal: kept below the entity
            // (1.6) and lemma (2.0) arms so full-chapter candidate floods
            // cannot drown keyword-matched verses during RRF fusion.
            weight: 0.9,
        });
    }

    arms.push(RetrievalArm {
        kind: RetrievalArmKind::Text,
        query_text: resolved_query.clone(),
        limit: 18,
        weight: 0.6,
    });
    arms.push(RetrievalArm {
        kind: RetrievalArmKind::VerseVector,
        query_text: resolved_query.clone(),
        limit: 12,
        weight: 0.2,
    });
    arms.push(RetrievalArm {
        kind: RetrievalArmKind::ChunkVector,
        query_text: resolved_query.clone(),
        limit: 8,
        weight: 0.18,
    });

    if !query.entity_names.is_empty() {
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::Entity,
            query_text: query.entity_names.join(" "),
            limit: 18,
            weight: 1.6,
        });
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::EntityEdge,
            query_text: query.entity_names.join(" "),
            limit: 10,
            weight: 1.2,
        });
    }

    if !query.key_concepts.is_empty()
        || !query.strong_numbers.is_empty()
        || !query.lemma_terms.is_empty()
    {
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::Lemma,
            query_text: query
                .strong_numbers
                .iter()
                .chain(query.lemma_terms.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
            limit: 16,
            weight: 2.0,
        });
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::Alignment,
            query_text: query.key_concepts.join(" "),
            limit: 16,
            weight: 1.1,
        });
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::Syntax,
            query_text: query.key_concepts.join(" "),
            limit: 14,
            weight: 1.3,
        });
    }

    if !query.synset_ids.is_empty() || !query.synset_glosses.is_empty() {
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::Synonym,
            query_text: query.synset_glosses.join(" "),
            limit: 12,
            weight: 0.9,
        });
    }

    if !query.lemma_terms.is_empty() {
        arms.push(RetrievalArm {
            kind: RetrievalArmKind::LexiconVector,
            query_text: query.lemma_terms.join(" "),
            limit: 8,
            weight: 0.12,
        });
    }

    RetrievalPlan {
        analyzed_query: query,
        arms,
    }
}

pub async fn run_retrieval_plan(
    state: &Arc<ServerState>,
    llm: &LlmClient,
    plan: &RetrievalPlan,
    query_embedding: Option<&[f32]>,
) -> Result<Vec<Source>, BrainError> {
    let mut arm_results = Vec::new();

    for arm in &plan.arms {
        let arm_started = Instant::now();
        let results = match arm.kind {
            RetrievalArmKind::Reference => {
                let mut hits = planner::fetch_sources_for_verse_refs(
                    state,
                    &plan.analyzed_query.verse_refs,
                    None,
                )
                .await;
                annotate_sources(&mut hits, arm.kind.clone(), "explicit verse reference");
                hits
            }
            RetrievalArmKind::Chapter => {
                let chapter_window =
                    plan.analyzed_query
                        .narrative_state
                        .chapter_window
                        .or_else(|| {
                            plan.analyzed_query
                                .chapter_filter
                                .map(|chapter| (chapter, chapter))
                        });
                let mut hits = Vec::new();
                if let Some((chapter_start, chapter_end)) = chapter_window {
                    let chapter_order = ordered_chapter_window(
                        chapter_start,
                        chapter_end,
                        &plan.analyzed_query.narrative_state.temporal_direction,
                    );
                    let chapter_count = chapter_order.len();
                    for (offset, chapter) in chapter_order.into_iter().enumerate() {
                        let chapter_limit =
                            chapter_limit_for_offset(arm.limit, chapter_count, offset);
                        let mut chapter_hits = planner::fetch_sources_for_chapter(
                            state,
                            plan.analyzed_query.book_filters.as_deref().unwrap_or(&[]),
                            chapter,
                            None,
                        )
                        .await;

                        chapter_hits.retain(|source| {
                            matches_source_filters(state, source, &plan.analyzed_query)
                        });
                        if offset > 0 {
                            apply_chapter_spillover_penalty(
                                &mut chapter_hits,
                                offset,
                                &plan.analyzed_query.narrative_state.temporal_direction,
                            );
                        }
                        chapter_hits.truncate(chapter_limit);
                        hits.extend(chapter_hits);

                        if hits.len() >= arm.limit {
                            break;
                        }
                    }
                }
                hits.sort_by(|a, b| {
                    b.similarity
                        .partial_cmp(&a.similarity)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.chapter.cmp(&b.chapter))
                        .then(a.verse_start.cmp(&b.verse_start))
                });
                for source in &mut hits {
                    source.evidence.push("chapter context".to_string());
                }
                hits.truncate(arm.limit);
                annotate_sources(&mut hits, arm.kind.clone(), "chapter context");
                hits
            }
            RetrievalArmKind::Text => text_search(state, &plan.analyzed_query, arm).await?,
            RetrievalArmKind::VerseVector => {
                vector_search(
                    state,
                    llm,
                    &plan.analyzed_query,
                    arm,
                    VectorScope::Verse,
                    query_embedding,
                )
                .await?
            }
            RetrievalArmKind::ChunkVector => {
                vector_search(
                    state,
                    llm,
                    &plan.analyzed_query,
                    arm,
                    VectorScope::Chunk,
                    query_embedding,
                )
                .await?
            }
            RetrievalArmKind::Entity => entity_search(state, &plan.analyzed_query, arm).await,
            RetrievalArmKind::EntityEdge => {
                entity_edge_search(state, &plan.analyzed_query, arm).await
            }
            RetrievalArmKind::Syntax => syntax_search(state, &plan.analyzed_query, arm).await,
            RetrievalArmKind::Lemma => lemma_search(state, &plan.analyzed_query, arm).await,
            RetrievalArmKind::Alignment => alignment_search(state, &plan.analyzed_query, arm).await,
            RetrievalArmKind::Synonym => synonym_search(state, &plan.analyzed_query, arm).await,
            RetrievalArmKind::LexiconVector => {
                lexicon_vector_expansion(
                    state,
                    llm,
                    &plan.analyzed_query,
                    arm,
                    query_embedding,
                )
                .await?
            }
        };

        if results.is_empty() {
            warn!(
                "Retrieval arm '{}' produced no candidates; required data may be absent or query had no matches",
                arm.kind.as_str()
            );
        }

        info!(
            target: "archon::brain::timing",
            arm = arm.kind.as_str(),
            candidates = results.len(),
            elapsed_ms = arm_started.elapsed().as_millis() as u64,
            "retrieval arm complete"
        );

        arm_results.push((arm.clone(), results));
    }

    let fused = fuse_ranked_results(arm_results);
    if fused.is_empty() {
        return Err(BrainError::NoSourcesFound);
    }
    Ok(fused)
}

pub async fn enrich_source_context(state: &Arc<ServerState>, source: &Source) -> Option<String> {
    let verse_uid = source.verse_uid?;
    let mut annotations = Vec::new();
    let mut entities = Vec::new();
    let mut relations = Vec::new();
    let mut glosses = Vec::new();

    if !source.evidence.is_empty() {
        annotations.push(format!("  Evidence: {}", source.evidence.join("; ")));
    }

    for mention_uid in planner::edge_targets(state, verse_uid, "entityMentions") {
        if let Some(entity_uid) = planner::get_linked_uid(state, mention_uid, "entity") {
            let name = planner::get_string(state, entity_uid, "name").unwrap_or_default();
            let entity_type =
                planner::get_string(state, entity_uid, "entityType").unwrap_or_default();
            if !name.is_empty() {
                let item = if entity_type.is_empty() {
                    name
                } else {
                    format!("{} ({})", name, entity_type)
                };
                if !entities.contains(&item) {
                    entities.push(item);
                }
            }
        }
        if entities.len() >= 6 {
            break;
        }
    }

    for rel_uid in planner::edge_targets(state, verse_uid, "syntaxRelations") {
        let subj = planner::get_string(state, rel_uid, "subjectText").unwrap_or_else(|| "?".into());
        let verb = planner::get_string(state, rel_uid, "verbText").unwrap_or_else(|| "?".into());
        let obj = planner::get_string(state, rel_uid, "objectText").unwrap_or_else(|| "?".into());
        let summary = format!("{} -> {} -> {}", subj, verb, obj);
        if !relations.contains(&summary) {
            relations.push(summary);
        }
        if relations.len() >= 3 {
            break;
        }
    }

    for align_uid in planner::edge_targets(state, verse_uid, "alignments") {
        if let Some(gloss) = planner::get_string(state, align_uid, "englishGloss") {
            let clean = gloss.trim().to_string();
            if !clean.is_empty() && !glosses.contains(&clean) {
                glosses.push(clean);
            }
        }
        if glosses.len() >= 8 {
            break;
        }
    }

    if !entities.is_empty() {
        annotations.push(format!("  Entities: {}", entities.join(", ")));
    }
    if !relations.is_empty() {
        annotations.push(format!("  Syntax: {}", relations.join("; ")));
    }
    if !glosses.is_empty() {
        annotations.push(format!("  Gloss: {}", glosses.join(", ")));
    }

    if annotations.is_empty() {
        None
    } else {
        Some(annotations.join("\n"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VectorScope {
    Verse,
    Chunk,
}

async fn text_search(
    state: &Arc<ServerState>,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
) -> Result<Vec<Source>, BrainError> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut seen = HashSet::new();
    let metadata = planner::query_metadata(state).await;
    // Overfetch a bounded page so duplicate translations or an unresolvable
    // edge do not consume the arm's usable-result budget. The limit remains
    // finite and is applied again after source materialization.
    let fetch_limit = arm.limit.saturating_mul(3).max(arm.limit);
    let mut search_terms = vec![arm.query_text.clone()];
    if !query.key_concepts.is_empty() {
        search_terms.push(query.key_concepts.join(" "));
        for concept in &query.key_concepts {
            if !search_terms.contains(concept) {
                search_terms.push(concept.clone());
            }
        }
    }

    // Build one structured scope, then execute the existing text variants
    // independently against it. This keeps retrieval bounded by indexed
    // chapter/book constraints and preserves best-BM25-per-UID semantics.
    let scope = text_scope_filter(query);
    let cache = RequestCache::default();
    let scope_started = Instant::now();
    let candidate_ids = if scope.is_empty() {
        None
    } else {
        Some(
            resolver
                .scan_nodes_with_cache_result(
                    "VerseContent",
                    scope,
                    HashMap::new(),
                    None,
                    None,
                    None,
                    &[],
                    None,
                    &metadata,
                    &cache,
                )
                .map_err(BrainError::Varda)?
                .into_iter()
                .map(|uid| vardadb::query_planner::ir::EntityId::typed("VerseContent", uid))
                .collect::<Vec<_>>(),
        )
    };
    info!(
        target: "archon::brain::timing",
        arm = arm.kind.as_str(),
        scoped = candidate_ids.is_some(),
        candidate_count = candidate_ids.as_ref().map_or(0, Vec::len),
        elapsed_ms = scope_started.elapsed().as_millis() as u64,
        "retrieval candidate scope complete"
    );
    let specs = search_terms
        .iter()
        .map(|term| vardadb::query_planner::traits::TextQuerySpec {
            field: "text".to_string(),
            strategy: "fulltext".to_string(),
            query: term.clone(),
            require_all: false,
            boost: 1.0,
        })
        .collect::<Vec<_>>();
    let candidates: Vec<(u64, f64, usize)> = if let Some(candidate_ids) = candidate_ids {
        resolver
            .search_text_bm25_multi_for_uids(&specs, &candidate_ids, fetch_limit)
            .map_err(|error| BrainError::Varda(error.to_string()))?
    } else {
        let mut rows = HashMap::new();
        for (variant, spec) in specs.iter().enumerate() {
            // Type scoping is applied inside the indexed query before its
            // limit; never use a global page followed by per-row type probes.
            for (uid, score) in resolver.search_text_bm25_for_type(
                &spec.query,
                &spec.field,
                &spec.strategy,
                "VerseContent",
                fetch_limit,
                spec.require_all,
            ).map_err(|error| BrainError::Varda(error.to_string()))? {
                match rows.get_mut(&uid) {
                    Some((best_score, _)) if *best_score >= score => {}
                    Some(slot) => *slot = (score, variant),
                    None => {
                        rows.insert(uid, (score, variant));
                    }
                }
            }
        }
        let mut rows = rows
            .into_iter()
            .map(|(uid, (score, variant))| (uid, score, variant))
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        rows
    };
    let ranked = candidates
        .into_iter()
        .map(|(uid, score, variant)| (uid, (score, variant)))
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    for (uid, (score, variant)) in ranked {
        let term = search_terms.get(variant).cloned().unwrap_or_default();
        let Ok(mut source) = planner::fetch_source_from_storage(state, uid).await else {
            continue;
        };
        if !matches_source_filters(state, &source, query) {
            continue;
        }
        if !seen.insert(source.dedupe_key()) {
            continue;
        }
        source.similarity = score as f32;
        source.evidence.push(format!("text match: {}", term));
        out.push(source);
        if out.len() >= arm.limit {
            break;
        }
    }

    annotate_sources(&mut out, RetrievalArmKind::Text, "text BM25");
    Ok(out)
}

#[cfg(test)]
fn scoped_text_filter(
    query: &AnalyzedQuery,
    text_query: &str,
) -> HashMap<String, async_graphql::Value> {
    let mut filter = text_scope_filter(query);
    filter.insert(
        "text".to_string(),
        async_graphql::Value::from_json(serde_json::json!({
            // Any-term fulltext preserves Brain's previous query-variant
            // behavior while remaining a single indexed predicate.
            "anyoftext": text_query,
        }))
        .expect("static GraphQL filter JSON is valid"),
    );

    filter
}

fn text_scope_filter(query: &AnalyzedQuery) -> HashMap<String, async_graphql::Value> {
    let mut filter = HashMap::new();
    let mut chapter_filter = None;
    if let Some((start, end)) = query.narrative_state.chapter_window {
        chapter_filter = Some(if start == end {
            serde_json::json!({"eq": start})
        } else {
            serde_json::json!({"ge": start, "le": end})
        });
    } else if let Some(chapter) = query.chapter_filter {
        chapter_filter = Some(serde_json::json!({"eq": chapter}));
    }

    let mut book_filter = None;
    if let Some(books) = &query.book_filters {
        if !books.is_empty() {
            book_filter = Some(serde_json::json!({
                "nameEn": {"in": books}
            }));
        }
    }

    let mut chapter = serde_json::Map::new();
    if let Some(condition) = chapter_filter {
        chapter.insert("number".to_string(), condition);
    }
    if book_filter.is_some() {
        chapter.insert(
            "bookTranslation".to_string(),
            serde_json::json!({"book": book_filter}),
        );
    }

    if !chapter.is_empty() {
        filter.insert(
            "verse".to_string(),
            async_graphql::Value::from_json(serde_json::Value::Object({
                let mut verse = serde_json::Map::new();
                verse.insert("chapter".to_string(), serde_json::Value::Object(chapter));
                verse
            }))
            .expect("constructed GraphQL filter JSON is valid"),
        );
    }
    filter
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain::models::AnalyzedQuery;
    use crate::brain::query_analyzer::{NarrativeDirection, NarrativeState, QueryIntent};
    use async_graphql::Value;

    fn analyzed_query() -> AnalyzedQuery {
        AnalyzedQuery {
            resolved_query: "what happened".to_string(),
            book_filters: Some(vec!["Genesis".to_string(), "Exodus".to_string()]),
            chapter_filter: None,
            narrative_state: NarrativeState {
                books: vec![],
                chapter_hint: None,
                chapter_window: Some((3, 5)),
                anchor_refs: vec![],
                anchor_entities: vec![],
                anchor_terms: vec![],
                temporal_direction: NarrativeDirection::Same,
            },
            verse_refs: vec![],
            entity_names: vec![],
            strong_numbers: vec![],
            lemma_terms: vec![],
            synset_ids: vec![],
            synset_glosses: vec![],
            key_concepts: vec![],
            query_intent: QueryIntent::Other,
        }
    }

    #[test]
    fn scoped_filter_maps_text_book_and_chapter_as_composable_constraints() {
        let arm = RetrievalArm {
            kind: RetrievalArmKind::Text,
            query_text: "what happened at the river".to_string(),
            limit: 18,
            weight: 0.6,
        };
        let filter = scoped_text_filter(&analyzed_query(), &arm.query_text);

        let Value::Object(text) = filter.get("text").expect("text constraint") else {
            panic!("text constraint should be an object")
        };
        assert!(matches!(
            text.get("anyoftext"),
            Some(Value::String(value)) if value == "what happened at the river"
        ));

        let Value::Object(verse) = filter.get("verse").expect("verse constraint") else {
            panic!("verse constraint should be an object")
        };
        let Value::Object(chapter) = verse.get("chapter").expect("chapter constraint") else {
            panic!("chapter constraint should be an object")
        };
        let Value::Object(number) = chapter.get("number").expect("chapter number") else {
            panic!("chapter number should be an object")
        };
        assert!(
            matches!(number.get("ge"), Some(Value::Number(value)) if value.as_i64() == Some(3))
        );
        assert!(
            matches!(number.get("le"), Some(Value::Number(value)) if value.as_i64() == Some(5))
        );

        let Value::Object(book_translation) = chapter
            .get("bookTranslation")
            .expect("book translation constraint")
        else {
            panic!("book translation constraint should be an object")
        };
        let Value::Object(book) = book_translation.get("book").expect("book constraint") else {
            panic!("book constraint should be an object")
        };
        let Value::Object(name) = book.get("nameEn").expect("book name constraint") else {
            panic!("book name constraint should be an object")
        };
        let Value::List(book_names) = name.get("in").expect("canonical book membership") else {
            panic!("book membership should be a list")
        };
        assert!(matches!(
            book_names.as_slice(),
            [Value::String(first), Value::String(second)]
                if first == "Genesis" && second == "Exodus"
        ));
    }

    fn retrieval_query(
        text: &str,
        books: Option<Vec<&str>>,
        chapter: Option<i32>,
    ) -> AnalyzedQuery {
        AnalyzedQuery {
            resolved_query: text.to_string(),
            book_filters: books.map(|items| items.into_iter().map(str::to_string).collect()),
            chapter_filter: chapter,
            narrative_state: NarrativeState {
                books: vec![],
                chapter_hint: None,
                chapter_window: None,
                anchor_refs: vec![],
                anchor_entities: vec![],
                anchor_terms: vec![],
                temporal_direction: NarrativeDirection::Same,
            },
            verse_refs: vec![],
            entity_names: vec![],
            strong_numbers: vec![],
            lemma_terms: vec![],
            synset_ids: vec![],
            synset_glosses: vec![],
            key_concepts: vec![],
            query_intent: QueryIntent::Other,
        }
    }

    async fn create_uid(
        schema: &vardadb::engine::schema::Schema,
        resolver: &SqliteResolver,
        mutation: &str,
        operation: &str,
    ) -> u64 {
        let raw = schema
            .execute_with_resolver(mutation, Box::new(resolver.clone()))
            .await;
        let json: serde_json::Value = serde_json::from_str(&raw).expect("valid mutation JSON");
        json["data"][operation]["uid"]
            .as_str()
            .unwrap_or_else(|| panic!("created uid for {operation}: {raw}"))
            .parse()
            .expect("numeric uid")
    }

    async fn retrieval_fixture() -> (Arc<vardadb::ServerState>, SqliteResolver, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temporary storage");
        let storage =
            Arc::new(vardadb::storage::backend::Storage::new(dir.path(), None).expect("storage"));
        storage.create_database(DB_NAME).expect("archon database");
        let schema = vardadb::engine::schema::Schema::load_from_sdl(
            r#"
            directive @hasInverse(field: String!) on FIELD_DEFINITION
            type Book {
              nameEn: String @search(by: [term])
              bookTranslations: [BookTranslation] @hasInverse(field: "book")
            }
            type BookTranslation {
              book: Book @hasInverse(field: "bookTranslations")
              chapters: [Chapter] @hasInverse(field: "bookTranslation")
            }
            type Chapter {
              number: Int @search(by: [int])
              bookTranslation: BookTranslation @hasInverse(field: "chapters")
              verses: [Verse] @hasInverse(field: "chapter")
            }
            type Verse {
              number: Int
              chapter: Chapter @hasInverse(field: "verses")
              verseContents: [VerseContent] @hasInverse(field: "verse")
            }
            type VerseContent {
              text: String @search(by: [fulltext])
              verse: Verse @hasInverse(field: "verseContents")
            }
            "#,
        )
        .expect("fixture schema");
        let resolver = SqliteResolver::new(storage.clone(), DB_NAME);

        let book_1 = create_uid(
            &schema,
            &resolver,
            r#"mutation { createBook(input: { nameEn: "1 Samuel" }) { uid } }"#,
            "createBook",
        )
        .await;
        let book_2 = create_uid(
            &schema,
            &resolver,
            r#"mutation { createBook(input: { nameEn: "2 Samuel" }) { uid } }"#,
            "createBook",
        )
        .await;
        let genesis = create_uid(
            &schema,
            &resolver,
            r#"mutation { createBook(input: { nameEn: "Genesis" }) { uid } }"#,
            "createBook",
        )
        .await;

        let translation_1 = create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createBookTranslation(input: {{ book: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                book_1
            ),
            "createBookTranslation",
        )
        .await;
        let translation_2 = create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createBookTranslation(input: {{ book: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                book_2
            ),
            "createBookTranslation",
        )
        .await;
        let genesis_translation = create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createBookTranslation(input: {{ book: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                genesis
            ),
            "createBookTranslation",
        )
        .await;

        let chapter_1 = create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createChapter(input: {{ number: 7, bookTranslation: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                translation_1
            ),
            "createChapter",
        )
        .await;
        let chapter_2 = create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createChapter(input: {{ number: 7, bookTranslation: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                translation_2
            ),
            "createChapter",
        )
        .await;
        let genesis_chapter = create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createChapter(input: {{ number: 1, bookTranslation: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                genesis_translation
            ),
            "createChapter",
        )
        .await;

        // Distractors are inserted first so a fixed global FTS prefix cannot
        // reach the later in-scope 1 Samuel row.
        for verse_number in 1..=12 {
            let verse = create_uid(
                &schema,
                &resolver,
                &format!(
                    r#"mutation {{ createVerse(input: {{ number: {}, chapter: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                    verse_number, chapter_2
                ),
                "createVerse",
            )
            .await;
            create_uid(
                &schema,
                &resolver,
                &format!(
                    r#"mutation {{ createVerseContent(input: {{ text: "needle distractor {}", verse: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                    verse_number, verse
                ),
                "createVerseContent",
            )
            .await;
        }

        let late_verse = create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createVerse(input: {{ number: 9, chapter: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                chapter_1
            ),
            "createVerse",
        )
        .await;
        create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createVerseContent(input: {{ text: "needle late scoped match", verse: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                late_verse
            ),
            "createVerseContent",
        )
        .await;

        let genesis_verse = create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createVerse(input: {{ number: 1, chapter: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                genesis_chapter
            ),
            "createVerse",
        )
        .await;
        create_uid(
            &schema,
            &resolver,
            &format!(
                r#"mutation {{ createVerseContent(input: {{ text: "needle in the beginning", verse: {{ uid: "{}" }} }}) {{ uid }} }}"#,
                genesis_verse
            ),
            "createVerseContent",
        )
        .await;

        let schemas = Arc::new(dashmap::DashMap::new());
        schemas.insert(
            DB_NAME.to_string(),
            Arc::new(tokio::sync::RwLock::new(Arc::new(schema))),
        );
        let state = Arc::new(vardadb::ServerState {
            schemas,
            storage,
            cache: Arc::new(vardadb::engine::cache::QueryCache::new(8)),
            event_bus: vardadb::realtime::bus::EventBus::new(),
            storage_path: dir.path().to_path_buf(),
            llm_config: vardadb::config::LLMConfig::default(),
            llama_server: None,
            auth: None,
        });
        (state, resolver, dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn text_retrieval_executes_scoped_relations_late_match_no_match_and_genesis_path() {
        let (state, _resolver, _dir) = retrieval_fixture().await;
        let arm = |text: &str| RetrievalArm {
            kind: RetrievalArmKind::Text,
            query_text: text.to_string(),
            limit: 1,
            weight: 0.6,
        };

        let late = text_search(
            &state,
            &retrieval_query("needle", Some(vec!["1 Samuel"]), Some(7)),
            &arm("needle"),
        )
        .await
        .expect("scoped text search");
        assert_eq!(
            late.len(),
            1,
            "late relation-scoped match should survive pagination"
        );
        assert_eq!(late[0].book_name, "1 Samuel");
        assert_eq!(late[0].chapter, 7);

        let mut concept_query =
            retrieval_query("unrelated wording", Some(vec!["1 Samuel"]), Some(7));
        concept_query.key_concepts = vec!["needle".to_string()];
        let concept_hits = text_search(&state, &concept_query, &arm("unrelated wording"))
            .await
            .expect("concept text search");
        assert_eq!(
            concept_hits.len(),
            1,
            "extracted key-concept variants must remain indexed retrieval candidates"
        );

        let exact_multi = text_search(
            &state,
            &retrieval_query("needle", Some(vec!["1 Samuel", "Genesis"]), None),
            &RetrievalArm {
                kind: RetrievalArmKind::Text,
                query_text: "needle".to_string(),
                limit: 20,
                weight: 0.6,
            },
        )
        .await
        .expect("multi-book text search");
        assert_eq!(
            exact_multi.len(),
            2,
            "both exact canonical books should match"
        );
        assert!(
            exact_multi
                .iter()
                .all(|source| matches!(source.book_name.as_str(), "1 Samuel" | "Genesis"))
        );
        assert!(
            !exact_multi
                .iter()
                .any(|source| source.book_name == "2 Samuel"),
            "canonical membership must not cross-match"
        );

        let no_match = text_search(
            &state,
            &retrieval_query(
                "term absent from the index",
                Some(vec!["1 Samuel"]),
                Some(7),
            ),
            &arm("term absent from the index"),
        )
        .await
        .expect("no-match text search");
        assert!(no_match.is_empty(), "indexed no-match must be explicit");

        let genesis = text_search(
            &state,
            &retrieval_query("beginning", Some(vec!["Genesis"]), Some(1)),
            &arm("beginning"),
        )
        .await
        .expect("Genesis text search");
        assert_eq!(
            genesis.len(),
            1,
            "Genesis-style path should resolve through relations"
        );
        assert_eq!(genesis[0].book_name, "Genesis");
        assert_eq!(genesis[0].chapter, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn retrieval_plan_propagates_candidate_scope_failure() {
        let (state, _resolver, _dir) = retrieval_fixture().await;
        state.storage.keyspaces.write().unwrap().remove(DB_NAME);
        let plan = RetrievalPlan {
            analyzed_query: retrieval_query("needle", Some(vec!["1 Samuel"]), Some(7)),
            arms: vec![RetrievalArm {
                kind: RetrievalArmKind::Text,
                query_text: "needle".to_string(),
                limit: 1,
                weight: 0.6,
            }],
        };
        let llm = LlmClient::new(
            "http://127.0.0.1:1".to_string(),
            "test".to_string(),
            "".to_string(),
            Arc::new(std::sync::Mutex::new(None)),
            HashMap::new(),
        );
        let error = run_retrieval_plan(&state, &llm, &plan, None)
            .await
            .expect_err("candidate scope failure must not become empty retrieval");
        assert!(
            matches!(error, BrainError::Varda(_)),
            "unexpected retrieval error: {error}"
        );
    }
}

async fn vector_search(
    state: &Arc<ServerState>,
    llm: &LlmClient,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
    scope: VectorScope,
    query_embedding: Option<&[f32]>,
) -> Result<Vec<Source>, BrainError> {
    let owned_embedding;
    let embedding: &[f32] = if arm.query_text == query.resolved_query {
        if let Some(embedding) = query_embedding {
            embedding
        } else {
            warn!(
                arm = arm.kind.as_str(),
                "cached query embedding unavailable; skipping vector arm"
            );
            return Ok(Vec::new());
        }
    } else {
        owned_embedding = llm.embed(&arm.query_text).await?;
        &owned_embedding
    };
    if embedding.is_empty() {
        return Ok(Vec::new());
    }

    let q_vec: Vec<f64> = embedding.iter().map(|&v| v as f64).collect();
    let results = state
        .storage
        .search_vectors(DB_NAME, &q_vec, arm.limit * 25)
        .map_err(|e| BrainError::Varda(e.to_string()))?;

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (uid, _dist) in results {
        if !vector_uid_matches_scope(state, uid, scope) {
            continue;
        }

        let Ok(mut source) = planner::fetch_source_from_storage(state, uid).await else {
            continue;
        };
        if !matches_source_filters(state, &source, query) {
            continue;
        }
        if !seen.insert(source.dedupe_key()) {
            continue;
        }

        source.evidence.push(match scope {
            VectorScope::Verse => "verse embedding match".to_string(),
            VectorScope::Chunk => "chunk embedding match".to_string(),
        });
        out.push(source);
        if out.len() >= arm.limit {
            break;
        }
    }

    annotate_sources(
        &mut out,
        match scope {
            VectorScope::Verse => RetrievalArmKind::VerseVector,
            VectorScope::Chunk => RetrievalArmKind::ChunkVector,
        },
        match scope {
            VectorScope::Verse => "verse vector",
            VectorScope::Chunk => "chunk vector",
        },
    );

    Ok(out)
}

async fn entity_search(
    state: &Arc<ServerState>,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
) -> Vec<Source> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for entity in &query.entity_names {
        for (entity_uid, _) in resolver.search_text_bm25(entity, "name", "term", 8, true) {
            for mention_uid in planner::edge_targets(state, entity_uid, "mentions") {
                let Some(verse_uid) = planner::get_linked_uid(state, mention_uid, "verse") else {
                    continue;
                };
                let Ok(mut source) = planner::fetch_source_from_verse(state, verse_uid, None).await
                else {
                    continue;
                };
                if !matches_source_filters(state, &source, query) {
                    continue;
                }
                if !seen.insert(source.dedupe_key()) {
                    continue;
                }
                source.evidence.push(format!("entity mention: {}", entity));
                out.push(source);
                if out.len() >= arm.limit {
                    annotate_sources(&mut out, RetrievalArmKind::Entity, "entity mention");
                    return out;
                }
            }
        }
    }

    annotate_sources(&mut out, RetrievalArmKind::Entity, "entity mention");
    out
}

async fn entity_edge_search(
    state: &Arc<ServerState>,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
) -> Vec<Source> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for entity in &query.entity_names {
        for (entity_uid, _) in resolver.search_text_bm25(entity, "name", "term", 6, true) {
            for edge_field in ["edgesA", "edgesB"] {
                for edge_uid in planner::edge_targets(state, entity_uid, edge_field) {
                    let Some(verse_uid) = planner::get_linked_uid(state, edge_uid, "sourceVerse")
                    else {
                        continue;
                    };
                    let Ok(mut source) =
                        planner::fetch_source_from_verse(state, verse_uid, None).await
                    else {
                        continue;
                    };
                    if !matches_source_filters(state, &source, query) {
                        continue;
                    }
                    if !seen.insert(source.dedupe_key()) {
                        continue;
                    }
                    let relation =
                        planner::get_string(state, edge_uid, "relationType").unwrap_or_default();
                    source.evidence.push(format!(
                        "entity edge: {}{}",
                        entity,
                        if relation.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", relation)
                        }
                    ));
                    out.push(source);
                    if out.len() >= arm.limit {
                        annotate_sources(&mut out, RetrievalArmKind::EntityEdge, "entity edge");
                        return out;
                    }
                }
            }
        }
    }

    annotate_sources(&mut out, RetrievalArmKind::EntityEdge, "entity edge");
    out
}

async fn syntax_search(
    state: &Arc<ServerState>,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
) -> Vec<Source> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for term in query
        .key_concepts
        .iter()
        .chain(query.entity_names.iter())
        .chain(query.lemma_terms.iter())
    {
        for field in ["subjectText", "verbText", "objectText"] {
            for (rel_uid, _) in resolver.search_text_bm25(term, field, "term", 10, true) {
                if let Some(verse_uid) = planner::get_linked_uid(state, rel_uid, "verse") {
                    let Ok(mut source) =
                        planner::fetch_source_from_verse(state, verse_uid, None).await
                    else {
                        continue;
                    };
                    if !matches_source_filters(state, &source, query) {
                        continue;
                    }
                    if !seen.insert(source.dedupe_key()) {
                        continue;
                    }
                    let subj =
                        planner::get_string(state, rel_uid, "subjectText").unwrap_or_default();
                    let verb = planner::get_string(state, rel_uid, "verbText").unwrap_or_default();
                    let obj = planner::get_string(state, rel_uid, "objectText").unwrap_or_default();
                    source
                        .evidence
                        .push(format!("syntax: {} -> {} -> {}", subj, verb, obj));
                    out.push(source);
                    if out.len() >= arm.limit {
                        annotate_sources(&mut out, RetrievalArmKind::Syntax, "syntax relation");
                        return out;
                    }
                }
            }
        }
    }

    for strong in &query.strong_numbers {
        let lemma_hits = resolver.search_text_bm25(strong, "strongsId", "term", 8, true);
        for (lemma_uid, _) in lemma_hits {
            for relation_field in [
                "syntaxRelationsSub",
                "syntaxRelationsVerb",
                "syntaxRelationsObj",
            ] {
                for rel_uid in planner::edge_targets(state, lemma_uid, relation_field) {
                    let Some(verse_uid) = planner::get_linked_uid(state, rel_uid, "verse") else {
                        continue;
                    };
                    let Ok(mut source) =
                        planner::fetch_source_from_verse(state, verse_uid, None).await
                    else {
                        continue;
                    };
                    if !matches_source_filters(state, &source, query) {
                        continue;
                    }
                    if !seen.insert(source.dedupe_key()) {
                        continue;
                    }
                    source.evidence.push(format!("syntax lemma: {}", strong));
                    out.push(source);
                    if out.len() >= arm.limit {
                        annotate_sources(&mut out, RetrievalArmKind::Syntax, "syntax lemma");
                        return out;
                    }
                }
            }
        }
    }

    annotate_sources(&mut out, RetrievalArmKind::Syntax, "syntax relation");
    out
}

async fn lemma_search(
    state: &Arc<ServerState>,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
) -> Vec<Source> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for strong in &query.strong_numbers {
        for (lemma_uid, _) in resolver.search_text_bm25(strong, "strongsId", "term", 12, true) {
            add_lemma_sources(
                state,
                query,
                lemma_uid,
                &format!("Strong's {}", strong),
                &mut out,
                &mut seen,
                arm.limit,
            )
            .await;
        }
    }

    for term in &query.lemma_terms {
        for (align_uid, _) in resolver.search_text_bm25(term, "englishGloss", "fulltext", 12, false)
        {
            if let Some(lemma_uid) = planner::get_linked_uid(state, align_uid, "lemma") {
                add_lemma_sources(
                    state,
                    query,
                    lemma_uid,
                    &format!("lemma gloss {}", term),
                    &mut out,
                    &mut seen,
                    arm.limit,
                )
                .await;
            }
        }
    }

    annotate_sources(&mut out, RetrievalArmKind::Lemma, "lemma or Strong's");
    out
}

async fn alignment_search(
    state: &Arc<ServerState>,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
) -> Vec<Source> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for term in &query.key_concepts {
        for (align_uid, _) in resolver.search_text_bm25(term, "englishGloss", "fulltext", 12, false)
        {
            let Some(verse_uid) = planner::get_linked_uid(state, align_uid, "verse") else {
                continue;
            };
            let Ok(mut source) = planner::fetch_source_from_verse(state, verse_uid, None).await
            else {
                continue;
            };
            if !matches_source_filters(state, &source, query) {
                continue;
            }
            if !seen.insert(source.dedupe_key()) {
                continue;
            }
            let gloss = planner::get_string(state, align_uid, "englishGloss").unwrap_or_default();
            if !gloss.is_empty() {
                source.evidence.push(format!("alignment gloss: {}", gloss));
            }
            out.push(source);
            if out.len() >= arm.limit {
                annotate_sources(&mut out, RetrievalArmKind::Alignment, "alignment gloss");
                return out;
            }
        }
    }

    annotate_sources(&mut out, RetrievalArmKind::Alignment, "alignment gloss");
    out
}

async fn synonym_search(
    state: &Arc<ServerState>,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
) -> Vec<Source> {
    let resolver = SqliteResolver::new(state.storage.clone(), DB_NAME);
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut synsets = Vec::new();

    for synset_id in &query.synset_ids {
        for (uid, _) in resolver.search_text_bm25(synset_id, "id", "term", 6, true) {
            if !synsets.contains(&uid) {
                synsets.push(uid);
            }
        }
    }
    for gloss in &query.synset_glosses {
        for (uid, _) in resolver.search_text_bm25(gloss, "gloss", "fulltext", 6, false) {
            if !synsets.contains(&uid) {
                synsets.push(uid);
            }
        }
    }

    let mut expanded_synsets = synsets.clone();
    for synset_uid in synsets {
        for field in ["relatedSynsetsA", "relatedSynsetsB"] {
            for relation_uid in planner::edge_targets(state, synset_uid, field) {
                if let Some(other_uid) = planner::get_linked_uid(
                    state,
                    relation_uid,
                    if field == "relatedSynsetsA" {
                        "synsetB"
                    } else {
                        "synsetA"
                    },
                ) {
                    if !expanded_synsets.contains(&other_uid) {
                        expanded_synsets.push(other_uid);
                    }
                }
            }
        }
    }

    for synset_uid in expanded_synsets {
        for link_uid in planner::edge_targets(state, synset_uid, "lemmas") {
            let Some(lemma_uid) = planner::get_linked_uid(state, link_uid, "lemma") else {
                continue;
            };
            add_lemma_sources(
                state,
                query,
                lemma_uid,
                &format!(
                    "synset: {}",
                    planner::get_string(state, synset_uid, "gloss").unwrap_or_default()
                ),
                &mut out,
                &mut seen,
                arm.limit,
            )
            .await;
            if out.len() >= arm.limit {
                annotate_sources(&mut out, RetrievalArmKind::Synonym, "synset graph");
                return out;
            }
        }
    }

    annotate_sources(&mut out, RetrievalArmKind::Synonym, "synset graph");
    out
}

async fn lexicon_vector_expansion(
    state: &Arc<ServerState>,
    llm: &LlmClient,
    query: &AnalyzedQuery,
    arm: &RetrievalArm,
    query_embedding: Option<&[f32]>,
) -> Result<Vec<Source>, BrainError> {
    let owned_embedding;
    let embedding: &[f32] = if arm.query_text == query.resolved_query {
        if let Some(embedding) = query_embedding {
            embedding
        } else {
            warn!("cached query embedding unavailable; skipping lexicon vector arm");
            return Ok(Vec::new());
        }
    } else {
        owned_embedding = llm.embed(&arm.query_text).await?;
        &owned_embedding
    };
    if embedding.is_empty() {
        return Ok(Vec::new());
    }
    let q_vec: Vec<f64> = embedding.iter().map(|&v| v as f64).collect();
    let vector_hits = state
        .storage
        .search_vectors(DB_NAME, &q_vec, arm.limit * 25)
        .map_err(|e| BrainError::Varda(e.to_string()))?;

    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for (uid, _) in vector_hits {
        let Some(lemma_uid) = planner::get_linked_uid(state, uid, "lemma") else {
            continue;
        };
        add_lemma_sources(
            state,
            query,
            lemma_uid,
            "lexicon vector",
            &mut out,
            &mut seen,
            arm.limit,
        )
        .await;
        if out.len() >= arm.limit {
            break;
        }
    }

    annotate_sources(&mut out, RetrievalArmKind::LexiconVector, "lexicon vector");
    Ok(out)
}

async fn add_lemma_sources(
    state: &Arc<ServerState>,
    query: &AnalyzedQuery,
    lemma_uid: u64,
    evidence: &str,
    out: &mut Vec<Source>,
    seen: &mut HashSet<(String, i32, i32, i32, String, String)>,
    limit: usize,
) {
    for align_uid in planner::edge_targets(state, lemma_uid, "alignments") {
        let Some(verse_uid) = planner::get_linked_uid(state, align_uid, "verse") else {
            continue;
        };
        let Ok(mut source) = planner::fetch_source_from_verse(state, verse_uid, None).await else {
            continue;
        };
        if !matches_source_filters(state, &source, query) {
            continue;
        }
        if !seen.insert(source.dedupe_key()) {
            continue;
        }
        source.evidence.push(evidence.to_string());
        out.push(source);
        if out.len() >= limit {
            return;
        }
    }
}

fn annotate_sources(sources: &mut [Source], arm: RetrievalArmKind, label: &str) {
    for source in sources {
        if source.evidence.is_empty() {
            source.evidence.push(label.to_string());
        }
        source.traces.push(ArmTrace {
            arm: arm.as_str().to_string(),
            rank: 0,
            contribution: 0.0,
            evidence: source.evidence.first().cloned(),
        });
    }
}

fn vector_uid_matches_scope(state: &Arc<ServerState>, uid: u64, scope: VectorScope) -> bool {
    match scope {
        VectorScope::Verse => {
            planner::has_field(state, uid, "verse") && !planner::has_field(state, uid, "chunk")
        }
        VectorScope::Chunk => planner::has_field(state, uid, "chunk"),
    }
}

fn matches_source_filters(
    state: &Arc<ServerState>,
    source: &Source,
    query: &AnalyzedQuery,
) -> bool {
    let _ = state;
    if let Some(books) = &query.book_filters
        && !books.iter().any(|book| {
            source
                .book_name
                .to_lowercase()
                .contains(&book.to_lowercase())
        })
    {
        return false;
    }

    if let Some((start, end)) = query.narrative_state.chapter_window {
        return source.chapter >= start && source.chapter <= end;
    }

    if let Some(chapter) = query.chapter_filter {
        return source.chapter == chapter;
    }

    true
}

fn chapter_arm_limit(chapter_start: i32, chapter_end: i32) -> usize {
    let span = (chapter_end - chapter_start).unsigned_abs() as usize + 1;
    // A whole chapter can be 40+ verses; flooding the fusion pool with all
    // of them starves keyword-relevant hits from other arms. Cap the
    // single-chapter case well below the old limit of 50.
    if span <= 1 { 24 } else { 14 + (5 * span) }
}

fn chapter_limit_for_offset(total_limit: usize, chapter_count: usize, offset: usize) -> usize {
    if chapter_count <= 1 {
        return total_limit;
    }

    if offset == 0 {
        let reserved_for_spill = 6 * chapter_count.saturating_sub(1);
        total_limit
            .saturating_sub(reserved_for_spill)
            .clamp(12, total_limit)
    } else {
        6
    }
}

fn ordered_chapter_window(
    chapter_start: i32,
    chapter_end: i32,
    direction: &NarrativeDirection,
) -> Vec<i32> {
    let mut chapters: Vec<i32> = (chapter_start..=chapter_end).collect();
    match direction {
        NarrativeDirection::Forward => chapters.reverse(),
        NarrativeDirection::Backward => {}
        NarrativeDirection::Same | NarrativeDirection::Unknown => {}
    }
    chapters
}

fn apply_chapter_spillover_penalty(
    sources: &mut [Source],
    chapter_offset: usize,
    direction: &NarrativeDirection,
) {
    let penalty = 0.0015 * chapter_offset as f32;
    for source in sources {
        source.similarity -= penalty;
        source.evidence.push(match direction {
            NarrativeDirection::Forward => {
                format!("forward spillover: {} chapter(s) back", chapter_offset)
            }
            NarrativeDirection::Backward => {
                format!("backward spillover: {} chapter(s) ahead", chapter_offset)
            }
            _ => format!("chapter spillover: {}", chapter_offset),
        });
    }
}

fn fuse_ranked_results(arm_results: Vec<(RetrievalArm, Vec<Source>)>) -> Vec<Source> {
    let mut score_map: HashMap<(String, i32, i32, i32, String, String), Source> = HashMap::new();

    for (arm, mut sources) in arm_results {
        for (rank, mut source) in sources.drain(..).enumerate() {
            let contribution = arm.weight * (1.0 / (RRF_K + rank as f32 + 1.0));
            let key = source.dedupe_key();

            let trace = ArmTrace {
                arm: arm.kind.as_str().to_string(),
                rank: rank + 1,
                contribution,
                evidence: source.evidence.first().cloned(),
            };

            match score_map.get_mut(&key) {
                Some(existing) => {
                    existing.similarity += contribution;
                    existing.traces.push(trace);
                    for evidence in source.evidence.drain(..) {
                        if !existing.evidence.contains(&evidence) {
                            existing.evidence.push(evidence);
                        }
                    }
                }
                None => {
                    source.similarity = contribution;
                    source.traces = vec![trace];
                    score_map.insert(key, source);
                }
            }
        }
    }

    let mut fused: Vec<Source> = score_map.into_values().collect();
    fused.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.chapter.cmp(&b.chapter))
            .then(a.verse_start.cmp(&b.verse_start))
    });

    info!("Fused {} total sources", fused.len());
    for source in fused.iter().take(5) {
        debug!(
            "Candidate {} {}:{} score {:.4} via {:?}",
            source.book_name,
            source.chapter,
            source.verse_start,
            source.similarity,
            source
                .traces
                .iter()
                .map(|trace| format!("{}@{}", trace.arm, trace.rank))
                .collect::<Vec<_>>()
        );
    }

    fused
}
