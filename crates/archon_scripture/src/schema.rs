//! Schema — Mapping from JSONL files to VardaDB types.
//!
//! Each IngestFile defines: which JSONL file to read, the VardaDB type name,
//! field mappings (JSONL key → GraphQL field name), which fields get FTS,
//! which are unique, and which create inverse edges.

/// An inverse edge to write when ingesting a node.
#[derive(Debug, Clone)]
pub struct InverseEdge {
    /// The JSONL field that holds the reference (e.g., "chapter_id")
    pub jsonl_field: &'static str,
    /// The inverse field on the TARGET node
    pub inverse_field: &'static str,
    /// Whether the inverse side is a list (most @hasInverse are list)
    pub inverse_is_list: bool,
}

/// Which FTS table to insert into.
#[derive(Debug, Clone)]
pub enum FtsStrategy {
    /// Insert into fts_data (stemmed, porter unicode61)
    Fulltext,
    /// Insert into fts_term_data (exact term, unicode61)
    Term,
}

/// A field that should be indexed for FTS.
#[derive(Debug, Clone)]
pub struct FtsField {
    pub graphql_name: &'static str,
    pub strategies: Vec<FtsStrategy>,
}

/// Definition for one JSONL→VardaDB ingest file.
#[derive(Debug, Clone)]
pub struct IngestFile {
    pub filename: &'static str,
    pub type_name: &'static str,
    /// Mapping from JSONL field name → VardaDB/GraphQL field name.
    /// If empty, JSONL keys are used as-is.
    pub field_map: Vec<(&'static str, &'static str)>,
    /// Fields that should be FTS-indexed
    pub fts_fields: Vec<FtsField>,
    /// Fields with @unique constraint
    pub unique_fields: Vec<&'static str>,
    /// Inverse edges to write
    pub inverses: Vec<InverseEdge>,
    /// Whether this file has an embedding vector
    /// The JSONL field name for the vector (if has_vector)
    /// Fields that hold references to other nodes (JSONL field → lookup prefix or type)
    /// These will be resolved from the global ID→UID map.
    pub ref_fields: Vec<(&'static str, &'static str)>,
}

/// Returns ingest files in dependency order.
pub fn ingest_files() -> Vec<IngestFile> {
    vec![
        // 1. languages.jsonl → Language
        IngestFile {
            filename: "languages.jsonl",
            type_name: "Language",
            field_map: vec![("code", "code"), ("name", "name")],
            fts_fields: vec![
                FtsField {
                    graphql_name: "code",
                    strategies: vec![FtsStrategy::Term],
                },
                FtsField {
                    graphql_name: "name",
                    strategies: vec![FtsStrategy::Term, FtsStrategy::Fulltext],
                },
            ],
            unique_fields: vec!["code"],
            inverses: vec![],
            ref_fields: vec![],
        },
        // 2. translations.jsonl → Translations
        IngestFile {
            filename: "translations.jsonl",
            type_name: "Translations",
            field_map: vec![
                ("code", "code"),
                ("name", "name"),
                ("language_id", "language"),
            ],
            fts_fields: vec![
                FtsField {
                    graphql_name: "code",
                    strategies: vec![FtsStrategy::Term],
                },
                FtsField {
                    graphql_name: "name",
                    strategies: vec![FtsStrategy::Term, FtsStrategy::Fulltext],
                },
            ],
            unique_fields: vec!["code"],
            inverses: vec![InverseEdge {
                jsonl_field: "language_id",
                inverse_field: "translations",
                inverse_is_list: true,
            }],
            ref_fields: vec![("language_id", "")],
        },
        // 3. categories.jsonl → Category
        IngestFile {
            filename: "categories.jsonl",
            type_name: "Category",
            field_map: vec![("name", "name")],
            fts_fields: vec![FtsField {
                graphql_name: "name",
                strategies: vec![FtsStrategy::Term, FtsStrategy::Fulltext],
            }],
            unique_fields: vec!["name"],
            inverses: vec![],
            ref_fields: vec![],
        },
        // 4. books.jsonl → Book
        IngestFile {
            filename: "books.jsonl",
            type_name: "Book",
            field_map: vec![
                ("code", "code"),
                ("name_en", "nameEn"),
                ("name_he", "nameHe"),
                ("book_corpus", "bookCorpus"),
                ("testament", "testament"),
            ],
            fts_fields: vec![FtsField {
                graphql_name: "nameEn",
                strategies: vec![FtsStrategy::Term, FtsStrategy::Fulltext],
            }],
            unique_fields: vec!["code"],
            inverses: vec![],
            ref_fields: vec![],
        },
        // 5. book_translations.jsonl → BookTranslation
        IngestFile {
            filename: "book_translations.jsonl",
            type_name: "BookTranslation",
            field_map: vec![("book_id", "book"), ("translation_id", "translation")],
            fts_fields: vec![],
            unique_fields: vec![],
            inverses: vec![
                InverseEdge {
                    jsonl_field: "book_id",
                    inverse_field: "bookTranslations",
                    inverse_is_list: true,
                },
                InverseEdge {
                    jsonl_field: "translation_id",
                    inverse_field: "bookTranslations",
                    inverse_is_list: true,
                },
            ],
            ref_fields: vec![("book_id", ""), ("translation_id", "")],
        },
        // 6. book_categories.jsonl → BookCategory
        IngestFile {
            filename: "book_categories.jsonl",
            type_name: "BookCategory",
            field_map: vec![("book_id", "book"), ("category_id", "category")],
            fts_fields: vec![],
            unique_fields: vec![],
            inverses: vec![
                InverseEdge {
                    jsonl_field: "book_id",
                    inverse_field: "bookCategories",
                    inverse_is_list: true,
                },
                InverseEdge {
                    jsonl_field: "category_id",
                    inverse_field: "bookCategories",
                    inverse_is_list: true,
                },
            ],
            ref_fields: vec![("book_id", ""), ("category_id", "")],
        },
        // 7. chapters.jsonl → Chapter
        IngestFile {
            filename: "chapters.jsonl",
            type_name: "Chapter",
            field_map: vec![
                ("number", "number"),
                ("book_translation_id", "bookTranslation"),
            ],
            fts_fields: vec![],
            unique_fields: vec![],
            inverses: vec![InverseEdge {
                jsonl_field: "book_translation_id",
                inverse_field: "chapters",
                inverse_is_list: true,
            }],
            ref_fields: vec![("book_translation_id", "")],
        },
        // 8. verses.jsonl → Verse
        IngestFile {
            filename: "verses.jsonl",
            type_name: "Verse",
            field_map: vec![
                ("number", "number"),
                ("logion_number", "logionNumber"),
                ("chunk_type", "chunkType"),
                ("chapter_heading", "chapterHeading"),
                ("section_heading", "sectionHeading"),
                ("paragraph_start", "paragraphStart"),
                ("poetry_indent", "poetryIndent"),
                ("metadata", "metadata"),
                ("chapter_id", "chapter"),
            ],
            fts_fields: vec![],
            unique_fields: vec![],
            inverses: vec![InverseEdge {
                jsonl_field: "chapter_id",
                inverse_field: "verses",
                inverse_is_list: true,
            }],
            ref_fields: vec![("chapter_id", "")],
        },
        // 9. verse_contents.jsonl → VerseContent
        IngestFile {
            filename: "verse_contents.jsonl",
            type_name: "VerseContent",
            field_map: vec![
                ("verse_id", "verse"),
                ("translation_id", "translation"),
                ("type", "type"),
                ("text", "text"),
                ("lemma", "lemma"),
                ("strong", "strong"),
                ("morph", "morph"),
                ("gloss", "gloss"),
                ("token_id", "tokenId"),
                ("token_index", "tokenIndex"),
                ("normalized_text", "normalizedText"),
            ],
            fts_fields: vec![FtsField {
                graphql_name: "text",
                strategies: vec![FtsStrategy::Fulltext],
            }],
            unique_fields: vec![],
            inverses: vec![
                InverseEdge {
                    jsonl_field: "verse_id",
                    inverse_field: "verseContents",
                    inverse_is_list: true,
                },
                InverseEdge {
                    jsonl_field: "translation_id",
                    inverse_field: "verseContents",
                    inverse_is_list: true,
                },
            ],
            ref_fields: vec![("verse_id", ""), ("translation_id", "")],
        },
        // 10. interlinear_tokens.jsonl → InterlinearToken
        IngestFile {
            filename: "interlinear_tokens.jsonl",
            type_name: "InterlinearToken",
            field_map: vec![
                ("verse_id", "verse"),
                ("translation_id", "translation"),
                ("language_code", "languageCode"),
                ("token_index", "tokenIndex"),
                ("strong", "strong"),
                ("reference", "reference"),
                ("transliteration", "transliteration"),
                ("original", "original"),
                ("english_gloss", "englishGloss"),
                ("grammar", "grammar"),
                ("metadata", "metadata"),
            ],
            fts_fields: vec![
                FtsField {
                    graphql_name: "strong",
                    strategies: vec![FtsStrategy::Term],
                },
                FtsField {
                    graphql_name: "reference",
                    strategies: vec![FtsStrategy::Term],
                },
                FtsField {
                    graphql_name: "transliteration",
                    strategies: vec![FtsStrategy::Term, FtsStrategy::Fulltext],
                },
                FtsField {
                    graphql_name: "original",
                    strategies: vec![FtsStrategy::Term, FtsStrategy::Fulltext],
                },
                FtsField {
                    graphql_name: "englishGloss",
                    strategies: vec![FtsStrategy::Fulltext],
                },
                FtsField {
                    graphql_name: "grammar",
                    strategies: vec![FtsStrategy::Term, FtsStrategy::Fulltext],
                },
            ],
            unique_fields: vec![],
            inverses: vec![InverseEdge {
                jsonl_field: "verse_id",
                inverse_field: "interlinearTokens",
                inverse_is_list: true,
            }],
            ref_fields: vec![("verse_id", ""), ("translation_id", "")],
        },
        // 11. chunks.jsonl → Chunk
        IngestFile {
            filename: "chunks.jsonl",
            type_name: "Chunk",
            field_map: vec![
                ("chunk_type", "chunkType"),
                ("text", "text"),
                ("start_verse_id", "startVerse"),
                ("end_verse_id", "endVerse"),
            ],
            fts_fields: vec![FtsField {
                graphql_name: "text",
                strategies: vec![FtsStrategy::Fulltext],
            }],
            unique_fields: vec![],
            inverses: vec![],
            ref_fields: vec![("start_verse_id", ""), ("end_verse_id", "")],
        },
    ]
}
