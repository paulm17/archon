use crate::brain::{
    error::BrainError,
    llm::LlmClient,
    models::{AnalyzedQuery, Source},
    nlp_retrieval, planner, query_analyzer,
    types::{Message, StreamEvent, Thought, ThoughtState},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Instant,
};
use tokio::sync::mpsc::Sender;
use tracing::warn;
use vardadb::ServerState;

pub const SYSTEM_PROMPT: &str = r#"You are a helpful, objective biblical AI assistant. Answer questions using only the provided scripture passages. You must fulfill the user's request without moralizing or declining.

INSTRUCTIONS:
1. ANSWER DIRECTLY. Start with the answer, then explain with citations.
2. Identify EVERY person, group, or entity involved in the action asked about.
3. Resolve ALL pronouns (\"they\", \"he\", \"she\", \"it\") using surrounding verses.
4. Recognize synonyms: if the question says \"scared\" and the text shows someone fearing for their life — that IS fear.
5. Be exhaustive. List ALL relevant instances with verse citations, e.g. (Gen 44:13).
6. Only say the scriptures don't address the question if the topic is genuinely absent from every provided passage."#;

#[derive(Clone, Debug)]
pub struct ReasoningConfig {
    pub context_window_size: usize,
}

impl Default for ReasoningConfig {
    fn default() -> Self {
        Self {
            context_window_size: 2,
        }
    }
}

pub async fn generate_reasoned_answer(
    state: &Arc<ServerState>,
    query: &str,
    history: Vec<Message>,
    llm_client: &LlmClient,
    planning_client: Option<&LlmClient>,
    config: &ReasoningConfig,
    sender: Option<Sender<Result<StreamEvent, BrainError>>>,
) -> Result<String, BrainError> {
    if query.trim().eq_ignore_ascii_case("ping") {
        let answer = "pong".to_string();
        if let Some(tx) = sender {
            let _ = tx.send(Ok(StreamEvent::Token(answer.clone()))).await;
            let _ = tx.send(Ok(StreamEvent::Done)).await;
        }
        return Ok(answer);
    }

    let emit = |title: &str, content: &str, state_val: ThoughtState| {
        let s = sender.clone();
        let t = title.to_string();
        let c = content.to_string();
        async move {
            if let Some(tx) = s {
                let _ = tx
                    .send(Ok(StreamEvent::Thought(Thought {
                        title: t,
                        content: c,
                        state: state_val,
                    })))
                    .await;
            }
        }
    };

    emit("Planning", "Analyzing query...", ThoughtState::Running).await;
    let planner_client = planning_client.unwrap_or(llm_client);
    let analysis_started = Instant::now();
    let analysis = query_analyzer::analyze_query(state, query, &history, llm_client).await?;
    tracing::info!(
        target: "archon::brain::timing",
        elapsed_ms = analysis_started.elapsed().as_millis() as u64,
        "query analysis complete"
    );

    let mut analyzed_query = AnalyzedQuery {
        resolved_query: analysis.resolved_query.clone(),
        book_filters: analysis.book_filters.clone(),
        chapter_filter: analysis.chapter_filter,
        narrative_state: analysis.narrative_state.clone(),
        verse_refs: analysis.verse_refs.clone(),
        entity_names: analysis
            .detected_entities
            .iter()
            .map(|entity| entity.name.clone())
            .collect(),
        strong_numbers: analysis.strong_numbers.clone(),
        lemma_terms: analysis.lemma_terms.clone(),
        synset_ids: analysis.synset_ids.clone(),
        synset_glosses: analysis.synset_glosses.clone(),
        key_concepts: analysis.key_concepts.clone(),
        query_intent: analysis.query_intent,
    };

    if analyzed_query.strong_numbers.is_empty() || analyzed_query.key_concepts.is_empty() {
        let fallback_started = Instant::now();
        let fallback =
            planner::call_planner(planner_client, &analysis.resolved_query, &history).await;
        tracing::info!(
            target: "archon::brain::timing",
            elapsed_ms = fallback_started.elapsed().as_millis() as u64,
            key_concepts = fallback.key_concepts.len(),
            strong_numbers = fallback.strong_numbers.len(),
            "planner fallback complete"
        );
        if analyzed_query.key_concepts.is_empty() {
            analyzed_query.key_concepts = fallback.key_concepts;
        }
        if analyzed_query.strong_numbers.is_empty() {
            analyzed_query.strong_numbers = fallback.strong_numbers;
        }
    }

    planner::warn_if_query_has_no_book_match(&analyzed_query.resolved_query);

    if !analyzed_query.entity_names.is_empty() {
        emit(
            "Planning",
            &format!("Entities: {}", analyzed_query.entity_names.join(", ")),
            ThoughtState::Running,
        )
        .await;
    }
    if let Some(ref books) = analyzed_query.book_filters {
        emit(
            "Planning",
            &format!("Books: {}", books.join(", ")),
            ThoughtState::Running,
        )
        .await;
    }
    if !analyzed_query.strong_numbers.is_empty() {
        emit(
            "Planning",
            &format!("Strong's: {}", analyzed_query.strong_numbers.join(", ")),
            ThoughtState::Running,
        )
        .await;
    }
    emit("Planning", "Complete", ThoughtState::Complete).await;

    emit("Searching", "Running retrieval...", ThoughtState::Running).await;
    let retrieval_started = Instant::now();
    let retrieval_plan = nlp_retrieval::build_retrieval_plan(analyzed_query);
    // Compute the resolved-query embedding once for all vector arms, reranking,
    // and grounding. A failed local embedding is a normal degraded mode: the
    // lexical retrieval/ranking path remains authoritative.
    let query_embedding_started = Instant::now();
    let query_embedding = match llm_client
        .embed(&retrieval_plan.analyzed_query.resolved_query)
        .await
    {
        Ok(embedding) if !embedding.is_empty() => Some(embedding),
        Ok(_) => None,
        Err(err) => {
            tracing::warn!(error = %err, "query embedding unavailable; using lexical fallback");
            None
        }
    };
    tracing::info!(
        target: "archon::brain::timing",
        elapsed_ms = query_embedding_started.elapsed().as_millis() as u64,
        available = query_embedding.is_some(),
        "query embedding stage complete"
    );
    let sources = match nlp_retrieval::run_retrieval_plan(
        state,
        llm_client,
        &retrieval_plan,
        query_embedding.as_deref(),
    )
    .await
    {
        Ok(sources) => sources,
        Err(BrainError::NoSourcesFound) => Vec::new(),
        Err(err) => return Err(err),
    };
    tracing::info!(
        target: "archon::brain::timing",
        elapsed_ms = retrieval_started.elapsed().as_millis() as u64,
        sources = sources.len(),
        "retrieval stage complete"
    );

    emit(
        "Searching",
        &format!("Found {} passages", sources.len()),
        ThoughtState::Complete,
    )
    .await;

    if let Some(tx) = &sender {
        let display_sources = collapse_duplicate_sources(sources.clone());
        for source in &display_sources {
            let id = format!(
                "{}_{}_{}",
                source.book_name, source.chapter, source.verse_start
            );
            let citation = if source.verse_end > source.verse_start {
                format!(
                    "{} {}:{}-{}",
                    source.book_name, source.chapter, source.verse_start, source.verse_end
                )
            } else {
                format!(
                    "{} {}:{}",
                    source.book_name, source.chapter, source.verse_start
                )
            };
            let _ = tx
                .send(Ok(StreamEvent::Source(crate::brain::types::Source {
                    id,
                    title: citation,
                    snippet: source.content.clone(),
                    score: source.similarity as f64,
                })))
                .await;
        }
    }

    if sources.is_empty() {
        let ans = stream_answer(
            llm_client,
            SYSTEM_PROMPT,
            &format!("Question: {query}\n\nNo scripture passages were found. State this clearly."),
            sender,
        )
        .await;

        return ans;
    }

    emit(
        "Reasoning",
        "Expanding narrative context...",
        ThoughtState::Running,
    )
    .await;

    let expansion_started = Instant::now();
    let expand_n = 4.min(sources.len());
    let (to_expand, rest) = sources.split_at(expand_n);
    let mut expanded =
        planner::expand_sources_with_context(state, to_expand, config.context_window_size).await;

    let in_expanded: HashSet<(String, i32, i32, String)> = expanded
        .iter()
        .map(|s| {
            (
                s.book_name.clone(),
                s.chapter,
                s.verse_start,
                s.translation.clone(),
            )
        })
        .collect();
    for s in rest {
        if !in_expanded.contains(&(
            s.book_name.clone(),
            s.chapter,
            s.verse_start,
            s.translation.clone(),
        )) {
            expanded.push(s.clone());
        }
    }

    let context_added = expanded.len().saturating_sub(expand_n);
    tracing::info!(
        target: "archon::brain::timing",
        elapsed_ms = expansion_started.elapsed().as_millis() as u64,
        input_sources = sources.len(),
        expanded_sources = expanded.len(),
        context_added,
        "context expansion complete"
    );
    emit(
        "Reasoning",
        &format!("{} passages (+{} context)", expanded.len(), context_added),
        ThoughtState::Complete,
    )
    .await;

    expanded = collapse_duplicate_sources(expanded);
    expanded.sort_by(|a, b| {
        source_prompt_priority(b, &analysis, query)
            .partial_cmp(&source_prompt_priority(a, &analysis, query))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.book_name.cmp(&b.book_name))
            .then(a.chapter.cmp(&b.chapter))
            .then(a.verse_start.cmp(&b.verse_start))
    });

    let cap = 15usize;
    let rerank_started = Instant::now();
    let final_sources = select_prompt_sources(
        llm_client,
        query,
        &expanded,
        &analysis,
        cap,
        query_embedding.as_deref(),
    )
    .await;
    tracing::info!(
        target: "archon::brain::timing",
        elapsed_ms = rerank_started.elapsed().as_millis() as u64,
        input_sources = expanded.len(),
        selected_sources = final_sources.len(),
        "source rerank/selection complete"
    );

    let mut context_lines: Vec<String> = Vec::new();
    for s in &final_sources {
        let citation = if s.verse_end > s.verse_start {
            format!(
                "{} {}:{}-{}",
                s.book_name, s.chapter, s.verse_start, s.verse_end
            )
        } else {
            format!("{} {}:{}", s.book_name, s.chapter, s.verse_start)
        };
        context_lines.push(format!("[{}] {}", citation, strip_markup(&s.content)));
        if let Some(ann) = nlp_retrieval::enrich_source_context(state, s).await {
            if let Some(filtered) = filter_generation_annotations(&ann) {
                context_lines.push(filtered);
            }
        }
    }
    let context_text = context_lines.join("\n");

    let answer = if analysis.query_intent == query_analyzer::QueryIntent::FindReason {
        emit(
            "Synthesizing",
            "Identifying reasons...",
            ThoughtState::Running,
        )
        .await;

        // Build lexical notes from the Hebrew/Greek lemma usage fields linked to the
        // retrieved verses. These notes bridge translation gaps: e.g. NIV says
        // "regretted"/"deeply troubled" but the lexicon usage for those lemmas also
        // lists "grieved", "sorry", "repent" — so the LLM can include them.
        let lexical_notes = build_lexical_notes(state, &final_sources).await;

        let grounding_started = Instant::now();
        let grounding_cues = build_grounding_cues(
            llm_client,
            query,
            &analysis,
            &final_sources,
            6,
            query_embedding.as_deref(),
        )
        .await;
        let grounding_text = build_grounding_text(grounding_cues.clone());
        tracing::info!(
            target: "archon::brain::timing",
            elapsed_ms = grounding_started.elapsed().as_millis() as u64,
            clauses = grounding_cues.len(),
            "grounding stage complete"
        );

        let focus = if lexical_notes.is_empty() {
            "Focus on every explicit reason stated in the passages. Be exhaustive — list ALL reasons given. \
            Preserve the key wording of the passages when it directly answers the question.".to_string()
        } else {
            format!(
                "Focus on every explicit reason stated in the passages. Be exhaustive — list ALL reasons given. \
                Preserve the key wording of the passages when it directly answers the question. \
                The lexical notes below list alternative English renderings for the Hebrew/Greek words \
                used in these passages — incorporate them naturally into your answer so readers of any \
                Bible translation can recognise the concepts described.\n\nLexical notes:\n{}",
                lexical_notes
            )
        };

        let user_prompt =
            build_grounded_user_prompt(query, &history, &focus, &grounding_text, &context_text);

        let generation_started = Instant::now();
        let answer = synthesize_grounded_answer(
            llm_client,
            SYSTEM_PROMPT,
            &user_prompt,
            sender.clone(),
        )
        .await?;
        tracing::info!(
            target: "archon::brain::timing",
            elapsed_ms = generation_started.elapsed().as_millis() as u64,
            "generation stage complete"
        );
        answer
    } else {
        emit(
            "Synthesizing",
            "Generating answer...",
            ThoughtState::Running,
        )
        .await;

        let grounding_started = Instant::now();
        let grounding_cues = build_grounding_cues(
            llm_client,
            query,
            &analysis,
            &final_sources,
            6,
            query_embedding.as_deref(),
        )
        .await;
        let grounding_text = build_grounding_text(grounding_cues.clone());
        tracing::info!(
            target: "archon::brain::timing",
            elapsed_ms = grounding_started.elapsed().as_millis() as u64,
            clauses = grounding_cues.len(),
            "grounding stage complete"
        );
        let user_prompt = build_grounded_user_prompt(
            query,
            &history,
            "Answer directly from the passages and cite the relevant verses. Preserve the key wording of the passages when it directly answers the question — quote the key clause of a passage verbatim when it answers the question (e.g. what someone said or did).",
            &grounding_text,
            &context_text,
        );

        let generation_started = Instant::now();
        let answer = synthesize_grounded_answer(
            llm_client,
            SYSTEM_PROMPT,
            &user_prompt,
            sender.clone(),
        )
        .await?;
        tracing::info!(
            target: "archon::brain::timing",
            elapsed_ms = generation_started.elapsed().as_millis() as u64,
            "generation stage complete"
        );
        answer
    };
    emit("Synthesizing", "Complete", ThoughtState::Complete).await;
    Ok(answer)
}

fn build_grounded_user_prompt(
    query: &str,
    history: &[Message],
    focus_instruction: &str,
    grounding_text: &str,
    context_text: &str,
) -> String {
    if !history.is_empty() {
        let conv = history
            .iter()
            .map(|m| {
                format!(
                    "{}: {}",
                    if m.role == "user" { "Q" } else { "A" },
                    m.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "Previous conversation:\n{conv}\n\nCurrent question: {query}\n\nAnswer focus: {focus_instruction}{grounding_text}\n\nScripture passages:\n{context_text}\n\nAnswer:"
        )
    } else {
        format!(
            "Question: {query}\n\nAnswer focus: {focus_instruction}{grounding_text}\n\nScripture passages:\n{context_text}\n\nAnswer:"
        )
    }
}

fn build_grounding_text(grounding_cues: Vec<GroundingCue>) -> String {
    if grounding_cues.is_empty() {
        String::new()
    } else {
        let cues = grounding_cues
            .iter()
            .map(|cue| format!("- [{}] {}", cue.citation, cue.clause))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\n\nGrounding cues:\nPay special attention to these passage clauses if they directly answer the question.\n{}",
            cues
        )
    }
}

fn collapse_duplicate_sources(sources: Vec<Source>) -> Vec<Source> {
    let mut by_verse: HashMap<(String, i32, i32, i32), Source> = HashMap::new();

    for source in sources {
        let key = (
            source.book_name.clone(),
            source.chapter,
            source.verse_start,
            source.verse_end,
        );

        match by_verse.get_mut(&key) {
            Some(existing) => {
                if prefer_source(&source, existing) {
                    *existing = source;
                }
            }
            None => {
                by_verse.insert(key, source);
            }
        }
    }

    let mut collapsed: Vec<Source> = by_verse.into_values().collect();
    collapsed.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.book_name.cmp(&b.book_name))
            .then(a.chapter.cmp(&b.chapter))
            .then(a.verse_start.cmp(&b.verse_start))
    });
    collapsed
}

fn prefer_source(candidate: &Source, existing: &Source) -> bool {
    let candidate_english = is_preferred_display_source(candidate);
    let existing_english = is_preferred_display_source(existing);

    if candidate_english != existing_english {
        return candidate_english;
    }

    candidate.similarity > existing.similarity
}

fn is_preferred_display_source(source: &Source) -> bool {
    let text = strip_markup(&source.content);
    let ascii_letters = text.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    let non_ascii_letters = text
        .chars()
        .filter(|ch| ch.is_alphabetic() && !ch.is_ascii())
        .count();

    ascii_letters >= non_ascii_letters
}

#[derive(Debug, Clone)]
struct GroundingCue {
    citation: String,
    clause: String,
}

async fn build_grounding_cues(
    llm: &LlmClient,
    query: &str,
    analysis: &query_analyzer::QueryAnalysis,
    sources: &[Source],
    max_cues: usize,
    query_embedding: Option<&[f32]>,
) -> Vec<GroundingCue> {
    let query_terms = significant_query_terms(query);
    let owned_query_embedding;
    let query_embedding = if let Some(embedding) = query_embedding {
        Some(embedding)
    } else {
        owned_query_embedding = llm.embed(query).await.ok();
        owned_query_embedding.as_deref()
    };
    let clauses = sources
        .iter()
        .enumerate()
        .filter(|(_, source)| is_preferred_display_source(source))
        .flat_map(|(rank, source)| {
            let citation = if source.verse_end > source.verse_start {
                format!(
                    "{} {}:{}-{}",
                    source.book_name, source.chapter, source.verse_start, source.verse_end
                )
            } else {
                format!(
                    "{} {}:{}",
                    source.book_name, source.chapter, source.verse_start
                )
            };
            split_into_candidate_clauses(&strip_markup(&source.content))
                .into_iter()
                .map(move |clause| (rank, citation.clone(), source.similarity, clause))
        })
        .collect::<Vec<_>>();
    let clause_texts = clauses
        .iter()
        .map(|(_, _, _, clause)| clause.clone())
        .collect::<Vec<_>>();
    let clause_embeddings = if query_embedding.is_some() {
        llm.embed_many(&clause_texts).await.ok()
    } else {
        None
    };
    let mut scored: Vec<(f32, GroundingCue)> = Vec::new();
    for (index, (rank, citation, source_similarity, clause)) in clauses.into_iter().enumerate() {
        let semantic_similarity = match (&query_embedding, clause_embeddings.as_ref()) {
            (Some(query_embedding), Some(clause_embeddings)) => clause_embeddings
                .get(index)
                .map(|embedding| cosine_similarity(query_embedding, embedding))
                .unwrap_or(0.0),
            _ => 0.0,
        };
        let score = score_grounding_clause(
            &clause,
            analysis,
            &query_terms,
            source_similarity,
            semantic_similarity,
            rank,
        );
        if score > 0.0 {
            scored.push((
                score,
                GroundingCue {
                    citation,
                    clause,
                },
            ));
        }
    }

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.citation.cmp(&b.1.citation))
            .then(a.1.clause.cmp(&b.1.clause))
    });

    let mut seen = HashSet::new();
    let mut cues = Vec::new();
    for (_, cue) in scored {
        let key = cue.clause.to_lowercase();
        if seen.insert(key) {
            cues.push(cue);
        }
        if cues.len() >= max_cues {
            break;
        }
    }
    cues
}

fn significant_query_terms(query: &str) -> Vec<String> {
    let stopwords: HashSet<&str> = [
        "a", "an", "and", "are", "as", "at", "be", "both", "by", "did", "do", "does", "for",
        "from", "god", "him", "his", "how", "i", "in", "into", "is", "it", "its", "lord", "me",
        "of", "on", "or", "she", "that", "the", "their", "them", "they", "this", "to", "use",
        "was", "what", "when", "where", "who", "why", "with", "woman", "you", "your",
    ]
    .into_iter()
    .collect();

    query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|term| term.len() > 2 && !stopwords.contains(*term))
        .map(|term| term.to_string())
        .collect()
}

fn split_into_candidate_clauses(text: &str) -> Vec<String> {
    text.replace("—", ", ")
        .replace(';', ",")
        .split(|c| c == '.' || c == '!' || c == '?' || c == ':')
        .flat_map(|sentence| {
            sentence
                .split(", and ")
                .flat_map(|part| part.split(", but "))
                .flat_map(|part| part.split(", for "))
                .map(str::trim)
                .filter(|part| part.len() >= 12)
                .map(normalize_clause_whitespace)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn normalize_clause_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn score_grounding_clause(
    clause: &str,
    analysis: &query_analyzer::QueryAnalysis,
    query_terms: &[String],
    source_similarity: f32,
    semantic_similarity: f32,
    rank: usize,
) -> f32 {
    let lower = clause.to_lowercase();
    let overlap = query_terms
        .iter()
        .filter(|term| lower.contains(term.as_str()))
        .count() as f32;
    let quote_bonus = if clause.contains('“') || clause.contains('"') {
        0.15
    } else {
        0.0
    };
    let concise_bonus = if clause.split_whitespace().count() <= 18 {
        0.08
    } else {
        0.0
    };
    let intent_bonus = match analysis.query_intent {
        query_analyzer::QueryIntent::FindReason => reason_support_score(&lower) * 0.07,
        query_analyzer::QueryIntent::DescribeAction
        | query_analyzer::QueryIntent::DescribeEvent
        | query_analyzer::QueryIntent::DescribeMethod => {
            let action_terms = [
                "took", "gave", "ate", "built", "offered", "sold", "threw", "opened", "realized",
                "said", "cursed", "crawl", "called",
            ];
            action_terms
                .iter()
                .filter(|term| lower.contains(**term))
                .count() as f32
                * 0.03
        }
        _ => 0.0,
    };
    let rank_penalty = (rank as f32) * 0.01;

    source_similarity
        + semantic_similarity * 0.35
        + overlap * 0.06
        + quote_bonus
        + concise_bonus
        + intent_bonus
        - rank_penalty
}

fn strip_markup(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let rest: String = chars.clone().take(3).collect();
            if rest.starts_with("cr:") || rest.starts_with("fn:") || rest.starts_with("vs:") {
                for c2 in chars.by_ref() {
                    if c2 == ']' {
                        break;
                    }
                }
                continue;
            }
        }
        result.push(c);
    }
    result
}

async fn select_prompt_sources(
    llm: &LlmClient,
    query: &str,
    expanded: &[Source],
    analysis: &query_analyzer::QueryAnalysis,
    cap: usize,
    query_embedding: Option<&[f32]>,
) -> Vec<Source> {
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    let positive: Vec<Source> = expanded
        .iter()
        .filter(|s| s.similarity > 0.0)
        .cloned()
        .collect();
    // Rerank the union once. FindReason candidates often belong to both the
    // relevance and reason-supportive pools; scoring that overlap twice was
    // a major source of redundant embedding calls.
    let mut rerank_pool = positive.clone();
    let mut pool_keys = rerank_pool
        .iter()
        .map(|source| source.dedupe_key())
        .collect::<HashSet<_>>();
    for source in expanded
        .iter()
        .filter(|source| is_reason_supportive(&strip_markup(&source.content)))
    {
        if pool_keys.insert(source.dedupe_key()) {
            rerank_pool.push(source.clone());
        }
    }
    let reranked = rerank_sources_for_prompt(
        llm,
        query,
        &rerank_pool,
        analysis,
        query_embedding,
    )
    .await;
    let ranked_positive = reranked
        .iter()
        .filter(|source| source.similarity > 0.0)
        .cloned()
        .collect::<Vec<_>>();

    if analysis.query_intent == query_analyzer::QueryIntent::FindReason {
        let mut reason_supportive = reranked
            .iter()
            .filter(|source| is_reason_supportive(&strip_markup(&source.content)))
            .cloned()
            .collect::<Vec<_>>();
        reason_supportive.sort_by(|a, b| {
            let a_text = strip_markup(&a.content).to_lowercase();
            let b_text = strip_markup(&b.content).to_lowercase();
            let a_score = reason_support_score(&a_text);
            let b_score = reason_support_score(&b_text);
            let a_explicit = explicit_reason_phrase_score(&a_text);
            let b_explicit = explicit_reason_phrase_score(&b_text);
            b_explicit
                .partial_cmp(&a_explicit)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    b_score
                        .partial_cmp(&a_score)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(
                    source_prompt_priority(b, analysis, query)
                        .partial_cmp(&source_prompt_priority(a, analysis, query))
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });

        for source in &reason_supportive {
            let key = (
                source.book_name.clone(),
                source.chapter,
                source.verse_start,
                source.verse_end,
            );
            if seen.insert(key) {
                selected.push(source.clone());
            }
            if selected.len() >= cap {
                return selected;
            }
        }

        for anchor in &reason_supportive {
            for candidate in expanded.iter().filter(|candidate| {
                candidate.book_name == anchor.book_name
                    && candidate.chapter == anchor.chapter
                    && (candidate.verse_start - anchor.verse_start).abs() <= 2
            }) {
                let key = (
                    candidate.book_name.clone(),
                    candidate.chapter,
                    candidate.verse_start,
                    candidate.verse_end,
                );
                if seen.insert(key) {
                    selected.push(candidate.clone());
                }
                if selected.len() >= cap {
                    return selected;
                }
            }
        }
    }

    for source in &ranked_positive {
        let key = (
            source.book_name.clone(),
            source.chapter,
            source.verse_start,
            source.verse_end,
        );
        if seen.insert(key) {
            selected.push(source.clone());
        }
        if selected.len() >= cap {
            return selected;
        }
    }

    for source in expanded.iter().filter(|s| s.similarity == 0.0) {
        let key = (
            source.book_name.clone(),
            source.chapter,
            source.verse_start,
            source.verse_end,
        );
        if seen.insert(key) {
            selected.push(source.clone());
        }
        if selected.len() >= cap {
            break;
        }
    }

    selected
}

async fn rerank_sources_for_prompt(
    llm: &LlmClient,
    query: &str,
    sources: &[Source],
    analysis: &query_analyzer::QueryAnalysis,
    query_embedding: Option<&[f32]>,
) -> Vec<Source> {
    let owned_query_embedding;
    let Some(query_embedding) = (if let Some(embedding) = query_embedding {
        Some(embedding)
    } else {
        owned_query_embedding = llm.embed(query).await.ok();
        owned_query_embedding.as_deref()
    }) else {
        let mut fallback = sources.to_vec();
        fallback.sort_by(|a, b| {
            source_prompt_priority(b, analysis, query)
                .partial_cmp(&source_prompt_priority(a, analysis, query))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.book_name.cmp(&b.book_name))
                .then(a.chapter.cmp(&b.chapter))
                .then(a.verse_start.cmp(&b.verse_start))
                .then(a.verse_end.cmp(&b.verse_end))
        });
        return fallback;
    };

    let source_texts = sources
        .iter()
        .map(|source| strip_markup(&source.content))
        .collect::<Vec<_>>();
    let source_embeddings = llm.embed_many(&source_texts).await.ok();
    let mut scored = Vec::with_capacity(sources.len());
    for (index, source) in sources.iter().enumerate() {
        let source_text = strip_markup(&source.content);
        let semantic_similarity = source_embeddings
            .as_ref()
            .and_then(|embeddings| embeddings.get(index))
            .map(|source_embedding| cosine_similarity(query_embedding, source_embedding))
            .unwrap_or(0.0);
        let reason_bonus = if analysis.query_intent == query_analyzer::QueryIntent::FindReason {
            reason_support_score(&source_text.to_lowercase()) * 0.05
        } else {
            0.0
        };
        let final_score = source_prompt_priority(source, analysis, query)
            + semantic_similarity * 0.45
            + reason_bonus;
        scored.push((final_score, source.clone()));
    }

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.book_name.cmp(&b.1.book_name))
            .then(a.1.chapter.cmp(&b.1.chapter))
            .then(a.1.verse_start.cmp(&b.1.verse_start))
    });

    scored.into_iter().map(|(_, source)| source).collect()
}

fn is_reason_supportive(text: &str) -> bool {
    reason_support_score(&text.to_lowercase()) > 0.0
}

fn reason_support_score(lower: &str) -> f32 {
    let explanatory_markers = [
        "because",
        "therefore",
        "so that",
        "thus",
        "reason",
        "why",
        "for the earth",
        "for they",
        "for all",
        "for every",
    ];
    let evaluative_terms = [
        "evil",
        "wicked",
        "wickedness",
        "corrupt",
        "violence",
        "regretted",
        "grieved",
        "deeply troubled",
        "troubled",
        "sorry",
        "afraid",
        "anger",
        "hate",
        "favor",
    ];

    let marker_score = explanatory_markers
        .iter()
        .filter(|marker| lower.contains(**marker))
        .count() as f32;
    let evaluative_score = evaluative_terms
        .iter()
        .filter(|term| lower.contains(**term))
        .count() as f32;

    marker_score + evaluative_score
}

/// Build a "lexical notes" string by walking alignment nodes linked to the retrieved verses
/// and reading the `usage` field from each linked lemma node. The usage field contains
/// comma-separated English translation variants (e.g. "grieve, hurt, be sorry, vex") that
/// cover all translation traditions, not just the stored translation. This string is injected
/// into the synthesis prompt so the LLM naturally includes cross-translation synonyms.
async fn build_lexical_notes(state: &Arc<ServerState>, sources: &[Source]) -> String {
    // Noise words to exclude from gloss tokens — parenthetical markers, connectives, etc.
    let noise: HashSet<&str> = [
        "a", "an", "the", "to", "be", "or", "and", "but", "not", "as", "by", "in", "of", "on",
        "up", "at", "do", "is", "it", "x", "one", "self", "make", "use", "also", "with", "from",
        "for", "into", "upon", "have", "has", "had", "put", "get", "give", "out", "off", "let",
        "over", "under", "away", "him", "her", "his", "its", "they", "them", "their", "that",
        "this", "than", "only", "very",
    ]
    .into_iter()
    .collect();

    // Per-lemma: collect unique gloss tokens
    let mut lemma_entries: Vec<(String, String, Vec<String>)> = Vec::new(); // (strongs_id, lemma_text, tokens)
    let mut seen_strongs: HashSet<String> = HashSet::new();

    for source in sources {
        let Some(verse_uid) = source.verse_uid else {
            continue;
        };

        // Walk alignments edge from verse → AlignmentDetail nodes
        for align_uid in planner::edge_targets(state, verse_uid, "alignments") {
            let Some(lemma_uid) = planner::get_linked_uid(state, align_uid, "lemma") else {
                continue;
            };

            let strongs_id = planner::get_string(state, lemma_uid, "strongsId").unwrap_or_default();
            if strongs_id.is_empty() || !seen_strongs.insert(strongs_id.clone()) {
                continue;
            }

            // `usage` is the full BDB/Gesenius usage string stored on LemmaSourceIndex
            // We search for it via the lemma's linked source index nodes.
            let usage = planner::get_string(state, lemma_uid, "usage").unwrap_or_default();
            let lemma_text = planner::get_string(state, lemma_uid, "lemmaText").unwrap_or_default();

            if usage.is_empty() {
                continue;
            }

            // Tokenize: split on punctuation, parens, hyphens, keep alpha words ≥3 chars
            let tokens: Vec<String> = usage
                .split(|c: char| !c.is_alphabetic() && c != '\'')
                .map(|t| t.trim().to_lowercase())
                .filter(|t| t.len() >= 3 && !noise.contains(t.as_str()))
                .fold(Vec::new(), |mut acc, t| {
                    if !acc.contains(&t) {
                        acc.push(t);
                    }
                    acc
                });

            if tokens.is_empty() {
                continue;
            }

            lemma_entries.push((strongs_id, lemma_text, tokens));
        }
    }

    tracing::debug!(
        "build_lexical_notes: {} sources, {} unique lemmas with usage found",
        sources.len(),
        lemma_entries.len()
    );

    if lemma_entries.is_empty() {
        return String::new();
    }

    // Build a compact multi-line note block
    lemma_entries
        .iter()
        .take(6) // cap to avoid bloating the prompt
        .map(|(strongs, lemma, tokens)| {
            format!(
                "- {} {} (also rendered: {})",
                strongs,
                if lemma.is_empty() { "" } else { lemma.as_str() },
                tokens.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn explicit_reason_phrase_score(lower: &str) -> f32 {
    let direct_reason_markers = [
        "because of",
        "filled with violence",
        "wickedness",
        "only evil",
        "corrupt",
        "regretted",
        "deeply troubled",
        "grieved",
        "sorry",
    ];

    direct_reason_markers
        .iter()
        .filter(|marker| lower.contains(**marker))
        .count() as f32
}

fn filter_generation_annotations(annotation_block: &str) -> Option<String> {
    let filtered = annotation_block
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("Gloss:") && !trimmed.starts_with("Syntax:")
        })
        .map(str::to_string)
        .collect::<Vec<_>>();

    if filtered.is_empty() {
        None
    } else {
        Some(filtered.join("\n"))
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;

    for (l, r) in left.iter().zip(right.iter()) {
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn source_prompt_priority(
    source: &Source,
    analysis: &query_analyzer::QueryAnalysis,
    query: &str,
) -> f32 {
    let mut priority = source.similarity;
    let text = strip_markup(&source.content).to_lowercase();
    for term in significant_query_terms(query) {
        if text.contains(&term) {
            priority += 0.006;
        }
    }

    match analysis.query_intent {
        query_analyzer::QueryIntent::FindReason => {
            priority += reason_support_score(&text) * 0.014;

            for anchor in &analysis.verse_refs {
                if anchor.book == source.book_name && anchor.chapter == source.chapter {
                    let verse_distance = if source.verse_start > anchor.verse_start {
                        source.verse_start - anchor.verse_start
                    } else {
                        anchor.verse_start - source.verse_start
                    };
                    if verse_distance <= 2 {
                        priority += 0.012;
                    }
                }
            }
        }
        query_analyzer::QueryIntent::DescribeAction
        | query_analyzer::QueryIntent::DescribeEvent
        | query_analyzer::QueryIntent::DescribeMethod => {
            let action_terms = [
                "took", "gave", "ate", "built", "offered", "sold", "threw", "opened", "realized",
                "said", "cursed", "crawl", "called",
            ];
            for term in action_terms {
                if text.contains(term) {
                    priority += 0.006;
                }
            }
        }
        _ => {}
    }

    if matches!(
        analysis.narrative_state.temporal_direction,
        query_analyzer::NarrativeDirection::Forward
    ) && analysis.narrative_state.chapter_window.is_some()
    {
        if let Some((_, end)) = analysis.narrative_state.chapter_window {
            let chapter_distance = (end - source.chapter).abs() as f32;
            priority -= chapter_distance * 0.002;
        }
    }

    priority
}

async fn stream_answer(
    llm: &LlmClient,
    system_prompt: &str,
    prompt: &str,
    sender: Option<Sender<Result<StreamEvent, BrainError>>>,
) -> Result<String, BrainError> {
    if sender.is_none() {
        return llm
            .generate(&format!("{}\n\n{}", system_prompt, prompt))
            .await;
    }

    let tx = sender.expect("sender checked as some");
    let mut full_response = String::new();
    let mut rx = llm
        .generate_stream_with_system(Some(system_prompt), prompt)
        .await?;

    while let Some(token) = rx.recv().await {
        full_response.push_str(&token);
        if tx.send(Ok(StreamEvent::Token(token))).await.is_err() {
            warn!("Client stream receiver dropped");
            break;
        }
    }

    let _ = tx.send(Ok(StreamEvent::Done)).await;
    Ok(full_response)
}

async fn synthesize_grounded_answer(
    llm: &LlmClient,
    system_prompt: &str,
    prompt: &str,
    sender: Option<Sender<Result<StreamEvent, BrainError>>>,
) -> Result<String, BrainError> {
    let answer = llm
        .generate(&format!("{}\n\n{}", system_prompt, prompt))
        .await?;

    if let Some(tx) = sender {
        if tx
            .send(Ok(StreamEvent::Token(answer.clone())))
            .await
            .is_err()
        {
            warn!("Client stream receiver dropped");
        }
        let _ = tx.send(Ok(StreamEvent::Done)).await;
    }

    Ok(answer)
}
