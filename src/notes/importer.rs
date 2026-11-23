use anyhow::{Context, Result};
use chrono::Utc;
use indicatif::{ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use regex::Regex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use varda_client_rs::{BulkRecord, GraphqlBuilder, VardaClient};
use walkdir::WalkDir;

static VERSE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b((?:[1-3]\s+)?[A-Z][a-z]+(?:\s+[A-Z][a-z]+)*)\s+(\d+):(\d+)(?:[–\-](\d+))?")
        .expect("valid verse regex")
});
static OBSIDIAN_IMAGE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"!\[\[([^\]\|#]+)(?:\|[^\]]*)?\]\]").expect("valid obsidian image regex")
});
static MARKDOWN_IMAGE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"!\[[^\]]*\]\(([^)]+)\)").expect("valid markdown image regex"));
static MARKDOWN_LINK_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\[[^\]]*\]\((https?://[^)\s]+)\)").expect("valid markdown link regex")
});
static RAW_URL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"https?://[^\s<>"')\]]+"#).expect("valid raw url regex"));

const SEMANTIC_CANDIDATE_THRESHOLD: f32 = 0.8;
const MIN_SHARED_VERSES_FOR_CONNECTION: usize = 2;
const UNCOMMON_VERSE_MAX_DOC_FREQ: usize = 3;
const UNCOMMON_VERSE_MAX_CORPUS_PERCENT: usize = 2;
const SECOND_PASS_CANDIDATE_LIMIT: usize = 12;

const BOOK_ALIASES: &[(&str, &str)] = &[
    ("gen", "Genesis"),
    ("genesis", "Genesis"),
    ("exod", "Exodus"),
    ("exo", "Exodus"),
    ("exodus", "Exodus"),
    ("lev", "Leviticus"),
    ("leviticus", "Leviticus"),
    ("num", "Numbers"),
    ("numbers", "Numbers"),
    ("deut", "Deuteronomy"),
    ("deuteronomy", "Deuteronomy"),
    ("josh", "Joshua"),
    ("joshua", "Joshua"),
    ("judg", "Judges"),
    ("judges", "Judges"),
    ("ruth", "Ruth"),
    ("1 sam", "1 Samuel"),
    ("1 samuel", "1 Samuel"),
    ("2 sam", "2 Samuel"),
    ("2 samuel", "2 Samuel"),
    ("1 kgs", "1 Kings"),
    ("1 kings", "1 Kings"),
    ("2 kgs", "2 Kings"),
    ("2 kings", "2 Kings"),
    ("1 chr", "1 Chronicles"),
    ("1 chronicles", "1 Chronicles"),
    ("2 chr", "2 Chronicles"),
    ("2 chronicles", "2 Chronicles"),
    ("ezra", "Ezra"),
    ("neh", "Nehemiah"),
    ("nehemiah", "Nehemiah"),
    ("esth", "Esther"),
    ("esther", "Esther"),
    ("job", "Job"),
    ("ps", "Psalms"),
    ("psalm", "Psalms"),
    ("psalms", "Psalms"),
    ("prov", "Proverbs"),
    ("proverbs", "Proverbs"),
    ("eccl", "Ecclesiastes"),
    ("ecclesiastes", "Ecclesiastes"),
    ("song", "Song of Solomon"),
    ("song of songs", "Song of Solomon"),
    ("song of solomon", "Song of Solomon"),
    ("isa", "Isaiah"),
    ("isaiah", "Isaiah"),
    ("jer", "Jeremiah"),
    ("jeremiah", "Jeremiah"),
    ("lam", "Lamentations"),
    ("lamentations", "Lamentations"),
    ("ezek", "Ezekiel"),
    ("ezekiel", "Ezekiel"),
    ("dan", "Daniel"),
    ("daniel", "Daniel"),
    ("hos", "Hosea"),
    ("hosea", "Hosea"),
    ("joel", "Joel"),
    ("amos", "Amos"),
    ("obad", "Obadiah"),
    ("obadiah", "Obadiah"),
    ("jonah", "Jonah"),
    ("mic", "Micah"),
    ("micah", "Micah"),
    ("nah", "Nahum"),
    ("nahum", "Nahum"),
    ("hab", "Habakkuk"),
    ("habakkuk", "Habakkuk"),
    ("zeph", "Zephaniah"),
    ("zephaniah", "Zephaniah"),
    ("hag", "Haggai"),
    ("haggai", "Haggai"),
    ("zech", "Zechariah"),
    ("zechariah", "Zechariah"),
    ("mal", "Malachi"),
    ("malachi", "Malachi"),
    ("matt", "Matthew"),
    ("matthew", "Matthew"),
    ("mark", "Mark"),
    ("luke", "Luke"),
    ("john", "John"),
    ("acts", "Acts"),
    ("rom", "Romans"),
    ("romans", "Romans"),
    ("1 cor", "1 Corinthians"),
    ("1 corinthians", "1 Corinthians"),
    ("2 cor", "2 Corinthians"),
    ("2 corinthians", "2 Corinthians"),
    ("gal", "Galatians"),
    ("galatians", "Galatians"),
    ("eph", "Ephesians"),
    ("ephesians", "Ephesians"),
    ("phil", "Philippians"),
    ("philippians", "Philippians"),
    ("col", "Colossians"),
    ("colossians", "Colossians"),
    ("1 thess", "1 Thessalonians"),
    ("1 thessalonians", "1 Thessalonians"),
    ("2 thess", "2 Thessalonians"),
    ("2 thessalonians", "2 Thessalonians"),
    ("1 tim", "1 Timothy"),
    ("1 timothy", "1 Timothy"),
    ("2 tim", "2 Timothy"),
    ("2 timothy", "2 Timothy"),
    ("titus", "Titus"),
    ("phlm", "Philemon"),
    ("philemon", "Philemon"),
    ("heb", "Hebrews"),
    ("hebrews", "Hebrews"),
    ("jas", "James"),
    ("james", "James"),
    ("1 pet", "1 Peter"),
    ("1 peter", "1 Peter"),
    ("2 pet", "2 Peter"),
    ("2 peter", "2 Peter"),
    ("1 john", "1 John"),
    ("2 john", "2 John"),
    ("3 john", "3 John"),
    ("jude", "Jude"),
    ("rev", "Revelation"),
    ("revelation", "Revelation"),
];

#[derive(Debug, Deserialize)]
pub struct ImportConfig {
    pub server: ServerConfig,
    pub vault: VaultConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub url: String,
    pub model: String,
    pub model_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VaultConfig {
    pub db: String,
    pub llm_model: String,
    pub path: String,
    pub attachment_root: String,
    #[serde(default)]
    pub exclude_folders: Vec<String>,
}

#[derive(Clone)]
struct NoteVerseRecord {
    gospel: String,
    reference: String,
    full_ref: String,
}

#[derive(Clone)]
struct NoteVideoRecord {
    url: String,
    video_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct IndexEntry {
    uid: String,
    title: String,
    summary: String,
    tags: Vec<String>,
    verses: Vec<String>,
    vector: Vec<f32>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct ImportCheckpoint {
    vault_root: String,
    completed_files: HashMap<String, IndexEntry>,
    second_pass_completed: bool,
    second_pass_processed_uids: HashSet<String>,
}

pub async fn run(config_path: &Path, vault_override: Option<&Path>) -> Result<()> {
    let config = load_config(config_path)?;
    let vault_root = match vault_override {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(&config.vault.path),
    };
    let vault_root = fs::canonicalize(&vault_root)
        .with_context(|| format!("resolving vault path {}", vault_root.display()))?;
    let attachment_root = fs::canonicalize(&config.vault.attachment_root)
        .with_context(|| format!("resolving attachment root {}", config.vault.attachment_root))?;
    let excluded_dirs = canonicalize_excluded_dirs(&vault_root, &config.vault.exclude_folders);
    let files = markdown_files(&vault_root, &excluded_dirs)?;
    let checkpoint_path = checkpoint_path(config_path);
    let mut checkpoint = load_checkpoint(&checkpoint_path, &vault_root)?;
    let graphql_url = format!("http://localhost:{}/graphql", config.server.port);
    let llm = importer_llm_config(&config);

    let client = VardaClient::new(graphql_url.clone()).with_database(&config.vault.db);
    let _bulk_target = Url::parse(&graphql_url).context("invalid derived graphql url")?;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .context("failed to build HTTP client")?;
    let varda_base_url = graphql_url.trim_end_matches("/graphql").to_string();

    load_model_if_needed(&http, &llm).await?;

    let pb = ProgressBar::new(files.len() as u64);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {pos}/{len} {msg}")
            .expect("valid progress template"),
    );

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = 0usize;
    let mut session_index: Vec<IndexEntry> = files
        .iter()
        .filter_map(|file_path| {
            let rel_path = file_path
                .strip_prefix(&vault_root)
                .unwrap_or(file_path)
                .to_string_lossy()
                .replace('\\', "/");
            checkpoint.completed_files.get(&rel_path).cloned()
        })
        .collect();
    let mut relationship_keys: HashSet<(String, String, String)> = HashSet::new();

    let import_result = async {
        for file_path in files {
            let rel_path = file_path
                .strip_prefix(&vault_root)
                .unwrap_or(&file_path)
                .display()
                .to_string()
                .replace('\\', "/");
            pb.set_message(rel_path.clone());

            if checkpoint.completed_files.contains_key(&rel_path) {
                skipped += 1;
                pb.println(format!("[ ↺ ] {} — checkpoint", rel_path));
                pb.inc(1);
                continue;
            }

            match process_file(
                &client,
                &http,
                &vault_root,
                &attachment_root,
                &varda_base_url,
                &file_path,
                &config.vault.db,
                &llm,
                &mut session_index,
                &mut relationship_keys,
            )
            .await
            {
                Ok(ProcessOutcome::Imported(index_entry)) => {
                    imported += 1;
                    session_index.push(index_entry.clone());
                    checkpoint
                        .completed_files
                        .insert(rel_path.clone(), index_entry);
                    save_checkpoint(&checkpoint_path, &checkpoint)?;
                    pb.println(format!("[ ✓ ] {} — done", rel_path));
                }
                Ok(ProcessOutcome::SkippedDuplicate(index_entry)) => {
                    skipped += 1;
                    session_index.push(index_entry.clone());
                    checkpoint
                        .completed_files
                        .insert(rel_path.clone(), index_entry);
                    save_checkpoint(&checkpoint_path, &checkpoint)?;
                    pb.println(format!("[ ! ] {} — skipped (duplicate title)", rel_path));
                }
                Err(error) => {
                    errors += 1;
                    pb.println(format!("[ x ] {} — {}", rel_path, error));
                }
            }
            pb.inc(1);
        }
        if !checkpoint.second_pass_completed {
            run_second_pass_relationships(
                &client,
                &config.vault.db,
                &http,
                &llm,
                &session_index,
                &mut relationship_keys,
                &pb,
                &checkpoint_path,
                &mut checkpoint,
            )
            .await?;
        }
        Result::<(), anyhow::Error>::Ok(())
    }
    .await;

    let unload_result = unload_model_if_needed(&http, &llm).await;

    pb.finish_and_clear();
    import_result?;
    unload_result?;
    println!(
        "✅ Import complete. {} imported, {} skipped, {} errors",
        imported, skipped, errors
    );
    Ok(())
}

enum ProcessOutcome {
    Imported(IndexEntry),
    SkippedDuplicate(IndexEntry),
}

async fn process_file(
    client: &VardaClient,
    http: &reqwest::Client,
    vault_root: &Path,
    attachment_root: &Path,
    varda_base_url: &str,
    file_path: &Path,
    db_name: &str,
    llm: &LlmConfig,
    session_index: &mut Vec<IndexEntry>,
    relationship_keys: &mut HashSet<(String, String, String)>,
) -> Result<ProcessOutcome> {
    let content = read_markdown_file_readonly(file_path)?;
    let title = derive_title(file_path, &content);

    let category_name = category_name_for_path(vault_root, file_path);
    if let Some(existing_entry) =
        fetch_existing_note_index_entry(client, vault_root, file_path, &title, &category_name)
            .await?
    {
        return Ok(ProcessOutcome::SkippedDuplicate(existing_entry));
    }

    let tags = generate_tags(vault_root, file_path);
    let category_id = ensure_note_category(client, category_name).await?;
    let verses = extract_verses(&content);
    let urls = extract_urls(&content);
    let note_videos = extract_note_videos(&urls);
    let images = import_images(
        http,
        varda_base_url,
        vault_root,
        attachment_root,
        file_path,
        &content,
    )
    .await?;
    let summary = generate_summary(http, llm, &title, &content)
        .await
        .unwrap_or_default();
    let vector = generate_embedding(http, llm, &title, &summary)
        .await
        .unwrap_or_default();

    let note_id = Uuid::new_v4().to_string();
    let timestamp = Utc::now().to_rfc3339();

    let note_uid = create_note(
        client,
        db_name,
        note_record(
            &note_id,
            &title,
            &content,
            &urls,
            &tags,
            &category_id,
            &images,
            &summary,
            &vector,
            &timestamp,
        ),
    )
    .await?;
    for verse in &verses {
        create_note_verse(client, db_name, note_verse_record(&note_uid, verse)).await?;
    }
    for video in &note_videos {
        create_note_video(client, db_name, note_video_record(&note_uid, video)).await?;
    }

    let relationships = build_relationships(session_index, &verses);
    for (other_id, rel_type) in relationships {
        if relationship_keys.insert((note_uid.clone(), other_id.clone(), rel_type.clone())) {
            create_note_relationship(
                client,
                db_name,
                relationship_record(&note_uid, &other_id, &rel_type),
            )
            .await?;
        }
    }

    Ok(ProcessOutcome::Imported(IndexEntry {
        uid: note_uid,
        title,
        summary,
        tags,
        verses: verses.into_iter().map(|verse| verse.full_ref).collect(),
        vector,
    }))
}

async fn run_second_pass_relationships(
    client: &VardaClient,
    db_name: &str,
    http: &reqwest::Client,
    llm: &LlmConfig,
    session_index: &[IndexEntry],
    relationship_keys: &mut HashSet<(String, String, String)>,
    parent_pb: &ProgressBar,
    checkpoint_path: &Path,
    checkpoint: &mut ImportCheckpoint,
) -> Result<()> {
    if session_index.len() < 2 {
        checkpoint.second_pass_completed = true;
        save_checkpoint(checkpoint_path, checkpoint)?;
        return Ok(());
    }

    let second_pass_pb = ProgressBar::new(session_index.len() as u64);
    let mut had_errors = false;
    second_pass_pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} [second pass {elapsed_precise}] {pos}/{len} {msg}",
        )
        .expect("valid second pass progress template"),
    );
    second_pass_pb.set_position(0);
    second_pass_pb.set_message("starting link analysis");
    parent_pb.println("Starting second-pass link analysis...");

    for current in session_index {
        if checkpoint.second_pass_processed_uids.contains(&current.uid) {
            second_pass_pb.inc(1);
            continue;
        }

        second_pass_pb.set_message(current.title.clone());
        let candidates = candidate_notes_for_second_pass(current, session_index);
        if candidates.is_empty() {
            checkpoint
                .second_pass_processed_uids
                .insert(current.uid.clone());
            save_checkpoint(checkpoint_path, checkpoint)?;
            second_pass_pb.inc(1);
            continue;
        }

        let selected_ids =
            match select_relationships_with_llm(http, llm, current, &candidates).await {
                Ok(selected_ids) => selected_ids,
                Err(error) => {
                    had_errors = true;
                    parent_pb.println(format!(
                        "[ ! ] second pass skipped for {} — {}",
                        current.title, error
                    ));
                    second_pass_pb.inc(1);
                    continue;
                }
            };
        if selected_ids.is_empty() {
            checkpoint
                .second_pass_processed_uids
                .insert(current.uid.clone());
            save_checkpoint(checkpoint_path, checkpoint)?;
            second_pass_pb.inc(1);
            continue;
        }

        for other_id in selected_ids {
            let rel_type = "NOTE_RELATED_BY_AI_SECOND_PASS".to_string();
            if relationship_keys.insert((current.uid.clone(), other_id.clone(), rel_type.clone())) {
                create_note_relationship(
                    client,
                    db_name,
                    relationship_record(&current.uid, &other_id, &rel_type),
                )
                .await?;
            }
        }
        checkpoint
            .second_pass_processed_uids
            .insert(current.uid.clone());
        save_checkpoint(checkpoint_path, checkpoint)?;
        second_pass_pb.inc(1);
    }

    if had_errors {
        parent_pb.println("Second pass completed with recoverable errors. Rerun import to retry unfinished notes.");
    } else {
        checkpoint.second_pass_completed = true;
        save_checkpoint(checkpoint_path, checkpoint)?;
    }
    second_pass_pb.finish_and_clear();
    Ok(())
}

async fn create_note(client: &VardaClient, _db_name: &str, record: BulkRecord) -> Result<String> {
    let fields = serde_json::Value::Object(record.fields.into_iter().collect());
    let mutation =
        format!("mutation($input: NoteInput!) {{ createNote(input: $input) {{ uid id }} }}");
    let result = client
        .post_dynamic(&mutation, json!({ "input": fields }))
        .await
        .context("createNote failed")?;
    result["createNote"]["uid"]
        .as_str()
        .map(ToOwned::to_owned)
        .context("createNote response missing uid")
}

async fn create_note_verse(client: &VardaClient, _db_name: &str, record: BulkRecord) -> Result<()> {
    let fields = serde_json::Value::Object(record.fields.into_iter().collect());
    let mutation =
        format!("mutation($input: NoteVerseInput!) {{ createNoteVerse(input: $input) {{ id }} }}");
    client
        .post_dynamic(&mutation, json!({ "input": fields }))
        .await
        .context("createNoteVerse failed")?;
    Ok(())
}

async fn create_note_relationship(
    client: &VardaClient,
    _db_name: &str,
    record: BulkRecord,
) -> Result<()> {
    let fields = serde_json::Value::Object(record.fields.into_iter().collect());
    let mutation = format!(
        "mutation($input: NoteRelationshipInput!) {{ createNoteRelationship(input: $input) {{ id }} }}"
    );
    client
        .post_dynamic(&mutation, json!({ "input": fields }))
        .await
        .context("createNoteRelationship failed")?;
    Ok(())
}

async fn create_note_video(client: &VardaClient, _db_name: &str, record: BulkRecord) -> Result<()> {
    let fields = serde_json::Value::Object(record.fields.into_iter().collect());
    let mutation =
        format!("mutation($input: NoteVideoInput!) {{ createNoteVideo(input: $input) {{ id }} }}");
    client
        .post_dynamic(&mutation, json!({ "input": fields }))
        .await
        .context("createNoteVideo failed")?;
    Ok(())
}

fn load_config(path: &Path) -> Result<ImportConfig> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading config file {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("parsing config file {}", path.display()))
}

fn importer_llm_config(config: &ImportConfig) -> LlmConfig {
    let model_path = Some(config.vault.llm_model.clone());
    LlmConfig {
        url: "http://localhost:8000".to_string(),
        model: derive_model_name(model_path.as_deref()),
        model_path,
    }
}

fn derive_model_name(model_path: Option<&str>) -> String {
    model_path
        .and_then(|path| {
            Path::new(path).components().find_map(|component| {
                let value = component.as_os_str().to_string_lossy();
                value
                    .strip_prefix("models--")
                    .map(|trimmed| trimmed.replace("--", "/"))
            })
        })
        .unwrap_or_else(|| "local-model".to_string())
}

fn canonicalize_excluded_dirs(vault_root: &Path, excluded: &[String]) -> Vec<PathBuf> {
    excluded
        .iter()
        .filter_map(|value| {
            let path = Path::new(value);
            let resolved = if path.is_absolute() {
                path.to_path_buf()
            } else {
                vault_root.join(path)
            };

            fs::canonicalize(&resolved).ok().and_then(|canonical| {
                if canonical.starts_with(vault_root) {
                    Some(canonical)
                } else {
                    None
                }
            })
        })
        .collect()
}

fn markdown_files(vault_root: &Path, excluded_dirs: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = WalkDir::new(vault_root)
        .into_iter()
        .filter_entry(|entry| {
            let path = entry.path();
            !excluded_dirs
                .iter()
                .any(|excluded| path.starts_with(excluded))
        })
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::{canonicalize_excluded_dirs, markdown_files};
    use anyhow::Result;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn markdown_files_skips_excluded_directories_recursively() -> Result<()> {
        let temp = TempDir::new()?;
        let vault_root = temp.path();

        fs::create_dir_all(vault_root.join("keep"))?;
        fs::create_dir_all(vault_root.join("skip/nested"))?;
        fs::write(vault_root.join("keep/note.md"), "# keep")?;
        fs::write(vault_root.join("skip/nested/hidden.md"), "# skip")?;

        let excluded = canonicalize_excluded_dirs(vault_root, &[String::from("skip")]);
        let files = markdown_files(vault_root, &excluded)?;

        assert_eq!(files, vec![vault_root.join("keep/note.md")]);
        Ok(())
    }

    #[test]
    fn canonicalize_excluded_dirs_ignores_paths_outside_vault() -> Result<()> {
        let temp = TempDir::new()?;
        let outside = TempDir::new()?;
        let vault_root = temp.path();

        fs::create_dir_all(vault_root.join("inside"))?;
        let excluded = canonicalize_excluded_dirs(
            vault_root,
            &[
                vault_root.join("inside").display().to_string(),
                outside.path().display().to_string(),
            ],
        );

        assert_eq!(excluded, vec![fs::canonicalize(vault_root.join("inside"))?]);
        Ok(())
    }
}

fn derive_title(file_path: &Path, content: &str) -> String {
    file_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            content
                .lines()
                .find_map(|line| line.strip_prefix("# ").map(str::trim))
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "Untitled Note".to_string())
}

fn generate_tags(vault_root: &Path, file_path: &Path) -> Vec<String> {
    let rel = file_path.strip_prefix(vault_root).unwrap_or(file_path);
    let mut tags = Vec::new();
    let mut current = String::new();

    if let Some(parent) = rel.parent() {
        for component in parent.components() {
            let part = component.as_os_str().to_string_lossy();
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(&part);
            tags.push(current.clone());
        }
    }

    if let Some(stem) = rel.file_stem() {
        tags.push(stem.to_string_lossy().into_owned());
    }

    tags
}

fn category_name_for_path(vault_root: &Path, file_path: &Path) -> String {
    let rel = file_path.strip_prefix(vault_root).unwrap_or(file_path);
    match rel.parent() {
        Some(parent) if parent != Path::new("") => parent.display().to_string(),
        _ => "root".to_string(),
    }
}

fn extract_verses(content: &str) -> Vec<NoteVerseRecord> {
    let mut seen = HashSet::new();
    let mut verses = Vec::new();

    for captures in VERSE_RE.captures_iter(content) {
        let raw_book = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
        let Some(gospel) = canonicalise_book(raw_book) else {
            continue;
        };
        let chapter = captures.get(2).map(|m| m.as_str()).unwrap_or_default();
        let verse = captures.get(3).map(|m| m.as_str()).unwrap_or_default();
        let reference = match captures.get(4) {
            Some(end) => format!("{}:{}-{}", chapter, verse, end.as_str()),
            None => format!("{}:{}", chapter, verse),
        };
        let full_ref = format!("{} {}", gospel, reference);
        if seen.insert(full_ref.to_lowercase()) {
            verses.push(NoteVerseRecord {
                gospel,
                reference,
                full_ref,
            });
        }
    }

    verses
}

fn canonicalise_book(input: &str) -> Option<String> {
    let normalized = input
        .trim()
        .trim_end_matches('.')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    BOOK_ALIASES
        .iter()
        .find_map(|(alias, canonical)| (*alias == normalized).then(|| (*canonical).to_string()))
}

async fn fetch_existing_note_index_entry(
    client: &VardaClient,
    vault_root: &Path,
    file_path: &Path,
    title: &str,
    category_name: &str,
) -> Result<Option<IndexEntry>> {
    let (query, vars) = GraphqlBuilder::new_query("queryNote")
        .arg("filter", json!({ "title": { "eq": title } }))
        .return_fields(&["id", "title", "category { name }"])
        .build();

    let result = client.post_dynamic(&query, vars).await?;
    let Some(note) = result["queryNote"].as_array().and_then(|items| {
        items.iter().find(|item| {
            item["title"].as_str() == Some(title)
                && item["category"]["name"].as_str() == Some(category_name)
        })
    }) else {
        return Ok(None);
    };

    let uid = note["id"]
        .as_str()
        .map(ToOwned::to_owned)
        .context("existing note query missing id")?;
    let content = read_markdown_file_readonly(file_path)?;
    let tags = generate_tags(vault_root, file_path);
    let verses = extract_verses(&content)
        .into_iter()
        .map(|verse| verse.full_ref)
        .collect();

    Ok(Some(IndexEntry {
        uid,
        title: title.to_string(),
        summary: String::new(),
        tags,
        verses,
        vector: Vec::new(),
    }))
}

fn checkpoint_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".notes-import-checkpoint.json")
}

fn load_checkpoint(path: &Path, vault_root: &Path) -> Result<ImportCheckpoint> {
    if !path.exists() {
        return Ok(ImportCheckpoint {
            vault_root: vault_root.display().to_string(),
            ..ImportCheckpoint::default()
        });
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("reading checkpoint file {}", path.display()))?;
    let checkpoint: ImportCheckpoint = serde_json::from_str(&content)
        .with_context(|| format!("parsing checkpoint file {}", path.display()))?;
    if checkpoint.vault_root == vault_root.display().to_string() {
        Ok(checkpoint)
    } else {
        Ok(ImportCheckpoint {
            vault_root: vault_root.display().to_string(),
            ..ImportCheckpoint::default()
        })
    }
}

fn save_checkpoint(path: &Path, checkpoint: &ImportCheckpoint) -> Result<()> {
    let temp_path = path.with_extension("json.tmp");
    let content = serde_json::to_vec_pretty(checkpoint).context("serializing import checkpoint")?;
    fs::write(&temp_path, content)
        .with_context(|| format!("writing checkpoint file {}", temp_path.display()))?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("moving checkpoint file into place {}", path.display()))?;
    Ok(())
}

async fn ensure_note_category(client: &VardaClient, category_name: String) -> Result<String> {
    let (query, vars) = GraphqlBuilder::new_query("queryNoteCategory")
        .arg("filter", json!({ "name": { "eq": &category_name } }))
        .return_fields(&["id", "name"])
        .build();
    let result = client.post_dynamic(&query, vars).await?;
    if let Some(existing_id) = result["queryNoteCategory"]
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item["id"].as_str())
    {
        return Ok(existing_id.to_string());
    }

    let category_id = Uuid::new_v4().to_string();
    let timestamp = Utc::now().to_rfc3339();
    let create_mutation = format!(
        "mutation {{ createNoteCategory(input: {{ id: {}, name: {}, createdAt: {}, updatedAt: {} }}) {{ id name }} }}",
        serde_json::to_string(&category_id)?,
        serde_json::to_string(&category_name)?,
        serde_json::to_string(&timestamp)?,
        serde_json::to_string(&timestamp)?,
    );
    let create_result = client
        .post_dynamic(&create_mutation, serde_json::Value::Null)
        .await?;

    create_result["createNoteCategory"]["id"]
        .as_str()
        .map(ToOwned::to_owned)
        .context("failed to create note category")
}

async fn generate_summary(
    http: &reqwest::Client,
    llm: &LlmConfig,
    title: &str,
    content: &str,
) -> Result<String> {
    let prompt = format!(
        "Summarize this sermon or study note in 2-4 sentences for a card preview.\nBe concise and factual. Output only the summary, no preamble.\n\nTitle: {title}\nContent: {}",
        truncate_chars(content, 4000),
    );

    let response: serde_json::Value = http
        .post(format!(
            "{}/v1/chat/completions",
            llm.url.trim_end_matches('/')
        ))
        .json(&json!({
            "model": llm.model,
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": 256,
            "temperature": 0.3,
            "stream": false
        }))
        .send()
        .await
        .context("summary request failed")?
        .json()
        .await
        .context("summary response parse failed")?;

    Ok(response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string())
}

async fn generate_embedding(
    http: &reqwest::Client,
    llm: &LlmConfig,
    title: &str,
    summary: &str,
) -> Result<Vec<f32>> {
    if summary.trim().is_empty() {
        return Ok(Vec::new());
    }

    let response: serde_json::Value = http
        .post(format!("{}/v1/embeddings", llm.url.trim_end_matches('/')))
        .json(&json!({
            "model": llm.model,
            "input": format!("{title}\n\n{summary}")
        }))
        .send()
        .await
        .context("embedding request failed")?
        .json()
        .await
        .context("embedding response parse failed")?;

    let values = response["data"][0]["embedding"]
        .as_array()
        .context("embedding payload missing data[0].embedding")?;

    Ok(values
        .iter()
        .map(|value| value.as_f64().unwrap_or_default() as f32)
        .collect())
}

fn note_record(
    note_id: &str,
    title: &str,
    content: &str,
    urls: &[String],
    tags: &[String],
    category_id: &str,
    images: &[String],
    summary: &str,
    vector: &[f32],
    timestamp: &str,
) -> BulkRecord {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), json!(note_id));
    fields.insert("title".to_string(), json!(title));
    fields.insert("content".to_string(), json!(content));
    fields.insert("urls".to_string(), json!(urls));
    fields.insert("images".to_string(), json!(images));
    fields.insert("tags".to_string(), json!(tags));
    fields.insert("category".to_string(), json!({ "id": category_id }));
    fields.insert("summary".to_string(), json!(summary));
    fields.insert("embedding".to_string(), json!(vector));
    fields.insert("createdAt".to_string(), json!(timestamp));
    fields.insert("updatedAt".to_string(), json!(timestamp));

    BulkRecord {
        type_name: "Note".to_string(),
        uid: None,
        fields,
    }
}

fn note_verse_record(note_uid: &str, verse: &NoteVerseRecord) -> BulkRecord {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), json!(Uuid::new_v4().to_string()));
    fields.insert("gospel".to_string(), json!(verse.gospel));
    fields.insert("reference".to_string(), json!(verse.reference));
    fields.insert("fullRef".to_string(), json!(verse.full_ref));
    fields.insert("note".to_string(), json!({ "uid": note_uid }));

    BulkRecord {
        type_name: "NoteVerse".to_string(),
        uid: None,
        fields,
    }
}

fn note_video_record(note_uid: &str, video: &NoteVideoRecord) -> BulkRecord {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), json!(Uuid::new_v4().to_string()));
    fields.insert("url".to_string(), json!(video.url));
    fields.insert("videoId".to_string(), json!(video.video_id));
    fields.insert("scriptContent".to_string(), json!(serde_json::Value::Null));
    fields.insert("note".to_string(), json!({ "uid": note_uid }));

    BulkRecord {
        type_name: "NoteVideo".to_string(),
        uid: None,
        fields,
    }
}

fn relationship_record(note_uid: &str, other_uid: &str, relationship_type: &str) -> BulkRecord {
    let mut fields = HashMap::new();
    fields.insert("id".to_string(), json!(Uuid::new_v4().to_string()));
    fields.insert("fromNote".to_string(), json!({ "uid": note_uid }));
    fields.insert("toNote".to_string(), json!({ "uid": other_uid }));
    fields.insert("relationshipType".to_string(), json!(relationship_type));

    BulkRecord {
        type_name: "NoteRelationship".to_string(),
        uid: None,
        fields,
    }
}

fn extract_note_videos(urls: &[String]) -> Vec<NoteVideoRecord> {
    let mut seen = HashSet::new();
    let mut videos = Vec::new();

    for url in urls {
        if let Some(video_id) = extract_youtube_video_id(url) {
            if seen.insert(video_id.clone()) {
                videos.push(NoteVideoRecord {
                    url: url.clone(),
                    video_id,
                });
            }
        }
    }

    videos
}

fn extract_youtube_video_id(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();

    if host == "youtu.be" {
        let id = parsed.path().trim_matches('/').to_string();
        return (!id.is_empty()).then_some(id);
    }

    if host == "youtube.com" || host == "m.youtube.com" {
        if let Some(id) = parsed
            .query_pairs()
            .find_map(|(key, value)| (key == "v").then(|| value.to_string()))
        {
            return Some(id);
        }

        let segments: Vec<_> = parsed
            .path_segments()
            .map(|parts| parts.collect::<Vec<_>>())
            .unwrap_or_default();

        if matches!(segments.first().copied(), Some("shorts" | "embed")) {
            return segments.get(1).map(|value| (*value).to_string());
        }
    }

    None
}

fn build_relationships(
    session_index: &[IndexEntry],
    verses: &[NoteVerseRecord],
) -> Vec<(String, String)> {
    let current_verses: HashSet<&str> =
        verses.iter().map(|verse| verse.full_ref.as_str()).collect();
    let verse_doc_frequency = build_verse_document_frequency(session_index);
    let mut relationships = HashSet::new();

    for entry in session_index {
        let shared_verses: Vec<&String> = entry
            .verses
            .iter()
            .filter(|verse| current_verses.contains(verse.as_str()))
            .collect();
        let has_uncommon_shared_verse = shared_verses
            .iter()
            .any(|verse| is_uncommon_verse(verse, &verse_doc_frequency, session_index.len()));
        if shared_verses.len() >= MIN_SHARED_VERSES_FOR_CONNECTION || has_uncommon_shared_verse {
            relationships.insert((entry.uid.clone(), "NOTE_SHARES_VERSE".to_string()));
        }
    }

    relationships.into_iter().collect()
}

fn build_verse_document_frequency(session_index: &[IndexEntry]) -> HashMap<String, usize> {
    let mut frequency = HashMap::new();

    for entry in session_index {
        let unique_verses: HashSet<&String> = entry.verses.iter().collect();
        for verse in unique_verses {
            *frequency.entry(verse.clone()).or_insert(0) += 1;
        }
    }

    frequency
}

fn is_uncommon_verse(
    verse: &str,
    verse_doc_frequency: &HashMap<String, usize>,
    corpus_size: usize,
) -> bool {
    let frequency = verse_doc_frequency.get(verse).copied().unwrap_or(0);
    if frequency <= UNCOMMON_VERSE_MAX_DOC_FREQ {
        return true;
    }

    let safe_corpus_size = corpus_size.max(1);
    (frequency * 100) / safe_corpus_size <= UNCOMMON_VERSE_MAX_CORPUS_PERCENT
}

fn verse_signal(
    verse: &str,
    verse_doc_frequency: &HashMap<String, usize>,
    corpus_size: usize,
) -> i32 {
    let frequency = verse_doc_frequency.get(verse).copied().unwrap_or(0);
    if frequency == 0 {
        return 4;
    }
    if is_uncommon_verse(verse, verse_doc_frequency, corpus_size) {
        return 3;
    }
    0
}

fn candidate_notes_for_second_pass<'a>(
    current: &'a IndexEntry,
    session_index: &'a [IndexEntry],
) -> Vec<&'a IndexEntry> {
    let verse_doc_frequency = build_verse_document_frequency(session_index);
    let current_verses: HashSet<&str> = current.verses.iter().map(String::as_str).collect();
    let mut scored: Vec<(&IndexEntry, i32)> = session_index
        .iter()
        .filter(|entry| entry.uid != current.uid)
        .filter_map(|entry| {
            let shared_verses: Vec<&String> = entry
                .verses
                .iter()
                .filter(|verse| current_verses.contains(verse.as_str()))
                .collect();
            let shared_verse_score = shared_verses
                .iter()
                .map(|verse| verse_signal(verse, &verse_doc_frequency, session_index.len()))
                .sum::<i32>();
            let has_strong_verse_overlap = shared_verses.len() >= MIN_SHARED_VERSES_FOR_CONNECTION
                || shared_verses.iter().any(|verse| {
                    is_uncommon_verse(verse, &verse_doc_frequency, session_index.len())
                });
            let semantic_score = if current.vector.is_empty() || entry.vector.is_empty() {
                0
            } else if cosine_similarity(&current.vector, &entry.vector)
                >= SEMANTIC_CANDIDATE_THRESHOLD
            {
                2
            } else {
                0
            };
            let total_score = shared_verse_score + semantic_score;
            ((semantic_score > 0 || has_strong_verse_overlap) && total_score > 0)
                .then_some((entry, total_score))
        })
        .collect();

    scored.sort_by(|(a_entry, a_score), (b_entry, b_score)| {
        b_score
            .cmp(a_score)
            .then_with(|| a_entry.title.cmp(&b_entry.title))
    });

    scored
        .into_iter()
        .take(SECOND_PASS_CANDIDATE_LIMIT)
        .map(|(entry, _)| entry)
        .collect()
}

async fn select_relationships_with_llm(
    http: &reqwest::Client,
    llm: &LlmConfig,
    current: &IndexEntry,
    candidates: &[&IndexEntry],
) -> Result<Vec<String>> {
    let candidate_text = candidates
        .iter()
        .map(|candidate| {
            format!(
                "ID: {}\nTitle: {}\nSummary: {}\nVerses: {}",
                candidate.uid,
                candidate.title,
                if candidate.summary.trim().is_empty() {
                    "(none)"
                } else {
                    candidate.summary.trim()
                },
                if candidate.verses.is_empty() {
                    "(none)".to_string()
                } else {
                    candidate.verses.join(", ")
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let prompt = format!(
        "You are linking Christian notes by relevance.\n\
Choose up to 3 candidate notes that are genuinely related to the CURRENT note.\n\
Only link notes that are making a similar point, argument, or interpretive move, or are using the same passage in a materially similar context.\n\
Do not link notes just because they mention the same person, doctrine, or broad theme, such as Paul, grace, or faith.\n\
Verse overlap matters only when it supports contextual similarity rather than loose topical overlap.\n\
Return JSON only in this exact shape: {{\"related_ids\":[\"id1\",\"id2\"]}}\n\
If none are relevant, return {{\"related_ids\":[]}}.\n\n\
CURRENT NOTE\n\
ID: {}\n\
Title: {}\n\
Summary: {}\n\
Verses: {}\n\n\
CANDIDATES\n\
{}",
        current.uid,
        current.title,
        if current.summary.trim().is_empty() {
            "(none)"
        } else {
            current.summary.trim()
        },
        if current.verses.is_empty() {
            "(none)".to_string()
        } else {
            current.verses.join(", ")
        },
        candidate_text
    );

    let response: serde_json::Value = http
        .post(format!(
            "{}/v1/chat/completions",
            llm.url.trim_end_matches('/')
        ))
        .json(&json!({
            "model": llm.model,
            "messages": [{ "role": "user", "content": prompt }],
            "max_tokens": 256,
            "temperature": 0.1,
            "stream": false
        }))
        .send()
        .await
        .context("second-pass relationship request failed")?
        .json()
        .await
        .context("second-pass relationship response parse failed")?;

    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .trim();

    #[derive(Deserialize)]
    struct RelatedIdsResponse {
        related_ids: Vec<String>,
    }

    let parsed: RelatedIdsResponse = serde_json::from_str(content)
        .or_else(|_| {
            extract_json_object(content)
                .ok_or_else(|| serde_json::Error::io(std::io::Error::other("missing JSON object")))
                .and_then(|json| serde_json::from_str(&json))
        })
        .context("failed to parse second-pass relationship JSON")?;

    let candidate_ids: HashSet<&str> = candidates
        .iter()
        .map(|candidate| candidate.uid.as_str())
        .collect();
    Ok(parsed
        .related_ids
        .into_iter()
        .filter(|id| candidate_ids.contains(id.as_str()))
        .collect())
}

fn extract_json_object(input: &str) -> Option<String> {
    let start = input.find('{')?;
    let bytes = input.as_bytes();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape_next = false;

    for (index, &byte) in bytes[start..].iter().enumerate() {
        if escape_next {
            escape_next = false;
            continue;
        }

        match byte {
            b'\\' if in_string => escape_next = true,
            b'"' => in_string = !in_string,
            b'{' if !in_string => depth += 1,
            b'}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(input[start..start + index + 1].to_string());
                }
            }
            _ => {}
        }
    }

    None
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    input.chars().take(max_chars).collect()
}

fn read_markdown_file_readonly(path: &Path) -> Result<String> {
    let mut file = OpenOptions::new()
        .read(true)
        .open(path)
        .with_context(|| format!("opening markdown file in read-only mode {}", path.display()))?;
    let mut content = String::new();
    file.read_to_string(&mut content)
        .with_context(|| format!("reading markdown file {}", path.display()))?;
    Ok(content)
}

async fn load_model_if_needed(http: &reqwest::Client, llm: &LlmConfig) -> Result<()> {
    let Some(model_path) = llm.model_path.as_ref() else {
        return Ok(());
    };

    http.post(format!("{}/llm/load", llm.url.trim_end_matches('/')))
        .json(&json!({ "model_path": model_path }))
        .send()
        .await
        .context("failed to call /llm/load")?
        .error_for_status()
        .context("model load failed")?;

    Ok(())
}

async fn unload_model_if_needed(http: &reqwest::Client, llm: &LlmConfig) -> Result<()> {
    let Some(_) = llm.model_path.as_ref() else {
        return Ok(());
    };

    http.post(format!("{}/llm/unload", llm.url.trim_end_matches('/')))
        .json(&json!({}))
        .send()
        .await
        .context("failed to call /llm/unload")?
        .error_for_status()
        .context("model unload failed")?;

    Ok(())
}

async fn import_images(
    http: &reqwest::Client,
    varda_base_url: &str,
    vault_root: &Path,
    attachment_root: &Path,
    note_path: &Path,
    content: &str,
) -> Result<Vec<String>> {
    let references = extract_image_references(content);
    let mut seen = HashSet::new();
    let mut uploaded = Vec::new();

    for reference in references {
        let Some(path) =
            resolve_attachment_path(vault_root, attachment_root, note_path, &reference)
        else {
            continue;
        };
        let canonical = fs::canonicalize(&path)
            .with_context(|| format!("resolving image path {}", path.display()))?;
        if !seen.insert(canonical.clone()) {
            continue;
        }

        let image_url = upload_image_file(http, varda_base_url, &canonical).await?;
        uploaded.push(image_url);
    }

    Ok(uploaded)
}

fn extract_image_references(content: &str) -> Vec<String> {
    let mut refs = Vec::new();

    for captures in OBSIDIAN_IMAGE_RE.captures_iter(content) {
        if let Some(raw) = captures.get(1) {
            refs.push(raw.as_str().trim().to_string());
        }
    }

    for captures in MARKDOWN_IMAGE_RE.captures_iter(content) {
        if let Some(raw) = captures.get(1) {
            let cleaned = raw
                .as_str()
                .trim()
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
            refs.push(cleaned);
        }
    }

    refs
}

fn extract_urls(content: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut urls = Vec::new();

    for captures in MARKDOWN_LINK_RE.captures_iter(content) {
        if let Some(value) = captures.get(1) {
            let url = value.as_str().trim().to_string();
            if seen.insert(url.to_lowercase()) {
                urls.push(url);
            }
        }
    }

    for captures in RAW_URL_RE.find_iter(content) {
        let url = captures
            .as_str()
            .trim_end_matches(['.', ',', ';', ':'])
            .to_string();
        if seen.insert(url.to_lowercase()) {
            urls.push(url);
        }
    }

    urls
}

fn resolve_attachment_path(
    vault_root: &Path,
    attachment_root: &Path,
    note_path: &Path,
    reference: &str,
) -> Option<PathBuf> {
    let cleaned = reference
        .trim()
        .trim_matches('"')
        .split('#')
        .next()
        .unwrap_or_default()
        .trim();
    if cleaned.is_empty() || cleaned.starts_with("http://") || cleaned.starts_with("https://") {
        return None;
    }

    let candidate = Path::new(cleaned);
    let mut probes = Vec::new();

    if candidate.is_absolute() {
        probes.push(candidate.to_path_buf());
    } else {
        probes.push(note_path.parent().unwrap_or(vault_root).join(candidate));
        probes.push(vault_root.join(candidate));
        probes.push(attachment_root.join(candidate));
    }

    if candidate.extension().is_none() {
        for ext in ["png", "jpg", "jpeg", "gif", "webp", "svg"] {
            probes.push(
                note_path
                    .parent()
                    .unwrap_or(vault_root)
                    .join(format!("{cleaned}.{ext}")),
            );
            probes.push(vault_root.join(format!("{cleaned}.{ext}")));
            probes.push(attachment_root.join(format!("{cleaned}.{ext}")));
        }
    }

    probes.into_iter().find(|path| path.exists())
}

async fn upload_image_file(
    http: &reqwest::Client,
    varda_base_url: &str,
    path: &Path,
) -> Result<String> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let mime = mime_for_path(path);
    let bytes = fs::read(path).with_context(|| format!("reading image file {}", path.display()))?;

    let metadata = format!(
        "filename {},filetype {}",
        encode_metadata(file_name),
        encode_metadata(mime),
    );

    let create = http
        .post(format!("{}/files", varda_base_url.trim_end_matches('/')))
        .header("Tus-Resumable", "1.0.0")
        .header("Upload-Length", bytes.len().to_string())
        .header("Upload-Metadata", metadata)
        .send()
        .await
        .with_context(|| format!("creating upload for {}", path.display()))?
        .error_for_status()
        .context("upload create failed")?;

    let upload_path = create
        .headers()
        .get("Location")
        .and_then(|value| value.to_str().ok())
        .context("missing upload location header")?
        .to_string();

    let patch = http
        .patch(format!(
            "{}{}",
            varda_base_url.trim_end_matches('/'),
            upload_path
        ))
        .header("Tus-Resumable", "1.0.0")
        .header("Content-Type", "application/offset+octet-stream")
        .header("Upload-Offset", "0")
        .body(bytes)
        .send()
        .await
        .with_context(|| format!("uploading image {}", path.display()))?
        .error_for_status()
        .context("upload patch failed")?;

    patch
        .headers()
        .get("Varda-File-Url")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
        .context("missing Varda-File-Url header")
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
    {
        Some(ext) if ext == "png" => "image/png",
        Some(ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg",
        Some(ext) if ext == "gif" => "image/gif",
        Some(ext) if ext == "webp" => "image/webp",
        Some(ext) if ext == "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn encode_metadata(value: &str) -> String {
    use base64::{Engine as _, engine::general_purpose};
    general_purpose::STANDARD.encode(value.as_bytes())
}
