use crate::brain::query_analyzer::{NarrativeState, QueryIntent, VerseRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRequest {
    pub question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub answer: String,
    pub sources: Vec<Source>,
    pub metadata: QueryMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryMetadata {
    pub complexity: QueryComplexity,
    pub source_count: usize,
    pub processing_time_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum QueryComplexity {
    Simple,
    Complex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum RetrievalArmKind {
    Reference,
    Chapter,
    Text,
    VerseVector,
    ChunkVector,
    Entity,
    EntityEdge,
    Syntax,
    Lemma,
    Alignment,
    Synonym,
    LexiconVector,
}

impl RetrievalArmKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Chapter => "chapter",
            Self::Text => "text",
            Self::VerseVector => "verse_vector",
            Self::ChunkVector => "chunk_vector",
            Self::Entity => "entity",
            Self::EntityEdge => "entity_edge",
            Self::Syntax => "syntax",
            Self::Lemma => "lemma",
            Self::Alignment => "alignment",
            Self::Synonym => "synonym",
            Self::LexiconVector => "lexicon_vector",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalArm {
    pub kind: RetrievalArmKind,
    pub query_text: String,
    pub limit: usize,
    pub weight: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalyzedQuery {
    pub resolved_query: String,
    pub book_filters: Option<Vec<String>>,
    pub chapter_filter: Option<i32>,
    pub narrative_state: NarrativeState,
    pub verse_refs: Vec<VerseRef>,
    pub entity_names: Vec<String>,
    pub strong_numbers: Vec<String>,
    pub lemma_terms: Vec<String>,
    pub synset_ids: Vec<String>,
    pub synset_glosses: Vec<String>,
    pub key_concepts: Vec<String>,
    pub query_intent: QueryIntent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalPlan {
    pub analyzed_query: AnalyzedQuery,
    pub arms: Vec<RetrievalArm>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArmTrace {
    pub arm: String,
    pub rank: usize,
    pub contribution: f32,
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    #[serde(skip)]
    pub uid: u64,
    pub chunk_type: String,
    pub book_name: String,
    pub testament: String,
    pub chapter: i32,
    pub verse_start: i32,
    pub verse_end: i32,
    pub chapter_heading: Option<String>,
    pub section_heading: Option<String>,
    pub content: String,
    pub similarity: f32,
    pub translation: String,
    pub verse_uid: Option<u64>,
    #[serde(skip)]
    pub traces: Vec<ArmTrace>,
    #[serde(skip)]
    pub evidence: Vec<String>,
    #[serde(skip)]
    pub is_context: bool,
}

impl Source {
    pub fn dedupe_key(&self) -> (String, i32, i32, i32, String, String) {
        (
            self.book_name.clone(),
            self.chapter,
            self.verse_start,
            self.verse_end,
            self.translation.clone(),
            self.chunk_type.clone(),
        )
    }
}

#[derive(Debug, Deserialize)]
pub struct GenerateResponse {
    pub response: String,
    pub done: bool,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingResponse {
    pub embedding: Vec<f32>,
}
