use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use clap::Args;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{Connection, OpenFlags, params};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

const LOCAL_DB_NAME: &str = "new_gospel";
const ARCHON_DB_NAME: &str = "archondb";
const LOCAL_VERSE_TYPE: &str = "NewGospelVerse";
const LOCAL_SUPPORT_TYPE: &str = "SupportingReference";
const SUPPORT_FIELD: &str = "supportingReferences";

const SNAPSHOT_TYPE: &str = "ScriptureComparisonSnapshot";
const COMPARISON_VERSE_TYPE: &str = "ScriptureComparisonVerse";
const COMPARISON_VERSE_TEXT_TYPE: &str = "ScriptureComparisonVerseText";
const COMPARISON_METRIC_TYPE: &str = "ScriptureComparisonMetric";
const COMPARISON_PAIR_SUMMARY_TYPE: &str = "ScriptureComparisonPairSummary";
const COMPARISON_HISTOGRAM_BUCKET_TYPE: &str = "ScriptureComparisonHistogramBucket";

const RESERVED_UID_START: u64 = 9_000_000_000;
const RESERVED_UID_END: u64 = 9_199_999_999;

const DEFAULT_TRANSLATIONS: &[&str] = &["NIV", "ESV", "NKJV", "NLT", "CSB", "NRSVUE", "LEB", "NET"];
const DEFAULT_BOOKS: &[&str] = &["Mark", "Matthew", "Luke", "John", "Thomas"];
const THOMAS_TRANSLATION_CODE: &str = "mark-m-mattison";
const VERBATIM_RUN_FLAG_WORDS: i64 = 7;

static ANNOTATION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[(?:vs:\d+|fn:[^\]]*|cr:[^\]]*)\]").expect("valid annotation marker regex")
});

static FULL_REFERENCE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^(?P<book>[1-3]?\s*[A-Za-z]+(?:\s+[A-Za-z]+)*)\s+(?P<chapter>\d+):(?P<verse>\d+)(?:-\d+)?$",
    )
    .expect("valid full reference regex")
});
static CHAPTER_VERSE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(?P<chapter>\d+):(?P<verse>\d+)(?:-\d+)?$").expect("valid chapter verse regex")
});
static VERSE_ONLY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(?P<verse>\d+)(?:-\d+)?$").expect("valid verse only regex"));

#[derive(Args, Debug)]
#[command(
    about = "Compare local gospel verses against copyrighted translations and persist a readonly snapshot"
)]
pub struct Cli {
    /// Override the registered archondb path.
    #[arg(long)]
    archon_db: Option<PathBuf>,

    /// Override the registered new_gospel path.
    #[arg(long)]
    local_db: Option<PathBuf>,

    #[arg(long, default_value_os_t = default_log_dir())]
    log_dir: PathBuf,

    #[arg(long, value_delimiter = ',', default_values_t = default_translations())]
    translations: Vec<String>,

    #[arg(long, value_delimiter = ',', default_values_t = default_books())]
    books: Vec<String>,

    #[arg(long, default_value_t = 5.0)]
    threshold_buffer: f32,

    #[arg(long, default_value = "mxbai")]
    embedding_model: String,

    #[arg(long, default_value_t = true)]
    fail_on_missing_translation: bool,
}

#[derive(Debug, Clone)]
struct PartialRecord {
    type_name: Option<String>,
    fields: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
struct TranslationRecord {
    uid: u64,
    code: String,
    name: String,
}

#[derive(Debug, Clone)]
struct BookRecord {
    uid: u64,
    code: String,
    name: String,
}

#[derive(Debug, Clone)]
struct BookTranslationRecord {
    book_uid: u64,
    translation_uid: u64,
}

#[derive(Debug, Clone)]
struct ChapterRecord {
    number: i64,
    book_translation_uid: u64,
}

#[derive(Debug, Clone)]
struct VerseRecord {
    uid: u64,
    number: i64,
    chapter_uid: u64,
}

#[derive(Debug, Clone)]
struct VerseContentRecord {
    verse_uid: u64,
    translation_uid: u64,
    text: String,
    content_type: String,
}

#[derive(Debug, Clone)]
struct VerseRef {
    book_code: String,
    book_name: String,
    chapter: i64,
    verse: i64,
}

impl VerseRef {
    fn label(&self) -> String {
        format!("{} {}:{}", self.book_name, self.chapter, self.verse)
    }
}

impl PartialEq for VerseRef {
    fn eq(&self, other: &Self) -> bool {
        self.book_code == other.book_code
            && self.chapter == other.chapter
            && self.verse == other.verse
    }
}

impl Eq for VerseRef {}

impl std::hash::Hash for VerseRef {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.book_code.hash(state);
        self.chapter.hash(state);
        self.verse.hash(state);
    }
}

#[derive(Debug, Clone)]
struct CopyrightVerse {
    reference: VerseRef,
    texts_by_translation: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct LocalVerse {
    uid: u64,
    number: i64,
    text: String,
    supporting_references: Vec<String>,
}

#[derive(Debug, Clone)]
struct LocalMatch {
    verse_uid: u64,
    verse_number: i64,
    text: String,
    supporting_reference_display: String,
    match_ordinal: i64,
}

#[derive(Debug, Clone)]
struct NormalizedText {
    text: String,
    tokens: Vec<String>,
}

#[derive(Debug, Clone)]
struct MetricSet {
    sequence_similarity_pct: f32,
    ngram_containment_pct: f32,
    token_overlap_pct: f32,
    semantic_similarity_pct: f32,
    longest_common_run_words: i64,
    longest_common_run_text: String,
}

#[derive(Debug, Clone)]
struct PairMetric {
    reference: VerseRef,
    left_code: String,
    right_code: String,
    metrics: MetricSet,
}

#[derive(Debug, Clone)]
struct LocalComparisonMetric {
    reference: VerseRef,
    local_match: LocalMatch,
    right_code: String,
    metrics: MetricSet,
    baseline_delta_pct: f32,
    exceeds_threshold: bool,
}

#[derive(Debug, Clone)]
struct ParseErrorEntry {
    verse_uid: u64,
    verse_number: i64,
    display: String,
    reason: String,
}

#[derive(Debug, Clone)]
struct UnmatchedVerseEntry {
    reference: VerseRef,
    texts_by_translation: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct PairSummary {
    word_run_counts: [i64; 7],
    comparison_kind: String,
    left_code: String,
    right_code: String,
    average_sequence_similarity_pct: f32,
    average_ngram_containment_pct: f32,
    average_token_overlap_pct: f32,
    average_semantic_similarity_pct: f32,
    average_longest_common_run_words: f32,
    min_sequence_similarity_pct: f32,
    max_sequence_similarity_pct: f32,
    min_ngram_containment_pct: f32,
    max_ngram_containment_pct: f32,
    verse_count: i64,
}

#[derive(Debug, Clone)]
struct HistogramBucket {
    bucket_start: i64,
    bucket_end: i64,
    count: i64,
}

#[derive(Debug, Clone)]
struct SnapshotSummary {
    baseline_mean_pct: f32,
    threshold_pct: f32,
    total_copyrighted_verses: i64,
    matched_local_verses: i64,
    unmatched_copyrighted_verses: i64,
    parse_error_count: i64,
}

#[derive(Debug, Clone)]
struct ArchonData {
    translations: HashMap<u64, TranslationRecord>,
    books: HashMap<u64, BookRecord>,
    book_translations: HashMap<u64, BookTranslationRecord>,
    chapters: HashMap<u64, ChapterRecord>,
    verses: HashMap<u64, VerseRecord>,
    verse_contents: Vec<VerseContentRecord>,
}

#[derive(Debug, Clone, Default)]
struct DbTables {
    main: String,
    history: String,
}

impl DbTables {
    fn for_db(name: &str) -> Self {
        Self {
            main: format!("{name}_main"),
            history: format!("{name}_history"),
        }
    }
}

struct UidAllocator {
    next_uid: u64,
}

impl UidAllocator {
    fn new(start: u64) -> Self {
        Self { next_uid: start }
    }

    fn next(&mut self) -> u64 {
        let uid = self.next_uid;
        self.next_uid += 1;
        uid
    }
}

struct EmbeddingEngine {
    model: TextEmbedding,
    cache: HashMap<String, Vec<f32>>,
}

struct ProgressReporter {
    multi: MultiProgress,
}

impl ProgressReporter {
    fn new() -> Self {
        Self {
            multi: MultiProgress::new(),
        }
    }

    fn phase(&self, message: impl Into<String>) {
        println!("{}", message.into());
    }

    fn spinner(&self, message: impl Into<String>) -> ProgressBar {
        let bar = self.multi.add(ProgressBar::new_spinner());
        bar.set_style(
            ProgressStyle::with_template("{spinner:.yellow} {msg}")
                .expect("valid spinner template"),
        );
        bar.enable_steady_tick(std::time::Duration::from_millis(100));
        bar.set_message(message.into());
        bar
    }

    fn progress_bar(&self, length: u64, message: impl Into<String>) -> ProgressBar {
        let bar = self.multi.add(ProgressBar::new(length));
        bar.set_style(
            ProgressStyle::with_template("{msg} [{bar:40.cyan/blue}] {pos}/{len} {percent}%")
                .expect("valid bar template")
                .progress_chars("=> "),
        );
        bar.set_message(message.into());
        bar
    }
}

impl EmbeddingEngine {
    fn new(model_name: &str) -> Result<Self> {
        let embedding_model = match model_name.trim().to_ascii_lowercase().as_str() {
            "mxbai" | "mxbai-embed-large-v1" => EmbeddingModel::MxbaiEmbedLargeV1,
            other => bail!("unsupported embedding model '{other}'"),
        };

        let model = TextEmbedding::try_new(
            InitOptions::new(embedding_model).with_show_download_progress(true),
        )
        .context("initializing fastembed model")?;

        Ok(Self {
            model,
            cache: HashMap::new(),
        })
    }

    fn embedding_for(&mut self, text: &str) -> Result<Vec<f32>> {
        let key = text.trim().to_string();
        if key.is_empty() {
            return Ok(Vec::new());
        }

        if let Some(existing) = self.cache.get(&key) {
            return Ok(existing.clone());
        }

        let embedding = self
            .model
            .embed(vec![key.clone()], None)
            .context("computing embedding")?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("embedding model returned no vectors"))?;

        self.cache.insert(key, embedding.clone());
        Ok(embedding)
    }
}

pub fn run(cli: Cli) -> Result<()> {
    let (archon_db, local_db) = resolve_database_paths(&cli)?;
    let effective_translations =
        resolve_effective_translation_requests(&cli.books, &cli.translations);
    let (canonical_books, gnostic_books) = split_books_by_tradition(&cli.books);
    let canonical_translations: Vec<String> = effective_translations
        .iter()
        .filter(|translation| !is_thomas_translation(translation))
        .cloned()
        .collect();
    let gnostic_translations: Vec<String> = if gnostic_books.is_empty() {
        Vec::new()
    } else {
        vec![THOMAS_TRANSLATION_CODE.to_string()]
    };
    let progress = ProgressReporter::new();
    progress.phase("Starting scripture comparison");
    progress.phase(format!("canonical books={}", canonical_books.join(", ")));
    progress.phase(format!(
        "canonical translations={}",
        canonical_translations.join(", ")
    ));
    progress.phase(format!("gnostic books={}", gnostic_books.join(", ")));
    progress.phase(format!(
        "gnostic translations={}",
        gnostic_translations.join(", ")
    ));
    progress.phase(format!("Source Archon DB: {}", archon_db.display()));
    progress.phase(format!(
        "Target local DB: {} (writes only new_gospel keyspace)",
        local_db.display()
    ));
    progress.phase(format!("Log directory: {}", cli.log_dir.display()));

    let archon_conn = Connection::open_with_flags(
        &archon_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("opening archon db {}", archon_db.display()))?;
    let mut local_conn = Connection::open(&local_db)
        .with_context(|| format!("opening local db {}", local_db.display()))?;

    let archon_tables = DbTables::for_db(ARCHON_DB_NAME);
    let local_tables = DbTables::for_db(LOCAL_DB_NAME);

    let load_archon_spinner = progress.spinner("Loading Archon scripture metadata");
    let archon_data = load_archon_data(&archon_conn, &archon_tables)?;
    load_archon_spinner.finish_with_message("Loaded Archon scripture metadata");
    let selected_books = resolve_selected_books(&archon_data.books, &cli.books)?;
    let selected_translations = resolve_selected_translations(
        &archon_data.translations,
        &effective_translations,
        cli.fail_on_missing_translation,
    )?;

    progress.phase("Phase 1/4: collecting copyrighted scripture by book and chapter");
    let copyrighted_verses = build_copyrighted_verses(
        &archon_data,
        &selected_books,
        &selected_translations,
        &progress,
    )?;
    let load_local_spinner = progress.spinner("Loading local NewGospelVerse records");
    let local_verses = load_local_verses(&local_conn, &local_tables)?;
    load_local_spinner.finish_with_message(format!(
        "Loaded local NewGospelVerse records ({})",
        local_verses.len()
    ));

    progress.phase("Phase 2/4: initializing semantic embedding model");
    let mut embeddings = EmbeddingEngine::new(&cli.embedding_model)?;
    progress.phase("Phase 3/4: comparing copyrighted translations against each other");
    let baseline_metrics =
        compute_baseline_metrics(&copyrighted_verses, &mut embeddings, &progress)?;
    let baseline_mean_pct = mean(
        baseline_metrics
            .iter()
            .map(|metric| metric.metrics.sequence_similarity_pct),
    );
    let threshold_pct = (baseline_mean_pct - cli.threshold_buffer).max(0.0);

    progress.phase(format!(
        "Dynamic threshold established: baseline mean {:.2}% minus buffer {:.2}% = {:.2}%",
        baseline_mean_pct, cli.threshold_buffer, threshold_pct
    ));
    progress.phase("Phase 4/4: comparing copyrighted scripture against local gospel");
    let (
        local_metrics,
        parse_error_count,
        unmatched_copyrighted_verses,
        matched_local_verses,
        parse_errors,
        unmatched_verses,
    ) = compute_local_comparisons(
        &copyrighted_verses,
        &local_verses,
        threshold_pct,
        baseline_mean_pct,
        &mut embeddings,
        &progress,
    )?;

    let snapshot_summary = SnapshotSummary {
        baseline_mean_pct,
        threshold_pct,
        total_copyrighted_verses: copyrighted_verses.len() as i64,
        matched_local_verses,
        unmatched_copyrighted_verses,
        parse_error_count,
    };

    write_run_logs(
        &cli.log_dir,
        &snapshot_summary,
        &cli.books,
        &effective_translations,
        &baseline_metrics,
        &local_metrics,
        &parse_errors,
        &unmatched_verses,
    )?;

    let persist_spinner = progress.spinner("Persisting comparison snapshot to local database");
    persist_snapshot(
        &mut local_conn,
        &local_tables,
        &cli.books,
        &effective_translations,
        cli.threshold_buffer,
        &snapshot_summary,
        &copyrighted_verses,
        &baseline_metrics,
        &local_metrics,
    )?;
    persist_spinner.finish_with_message("Persisted comparison snapshot to local database");

    println!(
        "Persisted comparison snapshot: baseline_mean={:.2}% threshold={:.2}% copyrighted_verses={} matched_local={} unmatched={} parse_errors={}",
        snapshot_summary.baseline_mean_pct,
        snapshot_summary.threshold_pct,
        snapshot_summary.total_copyrighted_verses,
        snapshot_summary.matched_local_verses,
        snapshot_summary.unmatched_copyrighted_verses,
        snapshot_summary.parse_error_count
    );

    Ok(())
}

fn resolve_database_paths(cli: &Cli) -> Result<(PathBuf, PathBuf)> {
    if let (Some(archon_db), Some(local_db)) = (&cli.archon_db, &cli.local_db) {
        return Ok((archon_db.clone(), local_db.clone()));
    }

    let config = vardadb::config::VardaConfig::load_from_file("config.toml")
        .map_err(|err| anyhow!("loading config.toml: {err}"))?;
    // Read the registry used by VardaDB's db list without initializing storage.
    let registry_path = Path::new(&config.server.storage_path).join("default.db");
    let registry = Connection::open_with_flags(&registry_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening database registry {}", registry_path.display()))?;
    let resolve = |name: &str, override_path: &Option<PathBuf>| -> Result<PathBuf> {
        if let Some(path) = override_path {
            return Ok(path.clone());
        }

        let key = format!("db:{name}");
        let value = registry
            .query_row(
                "SELECT value FROM sys WHERE key = ?1",
                [key.as_bytes()],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .with_context(|| format!("looking up '{name}' in {}", registry_path.display()))?;
        let path = String::from_utf8(value)
            .with_context(|| format!("invalid registered path for '{name}'"))?;
        Ok(PathBuf::from(path))
    };

    Ok((
        resolve(ARCHON_DB_NAME, &cli.archon_db)?,
        resolve(LOCAL_DB_NAME, &cli.local_db)?,
    ))
}

fn default_log_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("compare_logs")
}

fn default_translations() -> Vec<String> {
    DEFAULT_TRANSLATIONS
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

fn default_books() -> Vec<String> {
    DEFAULT_BOOKS
        .iter()
        .map(|value| (*value).to_string())
        .collect()
}

fn resolve_effective_translation_requests(
    requested_books: &[String],
    requested_translations: &[String],
) -> Vec<String> {
    let includes_thomas = requested_books
        .iter()
        .any(|book| matches!(to_book_code(book).as_deref(), Some("Thomas")));

    let mut effective = requested_translations.to_vec();
    if includes_thomas && !effective.iter().any(|code| is_thomas_translation(code)) {
        effective.push(THOMAS_TRANSLATION_CODE.to_string());
    }

    effective
}

fn split_books_by_tradition(requested_books: &[String]) -> (Vec<String>, Vec<String>) {
    let mut canonical = Vec::new();
    let mut gnostic = Vec::new();

    for book in requested_books {
        if matches!(to_book_code(book).as_deref(), Some("Thomas")) {
            gnostic.push(book.clone());
        } else {
            canonical.push(book.clone());
        }
    }

    (canonical, gnostic)
}

fn is_thomas_translation(value: &str) -> bool {
    normalize_comparison_token(value) == normalize_comparison_token(THOMAS_TRANSLATION_CODE)
}

fn load_archon_data(conn: &Connection, tables: &DbTables) -> Result<ArchonData> {
    let mut stmt = conn.prepare(&format!("SELECT key, value FROM \"{}\"", tables.main))?;
    let rows = stmt.query_map([], |row| {
        let key: Vec<u8> = row.get(0)?;
        let value: Vec<u8> = row.get(1)?;
        Ok((key, value))
    })?;

    let wanted_fields: HashSet<&str> = [
        "_type",
        "code",
        "name",
        "nameEn",
        "book",
        "translation",
        "bookTranslation",
        "chapter",
        "verse",
        "number",
        "text",
        "type",
    ]
    .into_iter()
    .collect();

    let mut partials: HashMap<u64, PartialRecord> = HashMap::new();

    for row in rows {
        let (key, value) = row?;
        if key.first().copied() != Some(0x01) || key.len() < 9 || value.len() < 16 {
            continue;
        }

        let uid = u64::from_be_bytes(key[1..9].try_into().expect("uid length checked"));
        let field = String::from_utf8_lossy(&key[9..]).into_owned();
        if !wanted_fields.contains(field.as_str()) {
            continue;
        }

        let parsed = serde_json::from_slice::<Value>(&value[16..])
            .with_context(|| format!("decoding archon field {field} for uid {uid}"))?;

        let partial = partials.entry(uid).or_insert_with(|| PartialRecord {
            type_name: None,
            fields: HashMap::new(),
        });

        if field == "_type" {
            partial.type_name = parsed.as_str().map(ToOwned::to_owned);
        } else {
            partial.fields.insert(field, parsed);
        }
    }

    let mut translations = HashMap::new();
    let mut books = HashMap::new();
    let mut book_translations = HashMap::new();
    let mut chapters = HashMap::new();
    let mut verses = HashMap::new();
    let mut verse_contents = Vec::new();

    for (uid, partial) in partials {
        match partial.type_name.as_deref() {
            Some("Translations") => {
                let code = required_string_field(&partial.fields, "code", uid)?;
                let name = required_string_field(&partial.fields, "name", uid)?;
                translations.insert(uid, TranslationRecord { uid, code, name });
            }
            Some("Book") => {
                let code = required_string_field(&partial.fields, "code", uid)?;
                let name = partial
                    .fields
                    .get("nameEn")
                    .and_then(Value::as_str)
                    .unwrap_or(&code)
                    .to_string();
                books.insert(uid, BookRecord { uid, code, name });
            }
            Some("BookTranslation") => {
                let Some(book_uid) = parse_uid_field_optional(&partial.fields, "book") else {
                    continue;
                };
                let Some(translation_uid) =
                    parse_uid_field_optional(&partial.fields, "translation")
                else {
                    continue;
                };
                book_translations.insert(
                    uid,
                    BookTranslationRecord {
                        book_uid,
                        translation_uid,
                    },
                );
            }
            Some("Chapter") => {
                let number = required_i64_field(&partial.fields, "number", uid)?;
                let Some(book_translation_uid) =
                    parse_uid_field_optional(&partial.fields, "bookTranslation")
                else {
                    continue;
                };
                chapters.insert(
                    uid,
                    ChapterRecord {
                        number,
                        book_translation_uid,
                    },
                );
            }
            Some("Verse") => {
                let number = required_i64_field(&partial.fields, "number", uid)?;
                let Some(chapter_uid) = parse_uid_field_optional(&partial.fields, "chapter") else {
                    continue;
                };
                verses.insert(
                    uid,
                    VerseRecord {
                        uid,
                        number,
                        chapter_uid,
                    },
                );
            }
            Some("VerseContent") => {
                let content_type = required_string_field(&partial.fields, "type", uid)?;
                let Some(text) = partial
                    .fields
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                else {
                    continue;
                };
                let Some(verse_uid) = parse_uid_field_optional(&partial.fields, "verse") else {
                    continue;
                };
                let Some(translation_uid) =
                    parse_uid_field_optional(&partial.fields, "translation")
                else {
                    continue;
                };
                verse_contents.push(VerseContentRecord {
                    verse_uid,
                    translation_uid,
                    text,
                    content_type,
                });
            }
            _ => {}
        }
    }

    Ok(ArchonData {
        translations,
        books,
        book_translations,
        chapters,
        verses,
        verse_contents,
    })
}

fn resolve_selected_books(
    books: &HashMap<u64, BookRecord>,
    requested_books: &[String],
) -> Result<HashMap<u64, BookRecord>> {
    let mut result = HashMap::new();

    for requested in requested_books {
        let requested_normalized = normalize_book_name(requested);
        let matched = books.values().find(|book| {
            normalize_book_name(&book.name) == requested_normalized
                || normalize_book_name(&book.code) == requested_normalized
        });

        let Some(book) = matched else {
            bail!("book '{requested}' was not found in archondb");
        };

        result.insert(book.uid, book.clone());
    }

    Ok(result)
}

fn resolve_selected_translations(
    translations: &HashMap<u64, TranslationRecord>,
    requested_codes: &[String],
    fail_on_missing: bool,
) -> Result<HashMap<u64, TranslationRecord>> {
    let mut result = HashMap::new();
    let mut missing = Vec::new();

    for requested in requested_codes {
        let requested_normalized = normalize_comparison_token(requested);
        let matched = translations.values().find(|translation| {
            translation.code.eq_ignore_ascii_case(requested)
                || translation.name.eq_ignore_ascii_case(requested)
                || normalize_comparison_token(&translation.code) == requested_normalized
                || normalize_comparison_token(&translation.name) == requested_normalized
        });

        match matched {
            Some(translation) => {
                result.insert(translation.uid, translation.clone());
            }
            None => missing.push(requested.clone()),
        }
    }

    if !missing.is_empty() && fail_on_missing {
        bail!(
            "missing required translations in archondb: {}",
            missing.join(", ")
        );
    }

    Ok(result)
}

fn build_copyrighted_verses(
    data: &ArchonData,
    selected_books: &HashMap<u64, BookRecord>,
    selected_translations: &HashMap<u64, TranslationRecord>,
    progress: &ProgressReporter,
) -> Result<Vec<CopyrightVerse>> {
    let selected_translation_uids: HashSet<u64> = selected_translations.keys().copied().collect();
    let selected_book_uids: HashSet<u64> = selected_books.keys().copied().collect();
    let thomas_translation_codes: HashSet<String> = selected_translations
        .values()
        .filter(|translation| is_thomas_translation(&translation.code))
        .map(|translation| translation.code.clone())
        .collect();
    let canonical_translation_codes: HashSet<String> = selected_translations
        .values()
        .filter(|translation| !is_thomas_translation(&translation.code))
        .map(|translation| translation.code.clone())
        .collect();

    let selected_book_translations: HashMap<u64, &BookTranslationRecord> = data
        .book_translations
        .iter()
        .filter_map(|(uid, record)| {
            if selected_book_uids.contains(&record.book_uid)
                && selected_translation_uids.contains(&record.translation_uid)
            {
                Some((*uid, record))
            } else {
                None
            }
        })
        .collect();

    let selected_chapters: HashMap<u64, &ChapterRecord> = data
        .chapters
        .iter()
        .filter_map(|(uid, chapter)| {
            if selected_book_translations.contains_key(&chapter.book_translation_uid) {
                Some((*uid, chapter))
            } else {
                None
            }
        })
        .collect();

    let selected_verses: HashMap<u64, &VerseRecord> = data
        .verses
        .iter()
        .filter_map(|(uid, verse)| {
            if selected_chapters.contains_key(&verse.chapter_uid) {
                Some((*uid, verse))
            } else {
                None
            }
        })
        .collect();

    let mut contents_by_verse: HashMap<u64, Vec<&VerseContentRecord>> = HashMap::new();
    for content in &data.verse_contents {
        if content.content_type == "text"
            && selected_translation_uids.contains(&content.translation_uid)
        {
            contents_by_verse
                .entry(content.verse_uid)
                .or_default()
                .push(content);
        }
    }

    let mut verses_by_chapter: HashMap<u64, Vec<&VerseRecord>> = HashMap::new();
    for verse in selected_verses.values() {
        verses_by_chapter
            .entry(verse.chapter_uid)
            .or_default()
            .push(*verse);
    }

    let mut chapters_by_book: BTreeMap<String, Vec<(u64, &ChapterRecord)>> = BTreeMap::new();
    for (chapter_uid, chapter) in &selected_chapters {
        let book_translation = selected_book_translations
            .get(&chapter.book_translation_uid)
            .ok_or_else(|| anyhow!("missing book translation {}", chapter.book_translation_uid))?;
        let book = selected_books
            .get(&book_translation.book_uid)
            .ok_or_else(|| anyhow!("missing book {}", book_translation.book_uid))?;
        chapters_by_book
            .entry(book.name.clone())
            .or_default()
            .push((*chapter_uid, *chapter));
    }

    let mut by_reference: HashMap<VerseRef, HashMap<String, String>> = HashMap::new();

    for (book_name, mut chapters) in chapters_by_book {
        chapters.sort_by_key(|(_, chapter)| chapter.number);
        let book_bar = progress.progress_bar(
            chapters.len() as u64,
            format!("{book_name}: collecting chapters"),
        );

        for (chapter_index, (chapter_uid, chapter)) in chapters.iter().enumerate() {
            book_bar.set_message(format!(
                "{book_name}: processing chapter {}/{}",
                chapter_index + 1,
                chapters.len()
            ));

            if let Some(verses) = verses_by_chapter.get(chapter_uid) {
                let mut verses = verses.clone();
                verses.sort_by_key(|verse| verse.number);

                for verse in verses {
                    if let Some(contents) = contents_by_verse.get(&verse.uid) {
                        for content in contents {
                            let book_translation = selected_book_translations
                                .get(&chapter.book_translation_uid)
                                .ok_or_else(|| {
                                    anyhow!(
                                        "missing book translation {}",
                                        chapter.book_translation_uid
                                    )
                                })?;
                            let book = selected_books.get(&book_translation.book_uid).ok_or_else(
                                || anyhow!("missing book {}", book_translation.book_uid),
                            )?;
                            let translation = selected_translations
                                .get(&content.translation_uid)
                                .ok_or_else(|| {
                                anyhow!("missing translation {}", content.translation_uid)
                            })?;

                            let canonical_code =
                                to_book_code(&book.name).unwrap_or_else(|| book.code.clone());
                            let reference = VerseRef {
                                book_code: canonical_code,
                                book_name: book.name.clone(),
                                chapter: chapter.number,
                                verse: verse.number,
                            };

                            by_reference
                                .entry(reference)
                                .or_default()
                                .insert(translation.code.clone(), content.text.clone());
                        }
                    }
                }
            }

            book_bar.inc(1);
        }

        book_bar.finish_with_message(format!(
            "{book_name}: collected {} chapters",
            chapters.len()
        ));
    }

    let mut verses: Vec<CopyrightVerse> = by_reference
        .into_iter()
        .filter_map(|(reference, texts_by_translation)| {
            let is_thomas_reference = matches!(
                to_book_code(&reference.book_name).as_deref(),
                Some("Thomas")
            ) || matches!(
                to_book_code(&reference.book_code).as_deref(),
                Some("Thomas")
            );
            let expected_codes = if is_thomas_reference {
                &thomas_translation_codes
            } else {
                &canonical_translation_codes
            };

            if !expected_codes.is_empty()
                && expected_codes
                    .iter()
                    .all(|code| texts_by_translation.contains_key(code))
            {
                Some(CopyrightVerse {
                    reference,
                    texts_by_translation,
                })
            } else {
                None
            }
        })
        .collect();

    verses.sort_by(|left, right| compare_reference(&left.reference, &right.reference));
    Ok(verses)
}

fn load_local_verses(conn: &Connection, tables: &DbTables) -> Result<Vec<LocalVerse>> {
    let mut stmt = conn.prepare(&format!("SELECT key, value FROM \"{}\"", tables.main))?;
    let rows = stmt.query_map([], |row| {
        let key: Vec<u8> = row.get(0)?;
        let value: Vec<u8> = row.get(1)?;
        Ok((key, value))
    })?;

    let mut fields: BTreeMap<u64, BTreeMap<String, Value>> = BTreeMap::new();
    for row in rows {
        let (key, value) = row?;
        if key.first().copied() != Some(0x01) || key.len() < 9 || value.len() < 16 {
            continue;
        }

        let uid = u64::from_be_bytes(key[1..9].try_into().expect("uid length checked"));
        let field = String::from_utf8_lossy(&key[9..]).into_owned();
        let parsed = serde_json::from_slice::<Value>(&value[16..])
            .with_context(|| format!("decoding local field {field} for uid {uid}"))?;
        fields.entry(uid).or_default().insert(field, parsed);
    }

    let mut verses = Vec::new();
    for (uid, record) in &fields {
        if record.get("_type").and_then(Value::as_str) != Some(LOCAL_VERSE_TYPE) {
            continue;
        }

        let number = record
            .get("number")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("missing number for local verse uid {uid}"))?;
        let text = record
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing text for local verse uid {uid}"))?
            .to_string();
        let supporting_references = load_supporting_references(record, &fields)?;

        verses.push(LocalVerse {
            uid: *uid,
            number,
            text,
            supporting_references,
        });
    }

    Ok(verses)
}

fn load_supporting_references(
    record: &BTreeMap<String, Value>,
    fields: &BTreeMap<u64, BTreeMap<String, Value>>,
) -> Result<Vec<String>> {
    let support_uids = record
        .get(SUPPORT_FIELD)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut supporting_references = Vec::with_capacity(support_uids.len());

    for support_uid in support_uids {
        let support_uid = support_uid
            .as_str()
            .ok_or_else(|| anyhow!("support uid must be stored as string"))?
            .parse::<u64>()
            .context("parsing support uid")?;
        let support = fields
            .get(&support_uid)
            .ok_or_else(|| anyhow!("missing support record for uid {support_uid}"))?;
        if support.get("_type").and_then(Value::as_str) != Some(LOCAL_SUPPORT_TYPE) {
            continue;
        }
        if let Some(display) = support.get("display").and_then(Value::as_str) {
            supporting_references.push(display.to_string());
        }
    }

    Ok(supporting_references)
}

fn compute_baseline_metrics(
    copyrighted_verses: &[CopyrightVerse],
    embeddings: &mut EmbeddingEngine,
    progress: &ProgressReporter,
) -> Result<Vec<PairMetric>> {
    let mut metrics = Vec::new();
    let total_pairs: usize = copyrighted_verses
        .iter()
        .map(|verse| {
            let count = verse.texts_by_translation.len();
            (count * count.saturating_sub(1)) / 2
        })
        .sum();
    let progress_bar =
        progress.progress_bar(total_pairs as u64, "Baseline comparisons".to_string());

    for verse in copyrighted_verses {
        progress_bar.set_message(format!("Baseline {}", verse.reference.label()));
        let mut codes: Vec<String> = verse.texts_by_translation.keys().cloned().collect();
        codes.sort();

        for left_index in 0..codes.len() {
            for right_index in (left_index + 1)..codes.len() {
                let left_code = &codes[left_index];
                let right_code = &codes[right_index];
                let left_text = verse
                    .texts_by_translation
                    .get(left_code)
                    .ok_or_else(|| anyhow!("missing left translation text"))?;
                let right_text = verse
                    .texts_by_translation
                    .get(right_code)
                    .ok_or_else(|| anyhow!("missing right translation text"))?;

                metrics.push(PairMetric {
                    reference: verse.reference.clone(),
                    left_code: left_code.clone(),
                    right_code: right_code.clone(),
                    metrics: compare_texts(left_text, right_text, embeddings)?,
                });
                progress_bar.inc(1);
            }
        }
    }

    progress_bar.finish_with_message("Baseline copyrighted-vs-copyrighted comparisons complete");

    Ok(metrics)
}

fn compute_local_comparisons(
    copyrighted_verses: &[CopyrightVerse],
    local_verses: &[LocalVerse],
    threshold_pct: f32,
    baseline_mean_pct: f32,
    embeddings: &mut EmbeddingEngine,
    progress: &ProgressReporter,
) -> Result<(
    Vec<LocalComparisonMetric>,
    i64,
    i64,
    i64,
    Vec<ParseErrorEntry>,
    Vec<UnmatchedVerseEntry>,
)> {
    let mut parsed_index: HashMap<VerseRef, Vec<LocalMatch>> = HashMap::new();
    let mut parse_error_count = 0_i64;
    let mut parse_errors = Vec::new();

    for local_verse in local_verses {
        let mut match_ordinal = 0_i64;
        for display in &local_verse.supporting_references {
            let parsed = parse_reference_display(display);
            if parsed.is_empty() {
                parse_error_count += 1;
                parse_errors.push(ParseErrorEntry {
                    verse_uid: local_verse.uid,
                    verse_number: local_verse.number,
                    display: display.clone(),
                    reason: "Could not parse supporting reference display".to_string(),
                });
                continue;
            }

            for parsed_reference in parsed {
                match_ordinal += 1;
                parsed_index
                    .entry(parsed_reference)
                    .or_default()
                    .push(LocalMatch {
                        verse_uid: local_verse.uid,
                        verse_number: local_verse.number,
                        text: local_verse.text.clone(),
                        supporting_reference_display: display.clone(),
                        match_ordinal,
                    });
            }
        }
    }

    let mut metrics = Vec::new();
    let mut unmatched_copyrighted = 0_i64;
    let mut matched_local = 0_i64;
    let mut unmatched_verses = Vec::new();
    let progress_bar = progress.progress_bar(
        copyrighted_verses.len() as u64,
        "Local comparisons by copyrighted verse".to_string(),
    );

    for verse in copyrighted_verses {
        progress_bar.set_message(format!("Local compare {}", verse.reference.label()));
        let matched = parsed_index.get(&verse.reference);
        let Some(matches) = matched else {
            unmatched_copyrighted += 1;
            unmatched_verses.push(UnmatchedVerseEntry {
                reference: verse.reference.clone(),
                texts_by_translation: verse.texts_by_translation.clone(),
            });
            progress_bar.inc(1);
            continue;
        };

        matched_local += matches.len() as i64;

        for local_match in matches {
            for (translation_code, translation_text) in &verse.texts_by_translation {
                let computed = compare_texts(&local_match.text, translation_text, embeddings)?;
                metrics.push(LocalComparisonMetric {
                    reference: verse.reference.clone(),
                    local_match: local_match.clone(),
                    right_code: translation_code.clone(),
                    baseline_delta_pct: computed.sequence_similarity_pct - baseline_mean_pct,
                    exceeds_threshold: computed.sequence_similarity_pct >= threshold_pct
                        || computed.longest_common_run_words >= VERBATIM_RUN_FLAG_WORDS,
                    metrics: computed,
                });
            }
        }
        progress_bar.inc(1);
    }

    progress_bar.finish_with_message("Copyrighted-vs-local comparisons complete");

    Ok((
        metrics,
        parse_error_count,
        unmatched_copyrighted,
        matched_local,
        parse_errors,
        unmatched_verses,
    ))
}

fn compare_texts(left: &str, right: &str, embeddings: &mut EmbeddingEngine) -> Result<MetricSet> {
    let left_clean = strip_annotations(left);
    let right_clean = strip_annotations(right);
    let left_normalized = normalize_text(&left_clean);
    let right_normalized = normalize_text(&right_clean);
    let sequence_similarity_pct =
        levenshtein_similarity(&left_normalized.tokens, &right_normalized.tokens) * 100.0;
    let ngram_containment_pct =
        ngram_containment(&left_normalized.tokens, &right_normalized.tokens, 4) * 100.0;
    let token_overlap_pct =
        jaccard_similarity(&left_normalized.tokens, &right_normalized.tokens) * 100.0;
    let (longest_common_run_words, longest_common_run_text) =
        longest_common_run(&left_normalized.tokens, &right_normalized.tokens);
    let semantic_similarity_pct = cosine_similarity(
        &embeddings.embedding_for(&left_normalized.text)?,
        &embeddings.embedding_for(&right_normalized.text)?,
    ) * 100.0;

    Ok(MetricSet {
        sequence_similarity_pct,
        ngram_containment_pct,
        token_overlap_pct,
        semantic_similarity_pct,
        longest_common_run_words: longest_common_run_words as i64,
        longest_common_run_text,
    })
}

fn persist_snapshot(
    conn: &mut Connection,
    tables: &DbTables,
    books: &[String],
    translations: &[String],
    threshold_buffer_pct: f32,
    summary: &SnapshotSummary,
    copyrighted_verses: &[CopyrightVerse],
    baseline_metrics: &[PairMetric],
    local_metrics: &[LocalComparisonMetric],
) -> Result<()> {
    ensure_tables(conn, tables)?;
    cleanup_reserved_range(conn, tables)?;

    let tx = conn.unchecked_transaction()?;
    let ts = (Utc::now().timestamp_micros() as i128).to_be_bytes();
    let mut allocator = UidAllocator::new(RESERVED_UID_START);

    let mut main_stmt = tx.prepare_cached(&format!(
        "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, ts = excluded.ts",
        tables.main
    ))?;
    let mut history_stmt = tx.prepare_cached(&format!(
        "INSERT INTO \"{}\" (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        tables.history
    ))?;

    let snapshot_uid = allocator.next();
    write_type_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        SNAPSHOT_TYPE,
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "generatedAt",
        &Value::String(Utc::now().to_rfc3339()),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "status",
        &Value::String("ready".to_string()),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "books",
        &Value::Array(
            books
                .iter()
                .map(|book| Value::String(book.clone()))
                .collect(),
        ),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "translationCodes",
        &Value::Array(
            translations
                .iter()
                .map(|translation| Value::String(translation.clone()))
                .collect(),
        ),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "thresholdMetric",
        &json!("sequence_similarity_pct"),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "verbatimRunFlagWords",
        &json!(VERBATIM_RUN_FLAG_WORDS),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "thresholdBufferPct",
        &json!(threshold_buffer_pct),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "baselineMeanPct",
        &json!(summary.baseline_mean_pct),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "thresholdPct",
        &json!(summary.threshold_pct),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "totalCopyrightedVerses",
        &json!(summary.total_copyrighted_verses),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "matchedLocalVerses",
        &json!(summary.matched_local_verses),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "unmatchedCopyrightedVerses",
        &json!(summary.unmatched_copyrighted_verses),
    )?;
    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "parseErrorCount",
        &json!(summary.parse_error_count),
    )?;

    let baseline_by_reference = group_baseline_metrics_by_reference(baseline_metrics);
    let local_by_reference_and_match = group_local_metrics(local_metrics);

    let mut verse_uids = Vec::new();
    let mut pair_summary_uids = Vec::new();
    let mut histogram_uids = Vec::new();

    for verse in copyrighted_verses {
        let baseline_uid = allocator.next();
        verse_uids.push(baseline_uid);
        write_type_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            COMPARISON_VERSE_TYPE,
        )?;
        write_edge_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            snapshot_uid,
            baseline_uid,
            "verses",
        )?;
        write_edge_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            snapshot_uid,
            "snapshot",
        )?;
        write_reference_fields(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            &verse.reference,
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            "matchOrdinal",
            &json!(0),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            "hasLocalMatch",
            &json!(false),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            "parseError",
            &json!(false),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            "parseErrorReason",
            &Value::Null,
        )?;

        let mut text_uids = Vec::new();
        for translation_code in translations {
            if let Some(text) = verse.texts_by_translation.get(translation_code) {
                let text_uid = allocator.next();
                text_uids.push(text_uid);
                write_type_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    text_uid,
                    COMPARISON_VERSE_TEXT_TYPE,
                )?;
                write_edge_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    baseline_uid,
                    text_uid,
                    "texts",
                )?;
                write_edge_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    text_uid,
                    baseline_uid,
                    "comparisonVerse",
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    text_uid,
                    "translationCode",
                    &json!(translation_code),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    text_uid,
                    "translationName",
                    &json!(translation_code),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    text_uid,
                    "sourceKind",
                    &json!("copyright"),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    text_uid,
                    "text",
                    &json!(text),
                )?;
            }
        }
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            "texts",
            &Value::Array(
                text_uids
                    .iter()
                    .map(|uid| Value::String(uid.to_string()))
                    .collect(),
            ),
        )?;

        let baseline_list = baseline_by_reference
            .get(&verse.reference)
            .cloned()
            .unwrap_or_default();
        let mut metric_uids = Vec::new();
        for metric in baseline_list {
            let metric_uid = allocator.next();
            metric_uids.push(metric_uid);
            write_metric_record(
                &mut main_stmt,
                &mut history_stmt,
                &ts,
                baseline_uid,
                metric_uid,
                "baseline",
                &metric.left_code,
                &metric.right_code,
                &metric.metrics,
                None,
                None,
            )?;
        }
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            baseline_uid,
            "metrics",
            &Value::Array(
                metric_uids
                    .iter()
                    .map(|uid| Value::String(uid.to_string()))
                    .collect(),
            ),
        )?;

        if let Some(local_groups) = local_by_reference_and_match.get(&verse.reference) {
            for ((verse_uid, match_ordinal), group) in local_groups {
                let local_verse_uid = allocator.next();
                verse_uids.push(local_verse_uid);
                write_type_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    COMPARISON_VERSE_TYPE,
                )?;
                write_edge_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    snapshot_uid,
                    local_verse_uid,
                    "verses",
                )?;
                write_edge_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    snapshot_uid,
                    "snapshot",
                )?;
                write_reference_fields(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    &verse.reference,
                )?;

                let exemplar = group
                    .first()
                    .ok_or_else(|| anyhow!("empty local comparison group"))?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "newGospelVerseNumber",
                    &json!(exemplar.local_match.verse_number),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "newGospelVerseUid",
                    &json!(verse_uid.to_string()),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "supportingReferenceDisplay",
                    &json!(exemplar.local_match.supporting_reference_display),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "matchOrdinal",
                    &json!(match_ordinal),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "hasLocalMatch",
                    &json!(true),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "parseError",
                    &json!(false),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "parseErrorReason",
                    &Value::Null,
                )?;

                let mut local_text_uids = Vec::new();
                let local_text_uid = allocator.next();
                local_text_uids.push(local_text_uid);
                write_type_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_text_uid,
                    COMPARISON_VERSE_TEXT_TYPE,
                )?;
                write_edge_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    local_text_uid,
                    "texts",
                )?;
                write_edge_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_text_uid,
                    local_verse_uid,
                    "comparisonVerse",
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_text_uid,
                    "translationCode",
                    &json!("LOCAL"),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_text_uid,
                    "translationName",
                    &json!("My Translation"),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_text_uid,
                    "sourceKind",
                    &json!("local"),
                )?;
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_text_uid,
                    "text",
                    &json!(exemplar.local_match.text),
                )?;

                for translation_code in translations {
                    if let Some(text) = verse.texts_by_translation.get(translation_code) {
                        let text_uid = allocator.next();
                        local_text_uids.push(text_uid);
                        write_type_record(
                            &mut main_stmt,
                            &mut history_stmt,
                            &ts,
                            text_uid,
                            COMPARISON_VERSE_TEXT_TYPE,
                        )?;
                        write_edge_record(
                            &mut main_stmt,
                            &mut history_stmt,
                            &ts,
                            local_verse_uid,
                            text_uid,
                            "texts",
                        )?;
                        write_edge_record(
                            &mut main_stmt,
                            &mut history_stmt,
                            &ts,
                            text_uid,
                            local_verse_uid,
                            "comparisonVerse",
                        )?;
                        write_field_record(
                            &mut main_stmt,
                            &mut history_stmt,
                            &ts,
                            text_uid,
                            "translationCode",
                            &json!(translation_code),
                        )?;
                        write_field_record(
                            &mut main_stmt,
                            &mut history_stmt,
                            &ts,
                            text_uid,
                            "translationName",
                            &json!(translation_code),
                        )?;
                        write_field_record(
                            &mut main_stmt,
                            &mut history_stmt,
                            &ts,
                            text_uid,
                            "sourceKind",
                            &json!("copyright"),
                        )?;
                        write_field_record(
                            &mut main_stmt,
                            &mut history_stmt,
                            &ts,
                            text_uid,
                            "text",
                            &json!(text),
                        )?;
                    }
                }
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "texts",
                    &Value::Array(
                        local_text_uids
                            .iter()
                            .map(|uid| Value::String(uid.to_string()))
                            .collect(),
                    ),
                )?;

                let mut local_metric_uids = Vec::new();
                for metric in group {
                    let metric_uid = allocator.next();
                    local_metric_uids.push(metric_uid);
                    write_metric_record(
                        &mut main_stmt,
                        &mut history_stmt,
                        &ts,
                        local_verse_uid,
                        metric_uid,
                        "local_vs_copyright",
                        "LOCAL",
                        &metric.right_code,
                        &metric.metrics,
                        Some(metric.baseline_delta_pct),
                        Some(metric.exceeds_threshold),
                    )?;
                }
                write_field_record(
                    &mut main_stmt,
                    &mut history_stmt,
                    &ts,
                    local_verse_uid,
                    "metrics",
                    &Value::Array(
                        local_metric_uids
                            .iter()
                            .map(|uid| Value::String(uid.to_string()))
                            .collect(),
                    ),
                )?;
            }
        }
    }

    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "verses",
        &Value::Array(
            verse_uids
                .iter()
                .map(|uid| Value::String(uid.to_string()))
                .collect(),
        ),
    )?;

    for summary_row in build_pair_summaries(baseline_metrics, local_metrics) {
        let uid = allocator.next();
        pair_summary_uids.push(uid);
        write_type_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            COMPARISON_PAIR_SUMMARY_TYPE,
        )?;
        write_edge_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            snapshot_uid,
            uid,
            "pairSummaries",
        )?;
        write_edge_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            snapshot_uid,
            "snapshot",
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "wordRunCounts",
            &json!(summary_row.word_run_counts),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "comparisonKind",
            &json!(summary_row.comparison_kind),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "leftTranslationCode",
            &json!(summary_row.left_code),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "rightTranslationCode",
            &json!(summary_row.right_code),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "averageSequenceSimilarityPct",
            &json!(summary_row.average_sequence_similarity_pct),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "averageNgramContainmentPct",
            &json!(summary_row.average_ngram_containment_pct),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "averageTokenOverlapPct",
            &json!(summary_row.average_token_overlap_pct),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "averageSemanticSimilarityPct",
            &json!(summary_row.average_semantic_similarity_pct),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "averageLongestCommonRunWords",
            &json!(summary_row.average_longest_common_run_words),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "minSequenceSimilarityPct",
            &json!(summary_row.min_sequence_similarity_pct),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "maxSequenceSimilarityPct",
            &json!(summary_row.max_sequence_similarity_pct),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "minNgramContainmentPct",
            &json!(summary_row.min_ngram_containment_pct),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "maxNgramContainmentPct",
            &json!(summary_row.max_ngram_containment_pct),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "verseCount",
            &json!(summary_row.verse_count),
        )?;
    }

    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "pairSummaries",
        &Value::Array(
            pair_summary_uids
                .iter()
                .map(|uid| Value::String(uid.to_string()))
                .collect(),
        ),
    )?;

    for bucket in build_histogram_buckets(local_metrics) {
        let uid = allocator.next();
        histogram_uids.push(uid);
        write_type_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            COMPARISON_HISTOGRAM_BUCKET_TYPE,
        )?;
        write_edge_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            snapshot_uid,
            uid,
            "histogramBuckets",
        )?;
        write_edge_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            snapshot_uid,
            "snapshot",
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "bucketStart",
            &json!(bucket.bucket_start),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "bucketEnd",
            &json!(bucket.bucket_end),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            uid,
            "count",
            &json!(bucket.count),
        )?;
    }

    write_field_record(
        &mut main_stmt,
        &mut history_stmt,
        &ts,
        snapshot_uid,
        "histogramBuckets",
        &Value::Array(
            histogram_uids
                .iter()
                .map(|uid| Value::String(uid.to_string()))
                .collect(),
        ),
    )?;

    drop(main_stmt);
    drop(history_stmt);
    tx.commit()?;

    if allocator.next_uid > RESERVED_UID_END {
        bail!("comparison snapshot exhausted reserved uid range");
    }

    Ok(())
}

fn write_reference_fields(
    main_stmt: &mut rusqlite::CachedStatement<'_>,
    history_stmt: &mut rusqlite::CachedStatement<'_>,
    ts: &[u8; 16],
    uid: u64,
    reference: &VerseRef,
) -> Result<()> {
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        uid,
        "bookCode",
        &json!(reference.book_code),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        uid,
        "bookName",
        &json!(reference.book_name),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        uid,
        "chapter",
        &json!(reference.chapter),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        uid,
        "verse",
        &json!(reference.verse),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        uid,
        "reference",
        &json!(reference.label()),
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn write_metric_record(
    main_stmt: &mut rusqlite::CachedStatement<'_>,
    history_stmt: &mut rusqlite::CachedStatement<'_>,
    ts: &[u8; 16],
    comparison_verse_uid: u64,
    metric_uid: u64,
    comparison_kind: &str,
    left_code: &str,
    right_code: &str,
    metrics: &MetricSet,
    baseline_delta_pct: Option<f32>,
    exceeds_threshold: Option<bool>,
) -> Result<()> {
    write_type_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        COMPARISON_METRIC_TYPE,
    )?;
    write_edge_record(
        main_stmt,
        history_stmt,
        ts,
        comparison_verse_uid,
        metric_uid,
        "metrics",
    )?;
    write_edge_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        comparison_verse_uid,
        "comparisonVerse",
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "comparisonKind",
        &json!(comparison_kind),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "leftTranslationCode",
        &json!(left_code),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "rightTranslationCode",
        &json!(right_code),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "sequenceSimilarityPct",
        &json!(metrics.sequence_similarity_pct),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "ngramContainmentPct",
        &json!(metrics.ngram_containment_pct),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "tokenOverlapPct",
        &json!(metrics.token_overlap_pct),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "semanticSimilarityPct",
        &json!(metrics.semantic_similarity_pct),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "longestCommonRunWords",
        &json!(metrics.longest_common_run_words),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "longestCommonRunText",
        &json!(metrics.longest_common_run_text),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "baselineDeltaPct",
        &baseline_delta_pct.map_or(Value::Null, |value| json!(value)),
    )?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        metric_uid,
        "exceedsThreshold",
        &exceeds_threshold.map_or(Value::Null, |value| json!(value)),
    )?;
    Ok(())
}

fn build_pair_summaries(
    baseline_metrics: &[PairMetric],
    local_metrics: &[LocalComparisonMetric],
) -> Vec<PairSummary> {
    let mut summaries = Vec::new();

    let mut baseline_groups: HashMap<(String, String), Vec<&MetricSet>> = HashMap::new();
    for metric in baseline_metrics {
        baseline_groups
            .entry((metric.left_code.clone(), metric.right_code.clone()))
            .or_default()
            .push(&metric.metrics);
    }

    for ((left, right), values) in baseline_groups {
        summaries.push(build_pair_summary("baseline", &left, &right, &values));
    }

    let mut local_groups: HashMap<(String, String), Vec<&MetricSet>> = HashMap::new();
    for metric in local_metrics {
        local_groups
            .entry(("LOCAL".to_string(), metric.right_code.clone()))
            .or_default()
            .push(&metric.metrics);
    }

    for ((left, right), values) in local_groups {
        summaries.push(build_pair_summary(
            "local_vs_copyright",
            &left,
            &right,
            &values,
        ));
    }

    summaries.sort_by(|left, right| {
        left.comparison_kind
            .cmp(&right.comparison_kind)
            .then_with(|| left.left_code.cmp(&right.left_code))
            .then_with(|| left.right_code.cmp(&right.right_code))
    });
    summaries
}

// Exclusive upper bounds for the first six bands. The final band is 30+.
fn word_run_counts(lengths: impl Iterator<Item = i64>) -> [i64; 7] {
    let mut counts = [0; 7];
    for length in lengths {
        let band = [4, 7, 10, 15, 20, 30].partition_point(|&limit| length >= limit);
        counts[band] += 1;
    }
    counts
}

fn build_pair_summary(
    comparison_kind: &str,
    left: &str,
    right: &str,
    values: &[&MetricSet],
) -> PairSummary {
    let verse_count = values.len() as i64;
    let sequence_values: Vec<f32> = values
        .iter()
        .map(|metric| metric.sequence_similarity_pct)
        .collect();
    let ngram_values: Vec<f32> = values
        .iter()
        .map(|metric| metric.ngram_containment_pct)
        .collect();
    let token_values: Vec<f32> = values
        .iter()
        .map(|metric| metric.token_overlap_pct)
        .collect();
    let semantic_values: Vec<f32> = values
        .iter()
        .map(|metric| metric.semantic_similarity_pct)
        .collect();
    let run_values: Vec<f32> = values
        .iter()
        .map(|metric| metric.longest_common_run_words as f32)
        .collect();

    PairSummary {
        word_run_counts: word_run_counts(
            values.iter().map(|metric| metric.longest_common_run_words),
        ),
        comparison_kind: comparison_kind.to_string(),
        left_code: left.to_string(),
        right_code: right.to_string(),
        average_sequence_similarity_pct: mean(sequence_values.iter().copied()),
        average_ngram_containment_pct: mean(ngram_values.iter().copied()),
        average_token_overlap_pct: mean(token_values.iter().copied()),
        average_semantic_similarity_pct: mean(semantic_values.iter().copied()),
        average_longest_common_run_words: mean(run_values.iter().copied()),
        min_sequence_similarity_pct: sequence_values
            .iter()
            .copied()
            .min_by(total_cmp_f32)
            .unwrap_or_default(),
        max_sequence_similarity_pct: sequence_values
            .iter()
            .copied()
            .max_by(total_cmp_f32)
            .unwrap_or_default(),
        min_ngram_containment_pct: ngram_values
            .iter()
            .copied()
            .min_by(total_cmp_f32)
            .unwrap_or_default(),
        max_ngram_containment_pct: ngram_values
            .iter()
            .copied()
            .max_by(total_cmp_f32)
            .unwrap_or_default(),
        verse_count,
    }
}

fn build_histogram_buckets(local_metrics: &[LocalComparisonMetric]) -> Vec<HistogramBucket> {
    let mut counts = [0_i64; 11];
    for metric in local_metrics {
        let rounded = metric
            .metrics
            .sequence_similarity_pct
            .floor()
            .clamp(0.0, 100.0) as usize;
        let bucket = (rounded / 10).min(10);
        counts[bucket] += 1;
    }

    counts
        .into_iter()
        .enumerate()
        .map(|(index, count)| HistogramBucket {
            bucket_start: (index as i64) * 10,
            bucket_end: if index == 10 {
                100
            } else {
                ((index as i64) * 10) + 9
            },
            count,
        })
        .collect()
}

fn group_baseline_metrics_by_reference(
    baseline_metrics: &[PairMetric],
) -> HashMap<VerseRef, Vec<PairMetric>> {
    let mut grouped = HashMap::new();
    for metric in baseline_metrics {
        grouped
            .entry(metric.reference.clone())
            .or_insert_with(Vec::new)
            .push(metric.clone());
    }
    grouped
}

fn group_local_metrics(
    local_metrics: &[LocalComparisonMetric],
) -> HashMap<VerseRef, HashMap<(u64, i64), Vec<LocalComparisonMetric>>> {
    let mut grouped = HashMap::new();
    for metric in local_metrics {
        grouped
            .entry(metric.reference.clone())
            .or_insert_with(HashMap::new)
            .entry((
                metric.local_match.verse_uid,
                metric.local_match.match_ordinal,
            ))
            .or_insert_with(Vec::new)
            .push(metric.clone());
    }
    grouped
}

fn write_run_logs(
    log_dir: &PathBuf,
    summary: &SnapshotSummary,
    books: &[String],
    translations: &[String],
    baseline_metrics: &[PairMetric],
    local_metrics: &[LocalComparisonMetric],
    parse_errors: &[ParseErrorEntry],
    unmatched_verses: &[UnmatchedVerseEntry],
) -> Result<()> {
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("creating log dir {}", log_dir.display()))?;

    let baseline_pair_summaries = build_pair_summaries(baseline_metrics, &[]);
    let local_pair_summaries = build_pair_summaries(&[], local_metrics);

    let results_path = log_dir.join("latest_results.json");
    let parse_errors_path = log_dir.join("latest_parse_errors.jsonl");
    let unmatched_path = log_dir.join("latest_unmatched.jsonl");

    let results_json = json!({
        "generatedAt": Utc::now().to_rfc3339(),
        "books": books,
        "translations": translations,
        "summary": {
            "baselineMeanPct": summary.baseline_mean_pct,
            "thresholdPct": summary.threshold_pct,
            "totalCopyrightedVerses": summary.total_copyrighted_verses,
            "matchedLocalVerses": summary.matched_local_verses,
            "unmatchedCopyrightedVerses": summary.unmatched_copyrighted_verses,
            "parseErrorCount": summary.parse_error_count,
        },
        "baselinePairSummaries": baseline_pair_summaries.iter().map(pair_summary_json).collect::<Vec<_>>(),
        "localPairSummaries": local_pair_summaries.iter().map(pair_summary_json).collect::<Vec<_>>(),
        "baselineComparisons": baseline_metrics.iter().map(baseline_metric_json).collect::<Vec<_>>(),
        "localComparisons": local_metrics.iter().map(local_metric_json).collect::<Vec<_>>(),
    });

    std::fs::write(&results_path, serde_json::to_vec_pretty(&results_json)?)
        .with_context(|| format!("writing {}", results_path.display()))?;

    let parse_error_lines = parse_errors
        .iter()
        .map(|entry| {
            serde_json::to_string(&json!({
                "verseUid": entry.verse_uid,
                "verseNumber": entry.verse_number,
                "display": entry.display,
                "reason": entry.reason,
            }))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    std::fs::write(&parse_errors_path, format!("{parse_error_lines}\n"))
        .with_context(|| format!("writing {}", parse_errors_path.display()))?;

    let unmatched_lines = unmatched_verses
        .iter()
        .map(|entry| {
            serde_json::to_string(&json!({
                "reference": entry.reference.label(),
                "bookCode": entry.reference.book_code,
                "chapter": entry.reference.chapter,
                "verse": entry.reference.verse,
                "textsByTranslation": entry.texts_by_translation,
            }))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    std::fs::write(&unmatched_path, format!("{unmatched_lines}\n"))
        .with_context(|| format!("writing {}", unmatched_path.display()))?;

    println!("Wrote results log: {}", results_path.display());
    println!("Wrote parse error log: {}", parse_errors_path.display());
    println!("Wrote unmatched verse log: {}", unmatched_path.display());

    Ok(())
}

fn pair_summary_json(summary: &PairSummary) -> Value {
    json!({
        "comparisonKind": summary.comparison_kind,
        "leftTranslationCode": summary.left_code,
        "rightTranslationCode": summary.right_code,
        "averageSequenceSimilarityPct": summary.average_sequence_similarity_pct,
        "averageNgramContainmentPct": summary.average_ngram_containment_pct,
        "averageTokenOverlapPct": summary.average_token_overlap_pct,
        "averageSemanticSimilarityPct": summary.average_semantic_similarity_pct,
        "averageLongestCommonRunWords": summary.average_longest_common_run_words,
        "minSequenceSimilarityPct": summary.min_sequence_similarity_pct,
        "maxSequenceSimilarityPct": summary.max_sequence_similarity_pct,
        "minNgramContainmentPct": summary.min_ngram_containment_pct,
        "maxNgramContainmentPct": summary.max_ngram_containment_pct,
        "verseCount": summary.verse_count,
        "wordRunCounts": summary.word_run_counts,
    })
}

fn baseline_metric_json(metric: &PairMetric) -> Value {
    json!({
        "reference": metric.reference.label(),
        "bookCode": metric.reference.book_code,
        "chapter": metric.reference.chapter,
        "verse": metric.reference.verse,
        "leftTranslationCode": metric.left_code,
        "rightTranslationCode": metric.right_code,
        "sequenceSimilarityPct": metric.metrics.sequence_similarity_pct,
        "ngramContainmentPct": metric.metrics.ngram_containment_pct,
        "tokenOverlapPct": metric.metrics.token_overlap_pct,
        "semanticSimilarityPct": metric.metrics.semantic_similarity_pct,
        "longestCommonRunWords": metric.metrics.longest_common_run_words,
        "longestCommonRunText": metric.metrics.longest_common_run_text,
    })
}

fn local_metric_json(metric: &LocalComparisonMetric) -> Value {
    json!({
        "reference": metric.reference.label(),
        "bookCode": metric.reference.book_code,
        "chapter": metric.reference.chapter,
        "verse": metric.reference.verse,
        "newGospelVerseUid": metric.local_match.verse_uid,
        "newGospelVerseNumber": metric.local_match.verse_number,
        "supportingReferenceDisplay": metric.local_match.supporting_reference_display,
        "rightTranslationCode": metric.right_code,
        "sequenceSimilarityPct": metric.metrics.sequence_similarity_pct,
        "ngramContainmentPct": metric.metrics.ngram_containment_pct,
        "tokenOverlapPct": metric.metrics.token_overlap_pct,
        "semanticSimilarityPct": metric.metrics.semantic_similarity_pct,
        "longestCommonRunWords": metric.metrics.longest_common_run_words,
        "longestCommonRunText": metric.metrics.longest_common_run_text,
        "baselineDeltaPct": metric.baseline_delta_pct,
        "exceedsThreshold": metric.exceeds_threshold,
    })
}

fn strip_annotations(text: &str) -> String {
    ANNOTATION_RE.replace_all(text, "").trim().to_string()
}

fn normalize_text(text: &str) -> NormalizedText {
    let mut normalized = String::new();

    for ch in text.nfkc().flat_map(|part| part.to_lowercase()) {
        if ch.is_alphanumeric() {
            normalized.push(ch);
        } else if ch.is_whitespace() {
            normalized.push(' ');
        } else {
            normalized.push(' ');
        }
    }

    let text = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let tokens = if text.is_empty() {
        Vec::new()
    } else {
        text.split_whitespace().map(ToOwned::to_owned).collect()
    };

    NormalizedText { text, tokens }
}

fn levenshtein_similarity(left: &[String], right: &[String]) -> f32 {
    let max_len = left.len().max(right.len());
    if max_len == 0 {
        return 1.0;
    }

    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];

    for (i, left_token) in left.iter().enumerate() {
        current[0] = i + 1;
        for (j, right_token) in right.iter().enumerate() {
            let cost = usize::from(left_token != right_token);
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + cost);
        }
        previous.clone_from_slice(&current);
    }

    1.0 - (previous[right.len()] as f32 / max_len as f32)
}

fn jaccard_similarity(left: &[String], right: &[String]) -> f32 {
    let left_set: HashSet<&str> = left.iter().map(String::as_str).collect();
    let right_set: HashSet<&str> = right.iter().map(String::as_str).collect();

    if left_set.is_empty() && right_set.is_empty() {
        return 1.0;
    }

    let intersection = left_set.intersection(&right_set).count() as f32;
    let union = left_set.union(&right_set).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn longest_common_run(left: &[String], right: &[String]) -> (usize, String) {
    if left.is_empty() || right.is_empty() {
        return (0, String::new());
    }

    let mut matrix = vec![vec![0_usize; right.len() + 1]; left.len() + 1];
    let mut best_length = 0_usize;
    let mut best_end = 0_usize;

    for (left_index, left_token) in left.iter().enumerate() {
        for (right_index, right_token) in right.iter().enumerate() {
            if left_token == right_token {
                let length = matrix[left_index][right_index] + 1;
                matrix[left_index + 1][right_index + 1] = length;
                if length > best_length {
                    best_length = length;
                    best_end = left_index + 1;
                }
            }
        }
    }

    if best_length == 0 {
        return (0, String::new());
    }

    let start = best_end - best_length;
    (best_length, left[start..best_end].join(" "))
}

fn ngram_containment(source: &[String], reference: &[String], n: usize) -> f32 {
    if source.len() < n {
        return if source == reference { 1.0 } else { 0.0 };
    }

    let ref_ngrams: HashSet<Vec<&str>> = reference
        .windows(n)
        .map(|w| w.iter().map(String::as_str).collect())
        .collect();

    let source_ngram_count = source.windows(n).count();

    let matched = source
        .windows(n)
        .filter(|w| ref_ngrams.contains(&w.iter().map(String::as_str).collect::<Vec<_>>()))
        .count();

    matched as f32 / source_ngram_count as f32
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }

    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;

    for (left_value, right_value) in left.iter().zip(right.iter()) {
        dot += left_value * right_value;
        left_norm += left_value * left_value;
        right_norm += right_value * right_value;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }

    dot / (left_norm.sqrt() * right_norm.sqrt())
}

fn parse_reference_display(display: &str) -> Vec<VerseRef> {
    let mut results = Vec::new();
    let mut current_book: Option<String> = None;
    let mut current_chapter: Option<i64> = None;

    for segment in display
        .split(',')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
    {
        if let Some(parsed) =
            parse_reference_segment(segment, current_book.as_deref(), current_chapter)
        {
            current_book = Some(parsed.book_name.clone());
            current_chapter = Some(parsed.chapter);
            results.push(parsed);
        }
    }

    results
}

fn parse_reference_segment(
    segment: &str,
    current_book: Option<&str>,
    current_chapter: Option<i64>,
) -> Option<VerseRef> {
    let cleaned = segment.trim();
    if cleaned.is_empty() {
        return None;
    }

    if let Some(captures) = FULL_REFERENCE_RE.captures(cleaned) {
        let book_name = captures.name("book")?.as_str().trim().to_string();
        let book_code = to_book_code(&book_name)?;
        let chapter = captures.name("chapter")?.as_str().parse::<i64>().ok()?;
        let verse = captures.name("verse")?.as_str().parse::<i64>().ok()?;

        return Some(VerseRef {
            book_code,
            book_name,
            chapter,
            verse,
        });
    }

    if let Some(captures) = CHAPTER_VERSE_RE.captures(cleaned) {
        let chapter = captures.name("chapter")?.as_str().parse::<i64>().ok()?;
        let verse = captures.name("verse")?.as_str().parse::<i64>().ok()?;
        let book_name = current_book?.to_string();
        let book_code = to_book_code(&book_name)?;

        return Some(VerseRef {
            book_code,
            book_name,
            chapter,
            verse,
        });
    }

    if let Some(captures) = VERSE_ONLY_RE.captures(cleaned) {
        let verse = captures.name("verse")?.as_str().parse::<i64>().ok()?;
        let book_name = current_book?.to_string();
        let book_code = to_book_code(&book_name)?;

        return Some(VerseRef {
            book_code,
            book_name,
            chapter: current_chapter?,
            verse,
        });
    }

    None
}

fn to_book_code(book_name: &str) -> Option<String> {
    match normalize_book_name(book_name).as_str() {
        "matthew" | "matt" | "mat" => Some("Matthew".to_string()),
        "mark" | "mrk" => Some("Mark".to_string()),
        "luke" | "luk" => Some("Luke".to_string()),
        "john" | "jhn" => Some("John".to_string()),
        "thomas" | "thom" | "gospel of thomas" => Some("Thomas".to_string()),
        _ => None,
    }
}

fn normalize_book_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric() || ch.is_whitespace())
        .collect::<String>()
        .trim()
        .to_ascii_lowercase()
}

fn normalize_comparison_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn compare_reference(left: &VerseRef, right: &VerseRef) -> Ordering {
    left.book_name
        .cmp(&right.book_name)
        .then_with(|| left.chapter.cmp(&right.chapter))
        .then_with(|| left.verse.cmp(&right.verse))
}

fn required_string_field(fields: &HashMap<String, Value>, field: &str, uid: u64) -> Result<String> {
    fields
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("missing string field '{field}' for uid {uid}"))
}

fn required_i64_field(fields: &HashMap<String, Value>, field: &str, uid: u64) -> Result<i64> {
    fields
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow!("missing int field '{field}' for uid {uid}"))
}

fn parse_uid_field_optional(fields: &HashMap<String, Value>, field: &str) -> Option<u64> {
    fields
        .get(field)
        .and_then(|value| parse_uid_value(value).ok())
}

fn parse_uid_value(value: &Value) -> Result<u64> {
    if let Some(uid) = value.as_u64() {
        return Ok(uid);
    }

    if let Some(uid) = value.as_i64() {
        if uid >= 0 {
            return Ok(uid as u64);
        }
    }

    if let Some(uid) = value.as_str() {
        let trimmed = uid.trim();
        if !trimmed.is_empty() {
            return trimmed
                .parse::<u64>()
                .with_context(|| format!("invalid uid value '{trimmed}'"));
        }
    }

    bail!("invalid uid value '{value}'")
}

fn ensure_tables(conn: &Connection, tables: &DbTables) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS \"sys\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS \"quarantine\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS \"sys_metrics\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS \"sys_traces\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS \"{main}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL, ts BLOB NOT NULL) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS \"{history}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;",
        main = tables.main,
        history = tables.history,
    ))?;
    Ok(())
}

fn cleanup_reserved_range(conn: &Connection, tables: &DbTables) -> Result<()> {
    let mut select_stmt = conn.prepare(&format!("SELECT key FROM \"{}\"", tables.main))?;
    let keys = select_stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    let mut to_delete = Vec::new();

    for key in keys {
        let key = key?;
        if key_references_reserved_uid(&key) {
            to_delete.push(key);
        }
    }

    let mut delete_stmt =
        conn.prepare_cached(&format!("DELETE FROM \"{}\" WHERE key = ?1", tables.main))?;
    for key in to_delete {
        delete_stmt.execute(params![key])?;
    }

    Ok(())
}

fn key_references_reserved_uid(key: &[u8]) -> bool {
    match key.first().copied() {
        Some(0x01) if key.len() >= 9 => uid_in_reserved_range(u64::from_be_bytes(
            key[1..9].try_into().expect("uid bytes checked"),
        )),
        Some(0x02) if key.len() >= 10 => uid_in_reserved_range(u64::from_be_bytes(
            key[key.len() - 8..].try_into().expect("uid bytes checked"),
        )),
        Some(0x03) if key.len() >= 18 => {
            let target = u64::from_be_bytes(key[1..9].try_into().expect("uid bytes checked"));
            let source =
                u64::from_be_bytes(key[key.len() - 8..].try_into().expect("uid bytes checked"));
            uid_in_reserved_range(target) || uid_in_reserved_range(source)
        }
        Some(0x04) if key.len() >= 17 => {
            let source = u64::from_be_bytes(key[1..9].try_into().expect("uid bytes checked"));
            let target = u64::from_be_bytes(key[9..17].try_into().expect("uid bytes checked"));
            uid_in_reserved_range(source) || uid_in_reserved_range(target)
        }
        _ => false,
    }
}

fn uid_in_reserved_range(uid: u64) -> bool {
    (RESERVED_UID_START..=RESERVED_UID_END).contains(&uid)
}

fn write_type_record(
    main_stmt: &mut rusqlite::CachedStatement<'_>,
    history_stmt: &mut rusqlite::CachedStatement<'_>,
    ts: &[u8; 16],
    uid: u64,
    type_name: &str,
) -> Result<()> {
    let type_idx_key = encode_type_index_key(type_name, uid);
    main_stmt.execute(params![type_idx_key, &[] as &[u8], &ts[..]])?;
    write_field_record(
        main_stmt,
        history_stmt,
        ts,
        uid,
        "_type",
        &Value::String(type_name.to_string()),
    )
}

fn write_field_record(
    main_stmt: &mut rusqlite::CachedStatement<'_>,
    history_stmt: &mut rusqlite::CachedStatement<'_>,
    ts: &[u8; 16],
    uid: u64,
    field: &str,
    value: &Value,
) -> Result<()> {
    let raw = serde_json::to_vec(value)?;
    let mut value_with_ts = Vec::with_capacity(16 + raw.len());
    value_with_ts.extend_from_slice(ts);
    value_with_ts.extend_from_slice(&raw);

    main_stmt.execute(params![encode_data_key(uid, field), value_with_ts, &ts[..]])?;
    history_stmt.execute(params![encode_history_key(ts, uid, field), raw])?;
    Ok(())
}

fn write_edge_record(
    main_stmt: &mut rusqlite::CachedStatement<'_>,
    history_stmt: &mut rusqlite::CachedStatement<'_>,
    ts: &[u8; 16],
    source_uid: u64,
    target_uid: u64,
    field: &str,
) -> Result<()> {
    main_stmt.execute(params![
        encode_edge_key(target_uid, field, source_uid),
        &[] as &[u8],
        &ts[..]
    ])?;
    main_stmt.execute(params![
        encode_reverse_edge_key(source_uid, target_uid, field),
        &[] as &[u8],
        &ts[..]
    ])?;
    history_stmt.execute(params![
        encode_history_key(ts, source_uid, field),
        &[] as &[u8]
    ])?;
    Ok(())
}

fn encode_data_key(uid: u64, predicate: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 8 + predicate.len());
    out.push(0x01);
    out.extend_from_slice(&uid.to_be_bytes());
    out.extend_from_slice(predicate.as_bytes());
    out
}

fn encode_type_index_key(type_name: &str, uid: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + type_name.len() + 1 + 8);
    out.push(0x03);
    out.extend_from_slice(type_name.as_bytes());
    out.push(0x00);
    out.extend_from_slice(&uid.to_be_bytes());
    out
}

fn encode_history_key(timestamp: &[u8; 16], uid: u64, predicate: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + 8 + predicate.len());
    out.extend_from_slice(timestamp);
    out.extend_from_slice(&uid.to_be_bytes());
    out.extend_from_slice(predicate.as_bytes());
    out
}

fn encode_edge_key(target_uid: u64, field: &str, source_uid: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 8 + field.len() + 1 + 8);
    out.push(0x07);
    out.extend_from_slice(&target_uid.to_be_bytes());
    out.extend_from_slice(field.as_bytes());
    out.push(0x00);
    out.extend_from_slice(&source_uid.to_be_bytes());
    out
}

fn encode_reverse_edge_key(source_uid: u64, target_uid: u64, field: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 8 + 8 + field.len());
    out.push(0x08);
    out.extend_from_slice(&source_uid.to_be_bytes());
    out.extend_from_slice(&target_uid.to_be_bytes());
    out.extend_from_slice(field.as_bytes());
    out
}

fn mean<I>(values: I) -> f32
where
    I: IntoIterator<Item = f32>,
{
    let mut total = 0.0_f32;
    let mut count = 0_u32;

    for value in values {
        total += value;
        count += 1;
    }

    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

fn total_cmp_f32(left: &f32, right: &f32) -> Ordering {
    left.total_cmp(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_comparison_reports() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let summary = SnapshotSummary {
            baseline_mean_pct: 20.0,
            threshold_pct: 25.0,
            total_copyrighted_verses: 1,
            matched_local_verses: 0,
            unmatched_copyrighted_verses: 1,
            parse_error_count: 1,
        };
        let error = ParseErrorEntry {
            verse_uid: 10,
            verse_number: 1,
            display: "unknown".into(),
            reason: "unknown book".into(),
        };
        let unmatched = UnmatchedVerseEntry {
            reference: VerseRef {
                book_code: "MAT".into(),
                book_name: "Matthew".into(),
                chapter: 1,
                verse: 2,
            },
            texts_by_translation: HashMap::from([("TEST".into(), "Example verse".into())]),
        };
        write_run_logs(
            &dir.path().to_owned(),
            &summary,
            &["MAT".into()],
            &["TEST".into()],
            &[],
            &[],
            &[error],
            &[unmatched],
        )?;
        let read = |name: &str| -> Result<Value> {
            Ok(serde_json::from_slice(&std::fs::read(
                dir.path().join(name),
            )?)?)
        };
        assert_eq!(
            read("latest_results.json")?["summary"]["thresholdPct"],
            25.0
        );
        assert_eq!(read("latest_parse_errors.jsonl")?["display"], "unknown");
        assert_eq!(
            read("latest_unmatched.jsonl")?["textsByTranslation"]["TEST"],
            "Example verse"
        );
        Ok(())
    }

    #[test]
    fn word_run_bands_cover_boundaries_once() {
        let lengths = [0, 3, 4, 6, 7, 9, 10, 14, 15, 19, 20, 29, 30, 41];
        for (index, length) in lengths.iter().enumerate() {
            let counts = word_run_counts(std::iter::once(*length));
            assert_eq!(counts[index / 2], 1);
            assert_eq!(counts.iter().sum::<i64>(), 1);
        }
        assert_eq!(word_run_counts(lengths.iter().copied()), [2; 7]);
    }

    #[test]
    fn normalize_text_collapses_case_punctuation_and_spacing() {
        let normalized = normalize_text("In the beginning,  WAS\tThe Word.");
        assert_eq!(normalized.text, "in the beginning was the word");
        assert_eq!(
            normalized.tokens,
            vec!["in", "the", "beginning", "was", "the", "word"]
        );
    }

    #[test]
    fn longest_common_run_finds_consecutive_words() {
        let left = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let right = vec!["x".into(), "b".into(), "c".into(), "y".into()];
        let (count, text) = longest_common_run(&left, &right);
        assert_eq!(count, 2);
        assert_eq!(text, "b c");
    }

    #[test]
    fn levenshtein_similarity_is_one_for_identical_tokens() {
        let tokens = vec!["one".into(), "two".into()];
        assert_eq!(levenshtein_similarity(&tokens, &tokens), 1.0);
    }

    #[test]
    fn jaccard_similarity_handles_overlap() {
        let left = vec!["one".into(), "two".into()];
        let right = vec!["two".into(), "three".into()];
        assert!((jaccard_similarity(&left, &right) - (1.0 / 3.0)).abs() < 0.0001);
    }

    #[test]
    fn parse_reference_display_handles_composites_and_blank_segments() {
        let parsed = parse_reference_display("Luke 14:35,  Matthew 5:13, , 15");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].book_code, "Luke");
        assert_eq!(parsed[1].book_code, "Matthew");
        assert_eq!(parsed[2].chapter, 5);
        assert_eq!(parsed[2].verse, 15);
    }

    #[test]
    fn parse_reference_display_handles_matt_abbreviation() {
        let parsed = parse_reference_display("Matt 1:1, 2:5");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].book_code, "Matthew");
        assert_eq!(parsed[0].chapter, 1);
        assert_eq!(parsed[0].verse, 1);
        assert_eq!(parsed[1].book_code, "Matthew");
        assert_eq!(parsed[1].chapter, 2);
        assert_eq!(parsed[1].verse, 5);
    }

    #[test]
    fn parse_reference_display_supports_gospel_of_thomas_alias() {
        let parsed = parse_reference_display("Gospel of Thomas 1:2, 3");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].book_code, "Thomas");
        assert_eq!(parsed[0].chapter, 1);
        assert_eq!(parsed[0].verse, 2);
        assert_eq!(parsed[1].book_code, "Thomas");
        assert_eq!(parsed[1].chapter, 1);
        assert_eq!(parsed[1].verse, 3);
    }

    #[test]
    fn resolve_selected_translations_matches_by_human_name() {
        let translations = HashMap::from([(
            1_u64,
            TranslationRecord {
                uid: 1,
                code: "mark-m-mattison".into(),
                name: "Mark M. Mattison".into(),
            },
        )]);

        let selected =
            resolve_selected_translations(&translations, &[String::from("Mark M. Mattison")], true)
                .expect("translation name should match");

        assert_eq!(selected.len(), 1);
        assert!(
            selected
                .values()
                .any(|entry| entry.code == "mark-m-mattison")
        );
    }

    #[test]
    fn resolve_effective_translation_requests_adds_mattison_for_thomas() {
        let books = vec!["Gospel of Thomas".to_string()];
        let translations: Vec<String> = DEFAULT_TRANSLATIONS
            .iter()
            .map(|value| (*value).to_string())
            .collect();

        let effective = resolve_effective_translation_requests(&books, &translations);

        let mut expected = translations;
        expected.push(THOMAS_TRANSLATION_CODE.to_string());
        assert_eq!(effective, expected);
    }

    #[test]
    fn histogram_buckets_cover_upper_bound() {
        let metrics = vec![LocalComparisonMetric {
            reference: VerseRef {
                book_code: "LUK".into(),
                book_name: "Luke".into(),
                chapter: 1,
                verse: 1,
            },
            local_match: LocalMatch {
                verse_uid: 1,
                verse_number: 1,
                text: "text".into(),
                supporting_reference_display: "Luke 1:1".into(),
                match_ordinal: 1,
            },
            right_code: "NIV".into(),
            metrics: MetricSet {
                sequence_similarity_pct: 100.0,
                ngram_containment_pct: 100.0,
                token_overlap_pct: 100.0,
                semantic_similarity_pct: 100.0,
                longest_common_run_words: 3,
                longest_common_run_text: "a b c".into(),
            },
            baseline_delta_pct: 10.0,
            exceeds_threshold: true,
        }];

        let buckets = build_histogram_buckets(&metrics);
        assert_eq!(buckets[10].count, 1);
    }
}
