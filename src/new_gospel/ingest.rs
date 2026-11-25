use anyhow::{Context, Result, anyhow};
use clap::Args;
use once_cell::sync::Lazy;
use regex::Regex;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

static VERSE_LINE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*\d+\s+(?P<text>.*?)(?:\s*\|\s*(?P<support>.*))?\s*$")
        .expect("valid verse regex")
});
static PAREN_TAG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\([^)]*\)").expect("valid parenthetical tag regex"));
static COMMA_EXPANSION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^(?P<prefix>.+?\s)(?P<refs>\d+(?::\d+(?:-\d+)?)?(?:\s*,\s*\d+(?::\d+(?:-\d+)?)?)*)$",
    )
    .expect("valid comma expansion regex")
});
const DB_NAME: &str = "new_gospel";
const VERSE_TYPE: &str = "NewGospelVerse";
const SUPPORT_TYPE: &str = "SupportingReference";
const SUPPORT_FIELD: &str = "supportingReferences";
const APP_SCHEMA_PATH: &str = "schema/new_gospel.graphql";

#[derive(Args, Debug)]
#[command(about = "Import chapter text from new-gospel.import_dir in config.toml")]
pub struct Cli {
    #[arg(long)]
    db_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedVerse {
    number: i64,
    pericope: Option<String>,
    text: String,
    notes: String,
    supporting_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VerifiedVerse {
    number: i64,
    pericope: Option<String>,
    text: String,
    notes: String,
    supporting_references: Vec<String>,
}

#[derive(Default)]
struct IngestState {
    next_uid: u64,
}

impl IngestState {
    fn new() -> Self {
        Self { next_uid: 1 }
    }

    fn next_uid(&mut self) -> u64 {
        let uid = self.next_uid;
        self.next_uid += 1;
        uid
    }
}

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

fn registered_db_path(db_dir: &Path, node_id: Option<u64>) -> Result<PathBuf> {
    anyhow::ensure!(
        db_dir.join("default.db").is_file(),
        "Database {DB_NAME} not found in the Archon registry at {}",
        db_dir.display()
    );
    let storage = vardadb::storage::backend::Storage::new(db_dir, node_id)?;
    storage
        .list_databases()
        .into_iter()
        .find(|(name, _)| name == DB_NAME)
        .map(|(_, path)| PathBuf::from(path))
        .with_context(|| {
            format!(
                "Database {DB_NAME} not found in the Archon registry at {}",
                db_dir.display()
            )
        })
}

pub fn run(cli: Cli) -> Result<()> {
    let config = crate::startup::load_varda_config("config.toml")
        .map_err(|e| anyhow!("Failed to load config.toml: {e}"))?;
    let db_dir = cli
        .db_dir
        .unwrap_or_else(|| PathBuf::from(config.server.storage_path));
    let settings = super::read_settings()?;
    let input_dir = match super::configured_path(&settings, &["new-gospel", "import_dir"])? {
        Some(path) => path,
        None => super::configured_path(&settings, &["ingest-new-gospel", "input_dir"])?
            .context("Set new-gospel.import_dir in config.toml")?,
    };
    let verses = parse_all_files(&input_dir)?;
    anyhow::ensure!(
        !verses.is_empty(),
        "No verses found in {}",
        input_dir.display()
    );

    let db_path = registered_db_path(&db_dir, config.server.node_id)?;
    let mut conn = open_db(&db_path)?;
    persist_db_schema(&db_dir)?;
    let tables = DbTables::for_db(DB_NAME);
    create_tables(&conn, &tables)?;
    import_verses(&mut conn, &tables, &verses)?;

    let verified = load_verified_verse(&conn, &tables, 1)?
        .ok_or_else(|| anyhow!("imported verse 1 not found after ingest"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "number": verified.number,
            "pericope": verified.pericope,
            "text": verified.text,
            "notes": verified.notes,
            "supportingReferences": verified.supporting_references.into_iter().map(|display| serde_json::json!({ "display": display })).collect::<Vec<_>>(),
        }))?
    );

    Ok(())
}

fn persist_db_schema(db_dir: &Path) -> Result<()> {
    fs::create_dir_all(db_dir).with_context(|| format!("creating db dir {}", db_dir.display()))?;

    let app_schema_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(APP_SCHEMA_PATH);
    let schema = fs::read_to_string(&app_schema_path)
        .with_context(|| format!("reading {}", app_schema_path.display()))?;

    let persisted_schema_path = db_dir.join(format!("{DB_NAME}_schema.graphql"));
    fs::write(&persisted_schema_path, schema)
        .with_context(|| format!("writing {}", persisted_schema_path.display()))?;

    Ok(())
}

fn open_db(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("opening registered database {}", db_path.display()))?;

    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA temp_store = MEMORY;
         PRAGMA mmap_size = 30000000000;
         PRAGMA cache_size = -131072;
         PRAGMA foreign_keys = OFF;",
    )?;

    Ok(conn)
}

fn create_tables(conn: &Connection, tables: &DbTables) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS \"{main}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL, ts BLOB NOT NULL) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS \"{history}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;",
        main = tables.main,
        history = tables.history,
    ))?;
    Ok(())
}

fn clear_keyspace(conn: &Connection, tables: &DbTables) -> Result<()> {
    conn.execute_batch(&format!(
        "DELETE FROM \"{main}\";
         DELETE FROM \"{history}\";",
        main = tables.main,
        history = tables.history,
    ))?;
    Ok(())
}

fn import_verses(conn: &mut Connection, tables: &DbTables, verses: &[ParsedVerse]) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    clear_keyspace(&tx, tables)?;
    let mut state = IngestState::new();
    let mut main_stmt = tx.prepare_cached(&format!(
        "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, ts = excluded.ts
         WHERE excluded.ts > \"{}\".ts",
        tables.main, tables.main
    ))?;
    let mut history_stmt = tx.prepare_cached(&format!(
        "INSERT INTO \"{}\" (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        tables.history
    ))?;

    let ts = [0u8; 16];

    for verse in verses {
        let verse_uid = state.next_uid();
        let mut support_uids = Vec::with_capacity(verse.supporting_references.len());

        write_type_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            verse_uid,
            VERSE_TYPE,
        )?;

        for display in &verse.supporting_references {
            let support_uid = state.next_uid();
            support_uids.push(support_uid);

            write_type_record(
                &mut main_stmt,
                &mut history_stmt,
                &ts,
                support_uid,
                SUPPORT_TYPE,
            )?;
            write_field_record(
                &mut main_stmt,
                &mut history_stmt,
                &ts,
                support_uid,
                "display",
                &Value::String(display.clone()),
            )?;

            write_edge_record(
                &mut main_stmt,
                &mut history_stmt,
                &ts,
                verse_uid,
                support_uid,
                SUPPORT_FIELD,
            )?;
        }

        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            verse_uid,
            "number",
            &Value::Number(verse.number.into()),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            verse_uid,
            "pericope",
            &match &verse.pericope {
                Some(value) => Value::String(value.clone()),
                None => Value::Null,
            },
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            verse_uid,
            "text",
            &Value::String(verse.text.clone()),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            verse_uid,
            "notes",
            &Value::String(verse.notes.clone()),
        )?;
        write_field_record(
            &mut main_stmt,
            &mut history_stmt,
            &ts,
            verse_uid,
            SUPPORT_FIELD,
            &Value::Array(
                support_uids
                    .iter()
                    .map(|uid| Value::String(uid.to_string()))
                    .collect(),
            ),
        )?;

        write_unique_index(
            &mut main_stmt,
            &ts,
            verse_uid,
            &format!("{VERSE_TYPE}.number"),
            &serde_json::to_string(&Value::Number(verse.number.into()))?,
        )?;
    }

    drop(main_stmt);
    drop(history_stmt);
    tx.commit()?;
    Ok(())
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

fn write_unique_index(
    main_stmt: &mut rusqlite::CachedStatement<'_>,
    ts: &[u8; 16],
    uid: u64,
    predicate: &str,
    value: &str,
) -> Result<()> {
    let key = encode_unique_index_key(predicate, value);
    main_stmt.execute(params![key, uid.to_be_bytes().to_vec(), &ts[..]])?;
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

fn parse_all_files(source_dir: &Path) -> Result<Vec<ParsedVerse>> {
    let mut verses = Vec::new();
    let mut next_number = 1_i64;

    let mut chapter_files: Vec<PathBuf> = fs::read_dir(source_dir)
        .with_context(|| format!("reading source directory {}", source_dir.display()))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("chapter_") && n.ends_with(".txt"))
                    .unwrap_or(false)
        })
        .collect();

    chapter_files.sort_by_key(|path| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("chapter_"))
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
    });

    for file_path in &chapter_files {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("reading source file {}", file_path.display()))?;
        verses.extend(parse_verses_from_str(&content, &mut next_number)?);
    }

    Ok(verses)
}

fn parse_verses_from_str(content: &str, next_number: &mut i64) -> Result<Vec<ParsedVerse>> {
    let mut parsed = Vec::new();
    let mut current_pericope: Option<String> = None;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(captures) = VERSE_LINE_RE.captures(line) {
            let text = captures
                .name("text")
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default();
            if text.is_empty() {
                continue;
            }

            let annotation_area = captures.name("support").map(|m| m.as_str()).unwrap_or("");
            let (supporting_references, notes) = parse_annotations(annotation_area);

            parsed.push(ParsedVerse {
                number: *next_number,
                pericope: current_pericope.clone(),
                text,
                notes,
                supporting_references,
            });
            *next_number += 1;
            continue;
        }

        current_pericope = Some(line.to_string());
    }

    Ok(parsed)
}

fn parse_annotations(annotation_area: &str) -> (Vec<String>, String) {
    let segments: Vec<&str> = annotation_area
        .split('|')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect();

    if segments.is_empty() {
        return (Vec::new(), String::new());
    }

    if segments.len() == 1 {
        return (
            parse_supporting_reference_segment(segments[0]),
            String::new(),
        );
    }

    let notes = segments.last().copied().unwrap_or_default().to_string();
    let references = segments[..segments.len() - 1]
        .iter()
        .copied()
        .flat_map(parse_supporting_reference_segment)
        .collect();
    (references, notes)
}

fn parse_supporting_reference_segment(support_area: &str) -> Vec<String> {
    let cleaned = PAREN_TAG_RE.replace_all(support_area, "");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        return Vec::new();
    }

    cleaned
        .split('|')
        .flat_map(|segment| expand_reference_segment(segment.trim()))
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn expand_reference_segment(segment: &str) -> Vec<String> {
    if segment.is_empty() {
        return Vec::new();
    }

    if let Some(captures) = COMMA_EXPANSION_RE.captures(segment) {
        let prefix = captures
            .name("prefix")
            .map(|m| m.as_str().trim_end().to_string())
            .unwrap_or_default();
        let refs = captures
            .name("refs")
            .map(|m| m.as_str())
            .unwrap_or_default();

        if refs.contains(',') {
            return refs
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("{prefix} {value}"))
                .collect();
        }
    }

    vec![segment.to_string()]
}

fn load_verified_verse(
    conn: &Connection,
    tables: &DbTables,
    verse_number: i64,
) -> Result<Option<VerifiedVerse>> {
    let index_key = encode_unique_index_key(
        &format!("{VERSE_TYPE}.number"),
        &serde_json::to_string(&Value::Number(verse_number.into()))?,
    );
    let uid_bytes: Option<Vec<u8>> = conn
        .query_row(
            &format!("SELECT value FROM \"{}\" WHERE key = ?1", tables.main),
            params![index_key],
            |row| row.get(0),
        )
        .optional()?;

    let Some(uid_bytes) = uid_bytes else {
        return Ok(None);
    };
    if uid_bytes.len() != 8 {
        return Err(anyhow!("invalid uid length for verse index"));
    }
    let uid = u64::from_be_bytes(uid_bytes.try_into().expect("uid bytes length checked"));

    let number = read_json_field(conn, &tables.main, uid, "number")?
        .and_then(|value| value.as_i64())
        .ok_or_else(|| anyhow!("missing number for verse uid {}", uid))?;
    let pericope = read_json_field(conn, &tables.main, uid, "pericope")?
        .and_then(|value| value.as_str().map(ToOwned::to_owned));
    let text = read_json_field(conn, &tables.main, uid, "text")?
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| anyhow!("missing text for verse uid {}", uid))?;
    let notes = read_json_field(conn, &tables.main, uid, "notes")?
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default();

    let support_uids = read_json_field(conn, &tables.main, uid, SUPPORT_FIELD)?
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut supporting_references = Vec::with_capacity(support_uids.len());
    for support_uid in support_uids {
        let support_uid = support_uid
            .as_str()
            .ok_or_else(|| anyhow!("support uid must be stored as string"))?
            .parse::<u64>()
            .context("parsing stored support uid")?;
        let display = read_json_field(conn, &tables.main, support_uid, "display")?
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .ok_or_else(|| anyhow!("missing display for support uid {}", support_uid))?;
        supporting_references.push(display);
    }

    Ok(Some(VerifiedVerse {
        number,
        pericope,
        text,
        notes,
        supporting_references,
    }))
}

fn read_json_field(conn: &Connection, table: &str, uid: u64, field: &str) -> Result<Option<Value>> {
    let value: Option<Vec<u8>> = conn
        .query_row(
            &format!("SELECT value FROM \"{}\" WHERE key = ?1", table),
            params![encode_data_key(uid, field)],
            |row| row.get(0),
        )
        .optional()?;

    let Some(value) = value else {
        return Ok(None);
    };
    if value.len() < 16 {
        return Err(anyhow!(
            "stored value for uid {} field {} is too short",
            uid,
            field
        ));
    }
    Ok(Some(serde_json::from_slice(&value[16..])?))
}

fn encode_data_key(uid: u64, predicate: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 8 + predicate.len());
    buf.push(0x01);
    buf.extend_from_slice(&uid.to_be_bytes());
    buf.extend_from_slice(predicate.as_bytes());
    buf
}

fn encode_unique_index_key(predicate: &str, value: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + predicate.len() + 1 + value.len());
    buf.push(0x02);
    buf.extend_from_slice(predicate.as_bytes());
    buf.push(0x00);
    buf.extend_from_slice(value.as_bytes());
    buf
}

fn encode_type_index_key(type_name: &str, uid: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + type_name.len() + 1 + 8);
    buf.push(0x03);
    buf.extend_from_slice(type_name.as_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&uid.to_be_bytes());
    buf
}

fn encode_history_key(timestamp: &[u8; 16], uid: u64, predicate: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(16 + 8 + predicate.len());
    buf.extend_from_slice(timestamp);
    buf.extend_from_slice(&uid.to_be_bytes());
    buf.extend_from_slice(predicate.as_bytes());
    buf
}

fn encode_edge_key(target_uid: u64, field: &str, source_uid: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 8 + field.len() + 1 + 8);
    buf.push(0x07);
    buf.extend_from_slice(&target_uid.to_be_bytes());
    buf.extend_from_slice(field.as_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&source_uid.to_be_bytes());
    buf
}

fn encode_reverse_edge_key(source_uid: u64, target_uid: u64, field: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + 8 + 8 + field.len() + 1);
    buf.push(0x08);
    buf.extend_from_slice(&source_uid.to_be_bytes());
    buf.extend_from_slice(&target_uid.to_be_bytes());
    buf.extend_from_slice(field.as_bytes());
    buf.push(0x00);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parses_first_sample_block() {
        let input = "Dedication to the Reader\n\n1 Since many have undertaken to compile a narrative about the events that have long since passed, | (NRSVUE) Luke 1:1\n";
        let mut next_number = 1;
        let parsed = parse_verses_from_str(input, &mut next_number).expect("parse should succeed");

        assert_eq!(
            parsed,
            vec![ParsedVerse {
                number: 1,
                pericope: Some("Dedication to the Reader".to_string()),
                text: "Since many have undertaken to compile a narrative about the events that have long since passed,".to_string(),
                notes: String::new(),
                supporting_references: vec!["Luke 1:1".to_string()],
            }]
        );
        assert_eq!(next_number, 2);
    }

    #[test]
    fn parses_two_field_annotations_as_references_without_notes() {
        assert_eq!(
            parse_annotations("Luke 1:1"),
            (vec!["Luke 1:1".to_string()], String::new())
        );
    }

    #[test]
    fn expands_comma_separated_references() {
        assert_eq!(
            parse_supporting_reference_segment("Gospel of Thomas 6, 9, 10"),
            vec![
                "Gospel of Thomas 6".to_string(),
                "Gospel of Thomas 9".to_string(),
                "Gospel of Thomas 10".to_string(),
            ]
        );
    }

    #[test]
    fn drops_tag_only_references() {
        assert!(parse_supporting_reference_segment("(creative)").is_empty());
    }

    #[test]
    fn parses_notes_as_third_pipe_delimited_field() {
        let input = "11 When they had finished everything required by the law of their Lord, they returned to Galilee, to their own town of Nazareth. | (NRSVUE) Luke 2:39 | Almost  but another clarification of the lord\n";
        let mut next_number = 11;
        let parsed = parse_verses_from_str(input, &mut next_number).expect("parse should succeed");

        assert_eq!(
            parsed,
            vec![ParsedVerse {
                number: 11,
                pericope: None,
                text: "When they had finished everything required by the law of their Lord, they returned to Galilee, to their own town of Nazareth.".to_string(),
                notes: "Almost  but another clarification of the lord".to_string(),
                supporting_references: vec!["Luke 2:39".to_string()],
            }]
        );
    }

    #[test]
    fn numbering_is_global_across_files() {
        let chapter_one = "Heading\n1 Verse one | Luke 1:1\n";
        let chapter_two = "Another heading\n1 Verse two | Mark 1:1\n";
        let mut next_number = 1;

        let first = parse_verses_from_str(chapter_one, &mut next_number).expect("chapter one");
        let second = parse_verses_from_str(chapter_two, &mut next_number).expect("chapter two");

        assert_eq!(first[0].number, 1);
        assert_eq!(second[0].number, 2);
    }

    #[test]
    fn encodes_data_key() {
        let key = encode_data_key(42, "text");
        assert_eq!(key[0], 0x01);
        assert_eq!(&key[1..9], &42_u64.to_be_bytes());
        assert_eq!(&key[9..], b"text");
    }

    #[test]
    fn encodes_type_index_key() {
        let key = encode_type_index_key("NewGospelVerse", 42);
        assert_eq!(key[0], 0x03);
        assert!(key.starts_with(&[0x03]));
        assert_eq!(&key[key.len() - 8..], &42_u64.to_be_bytes());
    }

    #[test]
    fn encodes_unique_index_key() {
        let key = encode_unique_index_key("NewGospelVerse.number", "1");
        assert_eq!(key[0], 0x02);
        let predicate_len = "NewGospelVerse.number".len();
        assert_eq!(&key[1..1 + predicate_len], b"NewGospelVerse.number");
        assert_eq!(key[1 + predicate_len], 0x00);
        assert_eq!(&key[2 + predicate_len..], b"1");
    }

    #[test]
    fn encodes_edge_key() {
        let key = encode_edge_key(9, SUPPORT_FIELD, 7);
        assert_eq!(key[0], 0x07);
        assert_eq!(&key[1..9], &9_u64.to_be_bytes());
        assert_eq!(&key[key.len() - 8..], &7_u64.to_be_bytes());
    }

    #[test]
    fn rejects_unregistered_database_file() {
        let dir = tempdir().unwrap();
        drop(vardadb::storage::backend::Storage::new(dir.path(), None).unwrap());
        let path = dir.path().join("new_gospel.db");
        assert!(open_db(&path).is_err());
        assert!(!path.exists());
        drop(Connection::open(&path).unwrap());
        assert!(
            registered_db_path(dir.path(), None)
                .unwrap_err()
                .to_string()
                .contains("not found")
        );
    }

    #[test]
    fn imports_and_reads_back_from_sqlite() {
        let dir = tempdir().expect("temp dir");
        let storage = vardadb::storage::backend::Storage::new(dir.path(), None).unwrap();
        storage.create_database(DB_NAME).unwrap();
        drop(storage);
        let path = registered_db_path(dir.path(), None).unwrap();
        let mut conn = open_db(&path).expect("open db");
        let tables = DbTables::for_db(DB_NAME);
        create_tables(&conn, &tables).expect("create tables");
        clear_keyspace(&conn, &tables).expect("clear keyspace");

        let verses = vec![ParsedVerse {
            number: 1,
            pericope: Some("Dedication to the Reader".to_string()),
            text: "Since many have undertaken to compile a narrative about the events that have long since passed,".to_string(),
            notes: "Example note".to_string(),
            supporting_references: vec!["Luke 1:1".to_string()],
        }];

        import_verses(&mut conn, &tables, &verses).expect("import verses");

        let created_tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("prepare sqlite_master query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query sqlite_master")
            .filter_map(|row| row.ok())
            .collect();
        assert!(created_tables.contains(&"new_gospel_main".to_string()));
        assert!(created_tables.contains(&"new_gospel_history".to_string()));

        let verified = load_verified_verse(&conn, &tables, 1)
            .expect("load verified verse")
            .expect("verse should exist");
        assert_eq!(verified.number, 1);
        assert_eq!(
            verified.pericope.as_deref(),
            Some("Dedication to the Reader")
        );
        assert_eq!(verified.notes, "Example note");
        assert_eq!(verified.supporting_references, vec!["Luke 1:1".to_string()]);
    }

    #[test]
    fn persists_database_schema_file() {
        let dir = tempdir().expect("temp dir");

        persist_db_schema(dir.path()).expect("persist db schema");

        let persisted_schema_path = dir.path().join("new_gospel_schema.graphql");
        let persisted_schema =
            fs::read_to_string(&persisted_schema_path).expect("read persisted schema");
        let source_schema =
            fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(APP_SCHEMA_PATH))
                .expect("read source schema");

        assert_eq!(persisted_schema, source_schema);
    }
}
