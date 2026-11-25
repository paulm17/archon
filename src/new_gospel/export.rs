use anyhow::{Context, Result, anyhow};
use clap::Args;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const DB_NAME: &str = "new_gospel";
const VERSE_TYPE: &str = "NewGospelVerse";
const SUPPORT_FIELD: &str = "supportingReferences";
const CHAPTER_PREFIX: &str = "chapter_";
const CHAPTER_SUFFIX: &str = ".txt";
const CHAPTER_SPLIT_AFTER: usize = 40;

#[derive(Args, Debug)]
#[command(about = "Export chapter text to new-gospel.export_dir in config.toml")]
pub struct Cli {
    #[arg(long, default_value_os_t = default_db_dir())]
    db_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DbVerse {
    number: i64,
    pericope: Option<String>,
    text: String,
    supporting_references: Vec<String>,
    notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChapterFile {
    file_name: String,
    content: String,
}

struct DbTables {
    main: String,
}

impl DbTables {
    fn for_db(name: &str) -> Self {
        Self {
            main: format!("{name}_main"),
        }
    }
}

pub(super) fn default_db_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("db_data")
}

fn output_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("output")
        .join("new_gospel")
}

pub fn run(cli: Cli) -> Result<()> {
    let settings = super::read_settings()?;
    let directory = super::configured_path(&settings, &["new-gospel", "export_dir"])?
        .unwrap_or_else(output_dir);
    let conn = open_db(&cli.db_dir)?;
    let tables = DbTables::for_db(DB_NAME);
    let verses = load_db_verses(&conn, &tables)?;
    let chapters = build_chapters(&verses);

    write_chapters(&directory, &chapters)?;

    println!(
        "Exported {} verses into {} chapter files in {}",
        verses.len(),
        chapters.len(),
        directory.display()
    );

    Ok(())
}

fn open_db(db_dir: &Path) -> Result<Connection> {
    let db_path = db_dir.join("new_gospel.db");
    let conn =
        Connection::open(&db_path).with_context(|| format!("opening {}", db_path.display()))?;

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

fn load_db_verses(conn: &Connection, tables: &DbTables) -> Result<Vec<DbVerse>> {
    let fields = load_fields(conn, &tables.main)?;
    let mut verses = Vec::new();
    for record in fields.values() {
        if record.get("_type").and_then(Value::as_str) != Some(VERSE_TYPE) {
            continue;
        }

        let number = record
            .get("number")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("missing number for verse"))?;
        let pericope = record
            .get("pericope")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let text = record
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing text for verse {number}"))?
            .to_string();
        let supporting_references = load_supporting_references(record, &fields)?;

        let notes = record
            .get("notes")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        verses.push(DbVerse {
            number,
            pericope,
            text,
            supporting_references,
            notes,
        });
    }

    verses.sort_by_key(|verse| verse.number);
    Ok(verses)
}

pub(super) fn load_fields(
    conn: &Connection,
    table: &str,
) -> Result<BTreeMap<u64, BTreeMap<String, Value>>> {
    let mut stmt = conn.prepare(&format!("SELECT key, value FROM \"{table}\""))?;
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

        let uid = u64::from_be_bytes(key[1..9].try_into().expect("uid slice length checked"));
        let field = String::from_utf8_lossy(&key[9..]).into_owned();
        let parsed = serde_json::from_slice::<Value>(&value[16..])
            .with_context(|| format!("decoding field {field} for uid {uid}"))?;
        fields.entry(uid).or_default().insert(field, parsed);
    }

    Ok(fields)
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
        let display = fields
            .get(&support_uid)
            .and_then(|support| support.get("display"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing display for support uid {support_uid}"))?;
        supporting_references.push(display.to_string());
    }

    Ok(supporting_references)
}

fn build_chapters(verses: &[DbVerse]) -> Vec<ChapterFile> {
    if verses.is_empty() {
        return Vec::new();
    }

    let mut chapters = Vec::new();
    let mut chapter_number = 1usize;
    let mut chapter_lines: Vec<String> = Vec::new();
    let mut chapter_verse_count = 0usize;
    let mut sequence = 1usize;
    let mut previous_pericope: Option<&str> = None;

    for verse in verses {
        let current_pericope = verse.pericope.as_deref();
        let is_new_pericope_block = current_pericope != previous_pericope;

        if chapter_verse_count >= CHAPTER_SPLIT_AFTER
            && chapter_verse_count > 0
            && is_new_pericope_block
        {
            chapters.push(ChapterFile {
                file_name: format!("{CHAPTER_PREFIX}{chapter_number}{CHAPTER_SUFFIX}"),
                content: finalize_chapter_content(&chapter_lines),
            });
            chapter_number += 1;
            chapter_lines.clear();
            chapter_verse_count = 0;
            sequence = 1;
        }

        if is_new_pericope_block {
            if let Some(pericope) = current_pericope {
                if !chapter_lines.is_empty() {
                    chapter_lines.push(String::new());
                }
                chapter_lines.push(pericope.to_string());
                chapter_lines.push(String::new());
            }
        }

        chapter_lines.push(format_verse_line(sequence, verse));
        chapter_verse_count += 1;
        sequence += 1;
        previous_pericope = current_pericope;
    }

    chapters.push(ChapterFile {
        file_name: format!("{CHAPTER_PREFIX}{chapter_number}{CHAPTER_SUFFIX}"),
        content: finalize_chapter_content(&chapter_lines),
    });

    chapters
}

fn format_verse_line(sequence: usize, verse: &DbVerse) -> String {
    let display = verse.supporting_references.join(", ");
    let text = verse
        .text
        .replace("\r\n", " ")
        .replace('\r', " ")
        .replace('\n', " ");
    let base = if display.is_empty() {
        format!("{sequence} {text}")
    } else {
        format!("{sequence} {text} | {display}")
    };
    match &verse.notes {
        Some(note) if !note.is_empty() => {
            let note = note
                .replace("\r\n", " ")
                .replace('\r', " ")
                .replace('\n', " ");
            let note = note.trim_end();
            format!("{base} | {note}")
        }
        _ => base,
    }
}

fn finalize_chapter_content(lines: &[String]) -> String {
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn write_chapters(dir: &Path, chapters: &[ChapterFile]) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    clear_existing_chapter_files(dir)?;

    for chapter in chapters {
        let path = dir.join(&chapter.file_name);
        fs::write(&path, &chapter.content)
            .with_context(|| format!("writing {}", path.display()))?;
    }

    Ok(())
}

fn clear_existing_chapter_files(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if is_generated_chapter_file(name) {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }

    Ok(())
}

fn is_generated_chapter_file(name: &str) -> bool {
    if !name.starts_with(CHAPTER_PREFIX) || !name.ends_with(CHAPTER_SUFFIX) {
        return false;
    }

    let middle = &name[CHAPTER_PREFIX.len()..name.len() - CHAPTER_SUFFIX.len()];
    !middle.is_empty() && middle.chars().all(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::tempdir;

    #[test]
    fn exports_under_output_new_gospel() {
        assert_eq!(
            output_dir(),
            Path::new(env!("CARGO_MANIFEST_DIR")).join("output/new_gospel")
        );
    }

    #[test]
    fn sorts_loaded_verses_by_number() {
        let dir = tempdir().expect("temp dir");
        let conn = open_db(dir.path()).expect("open db");
        let tables = DbTables::for_db(DB_NAME);
        create_tables(&conn, &tables).expect("create tables");

        seed_verse(&conn, &tables, 10, 2, Some("Later"), &["Luke 1:2"]).expect("seed verse 2");
        seed_verse(&conn, &tables, 20, 1, Some("Earlier"), &["Luke 1:1"]).expect("seed verse 1");

        let verses = load_db_verses(&conn, &tables).expect("load verses");

        assert_eq!(verses.len(), 2);
        assert_eq!(verses[0].number, 1);
        assert_eq!(verses[1].number, 2);
        assert_eq!(verses[0].text, "Verse 1");
    }

    #[test]
    fn formats_single_supporting_reference() {
        let verse = DbVerse {
            number: 12,
            pericope: None,
            text: "Verse 12".to_string(),
            supporting_references: vec!["Luke 2:12".to_string()],
            notes: None,
        };

        assert_eq!(format_verse_line(3, &verse), "3 Verse 12 | Luke 2:12");
    }

    #[test]
    fn formats_multiple_supporting_references() {
        let verse = DbVerse {
            number: 12,
            pericope: None,
            text: "Verse 12".to_string(),
            supporting_references: vec!["Luke 2:12".to_string(), "Mark 1:12".to_string()],
            notes: None,
        };

        assert_eq!(
            format_verse_line(3, &verse),
            "3 Verse 12 | Luke 2:12, Mark 1:12"
        );
    }

    #[test]
    fn formats_missing_pericope_without_title() {
        let chapters = build_chapters(&[DbVerse {
            number: 1,
            pericope: None,
            text: "Verse 1".to_string(),
            supporting_references: vec!["Luke 1:1".to_string()],
            notes: None,
        }]);

        assert_eq!(chapters.len(), 1);
        assert_eq!(chapters[0].content, "1 Verse 1 | Luke 1:1\n");
    }

    #[test]
    fn keeps_single_chapter_when_under_limit() {
        let verses = make_verses(1..=3, &["Start"], &[1..=3]);
        let chapters = build_chapters(&verses);

        assert_eq!(chapters.len(), 1);
        assert!(chapters[0].content.contains("Start"));
        assert!(chapters[0].content.contains("3 Verse 3 | Ref 3"));
    }

    #[test]
    fn does_not_split_at_exactly_forty_without_following_pericope() {
        let verses = make_verses(1..=40, &["One"], &[1..=40]);
        let chapters = build_chapters(&verses);

        assert_eq!(chapters.len(), 1);
        assert!(chapters[0].content.contains("40 Verse 40 | Ref 40"));
    }

    #[test]
    fn splits_after_passing_forty_at_next_pericope() {
        let verses = make_verses(1..=43, &["One", "Two"], &[1..=41, 42..=43]);
        let chapters = build_chapters(&verses);

        assert_eq!(chapters.len(), 2);
        assert!(chapters[0].content.contains("41 Verse 41 | Ref 41"));
        assert!(!chapters[0].content.contains("\nTwo\n\n"));
        assert_eq!(
            chapters[1].content,
            "Two\n\n1 Verse 42 | Ref 42\n2 Verse 43 | Ref 43\n"
        );
    }

    #[test]
    fn splits_when_pericope_starts_immediately_after_forty() {
        let verses = make_verses(1..=42, &["One", "Two"], &[1..=40, 41..=42]);
        let chapters = build_chapters(&verses);

        assert_eq!(chapters.len(), 2);
        assert!(chapters[0].content.contains("40 Verse 40 | Ref 40"));
        assert_eq!(
            chapters[1].content,
            "Two\n\n1 Verse 41 | Ref 41\n2 Verse 42 | Ref 42\n"
        );
    }

    #[test]
    fn writes_chapters_and_clears_old_generated_files() {
        let db_dir = tempdir().expect("db dir");
        let out_dir = tempdir().expect("output dir");
        let conn = open_db(db_dir.path()).expect("open db");
        let tables = DbTables::for_db(DB_NAME);
        create_tables(&conn, &tables).expect("create tables");

        for number in 1..=42 {
            let pericope = if number <= 40 {
                Some("First")
            } else {
                Some("Second")
            };
            let support = format!("Ref {number}");
            seed_verse(&conn, &tables, number as u64, number, pericope, &[&support])
                .expect("seed verse");
        }

        fs::create_dir_all(out_dir.path()).expect("create output dir");
        fs::write(out_dir.path().join("chapter_99.txt"), "stale").expect("stale file");
        fs::write(out_dir.path().join("notes.txt"), "keep").expect("unrelated file");

        let verses = load_db_verses(&conn, &tables).expect("load verses");
        let chapters = build_chapters(&verses);
        write_chapters(out_dir.path(), &chapters).expect("write chapters");

        let chapter_one =
            fs::read_to_string(out_dir.path().join("chapter_1.txt")).expect("read chapter 1");
        let chapter_two =
            fs::read_to_string(out_dir.path().join("chapter_2.txt")).expect("read chapter 2");

        assert!(chapter_one.starts_with("First\n\n1 Verse 1 | Ref 1\n"));
        assert!(chapter_one.contains("40 Verse 40 | Ref 40\n"));
        assert_eq!(
            chapter_two,
            "Second\n\n1 Verse 41 | Ref 41\n2 Verse 42 | Ref 42\n"
        );
        assert!(!out_dir.path().join("chapter_99.txt").exists());
        assert_eq!(
            fs::read_to_string(out_dir.path().join("notes.txt")).expect("read unrelated file"),
            "keep"
        );
    }

    fn make_verses(
        numbers: std::ops::RangeInclusive<i64>,
        pericopes: &[&str],
        groups: &[std::ops::RangeInclusive<i64>],
    ) -> Vec<DbVerse> {
        let mut verses = Vec::new();
        for number in numbers {
            let pericope = groups
                .iter()
                .enumerate()
                .find(|(_, range)| range.contains(&number))
                .and_then(|(index, _)| pericopes.get(index).copied())
                .map(str::to_string);
            verses.push(DbVerse {
                number,
                pericope,
                text: format!("Verse {number}"),
                supporting_references: vec![format!("Ref {number}")],
                notes: None,
            });
        }
        verses
    }

    fn create_tables(conn: &Connection, tables: &DbTables) -> Result<()> {
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS \"sys\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS \"quarantine\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS \"sys_metrics\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS \"sys_traces\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS \"{main}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL, ts BLOB NOT NULL) WITHOUT ROWID;",
            main = tables.main,
        ))?;
        Ok(())
    }

    fn seed_verse(
        conn: &Connection,
        tables: &DbTables,
        uid: u64,
        number: i64,
        pericope: Option<&str>,
        references: &[&str],
    ) -> Result<()> {
        let ts = [0u8; 16];
        insert_field(
            conn,
            &tables.main,
            uid,
            "_type",
            &Value::String(VERSE_TYPE.to_string()),
            &ts,
        )?;
        insert_field(
            conn,
            &tables.main,
            uid,
            "number",
            &Value::Number(number.into()),
            &ts,
        )?;
        insert_field(
            conn,
            &tables.main,
            uid,
            "pericope",
            &match pericope {
                Some(value) => Value::String(value.to_string()),
                None => Value::Null,
            },
            &ts,
        )?;
        insert_field(
            conn,
            &tables.main,
            uid,
            "text",
            &Value::String(format!("Verse {number}")),
            &ts,
        )?;

        let mut support_ids = Vec::new();
        for (index, reference) in references.iter().enumerate() {
            let support_uid = uid * 100 + index as u64 + 1;
            support_ids.push(Value::String(support_uid.to_string()));
            insert_field(
                conn,
                &tables.main,
                support_uid,
                "display",
                &Value::String((*reference).to_string()),
                &ts,
            )?;
        }

        insert_field(
            conn,
            &tables.main,
            uid,
            SUPPORT_FIELD,
            &Value::Array(support_ids),
            &ts,
        )?;

        Ok(())
    }

    fn insert_field(
        conn: &Connection,
        table: &str,
        uid: u64,
        field: &str,
        value: &Value,
        ts: &[u8; 16],
    ) -> Result<()> {
        let raw = serde_json::to_vec(value)?;
        let mut value_with_ts = Vec::with_capacity(16 + raw.len());
        value_with_ts.extend_from_slice(ts);
        value_with_ts.extend_from_slice(&raw);
        conn.execute(
            &format!(
                "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3)",
                table
            ),
            params![encode_data_key(uid, field), value_with_ts, &ts[..]],
        )?;
        Ok(())
    }

    fn encode_data_key(uid: u64, predicate: &str) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 8 + predicate.len());
        buf.push(0x01);
        buf.extend_from_slice(&uid.to_be_bytes());
        buf.extend_from_slice(predicate.as_bytes());
        buf
    }
}
