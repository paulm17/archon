use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, ensure};
use rusqlite::{Connection, OpenFlags};
use serde::de::{Error, MapAccess, Visitor};
use serde_json::Value;
use vardadb::bridge::sqlite_resolver::SqliteResolver;
use vardadb::realtime::bus::MutationSource;
use vardadb::storage::backend::Storage;

use super::export::{default_db_dir, load_fields};

const DB: &str = "new_gospel";
const VERSE: &str = "NewGospelVerse";
const MARKS: &str = "assembledVerseMarks";

#[derive(clap::Subcommand)]
pub enum Command {
    /// Replace marks on the verses listed in a JSON file (an empty array clears a verse)
    Import(FileArgs),
    /// Export saved marks to a new JSON file keyed by verse number
    Export(FileArgs),
}

#[derive(clap::Args)]
pub struct FileArgs {
    /// Override the JSON file set in config.toml
    file: Option<PathBuf>,
    /// Directory containing new_gospel.db
    #[arg(long, default_value_os_t = default_db_dir())]
    db_dir: PathBuf,
}

#[derive(serde::Deserialize)]
struct Backup(#[serde(deserialize_with = "read_unique_verses")] BTreeMap<i64, Vec<Value>>);

fn read_unique_verses<'de, D>(deserializer: D) -> Result<BTreeMap<i64, Vec<Value>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Verses;

    impl<'de> Visitor<'de> for Verses {
        type Value = BTreeMap<i64, Vec<Value>>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a JSON object mapping verse numbers to marks arrays")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<Self::Value, A::Error> {
            let mut verses = BTreeMap::new();
            while let Some((number, marks)) = entries.next_entry()? {
                if verses.insert(number, marks).is_some() {
                    return Err(A::Error::custom(format!("duplicate verse {number}")));
                }
            }
            Ok(verses)
        }
    }

    deserializer.deserialize_map(Verses)
}

struct SavedVerse {
    uid: u64,
    raw_marks: Option<Value>,
}

fn read_verses(db_path: &Path) -> Result<BTreeMap<i64, SavedVerse>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {}", db_path.display()))?;
    let mut verses = BTreeMap::new();
    for (uid, mut fields) in load_fields(&conn, "new_gospel_main")? {
        if fields.get("_type").and_then(Value::as_str) != Some(VERSE) {
            continue;
        }
        let number = fields
            .get("number")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("missing verse number for uid {uid}"))?;
        let verse = SavedVerse {
            uid,
            raw_marks: fields.remove(MARKS).filter(|value| !value.is_null()),
        };
        ensure!(
            verses.insert(number, verse).is_none(),
            "duplicate verse {number} in database"
        );
    }
    Ok(verses)
}

fn validate_marks(number: i64, marks: &[Value]) -> Result<()> {
    for (index, mark) in marks.iter().enumerate() {
        let strings = ["id", "rowKey", "translation", "reference", "text"];
        let valid_range = mark["start"]
            .as_u64()
            .zip(mark["end"].as_u64())
            .is_some_and(|(start, end)| end > start && end <= 9_007_199_254_740_991);
        ensure!(
            strings.iter().all(|field| mark[*field].is_string())
                && valid_range
                && mark.get("cleanText").is_none_or(Value::is_boolean),
            "invalid mark {} for verse {number}",
            index + 1
        );
    }
    Ok(())
}

fn resolve_file(file: Option<PathBuf>, config_key: &str) -> Result<PathBuf> {
    if let Some(file) = file {
        return Ok(file);
    }
    super::configured_path(
        &super::read_settings()?,
        &["new-gospel", "marks", config_key],
    )?
    .with_context(|| {
        format!("Set new-gospel.marks.{config_key} in config.toml or pass a JSON file")
    })
}

fn export(args: FileArgs) -> Result<()> {
    let file = resolve_file(args.file, "export_file")?;
    let verses = read_verses(&args.db_dir.join("new_gospel.db"))?;
    let mut backup = BTreeMap::new();
    for (number, verse) in verses {
        if let Some(raw) = verse.raw_marks {
            let marks: Vec<Value> =
                serde_json::from_str(raw.as_str().context("expected a saved marks string")?)
                    .with_context(|| format!("decoding saved marks for verse {number}"))?;
            validate_marks(number, &marks)?;
            backup.insert(number, marks);
        }
    }

    let parent = file.parent().filter(|path| !path.as_os_str().is_empty());
    let mut output = tempfile::NamedTempFile::new_in(parent.unwrap_or_else(|| Path::new(".")))?;
    serde_json::to_writer_pretty(&mut output, &backup)?;
    writeln!(output)?;
    output
        .persist_noclobber(&file)
        .with_context(|| format!("creating {}", file.display()))?;
    println!(
        "Exported marks for {} verse(s) to {}",
        backup.len(),
        file.display()
    );
    Ok(())
}

fn import(args: FileArgs) -> Result<()> {
    let file = resolve_file(args.file, "import_file")?;
    let input = File::open(&file).with_context(|| format!("opening {}", file.display()))?;
    let Backup(backup) = serde_json::from_reader(BufReader::new(input))
        .with_context(|| format!("reading {}", file.display()))?;
    let db_path = args.db_dir.join("new_gospel.db").canonicalize()?;
    let verses = read_verses(&db_path)?;
    for (&number, marks) in &backup {
        ensure!(
            verses.contains_key(&number),
            "verse {number} does not exist"
        );
        validate_marks(number, marks)?;
    }

    if !backup.is_empty() {
        // Register only the target database in a private, temporary Varda registry.
        let registry = tempfile::tempdir()?;
        let storage = Arc::new(Storage::new(registry.path(), None)?);
        let db_path = db_path
            .to_str()
            .context("database path must be valid UTF-8")?;
        storage.create_database_with_path(DB, Some(db_path.to_owned()))?;
        let resolver = SqliteResolver::new(storage, DB);

        for (&number, marks) in &backup {
            let value = Value::String(serde_json::to_string(marks)?);
            let verse = &verses[&number];
            if verse.raw_marks.as_ref() == Some(&value) {
                continue;
            }
            resolver
                .update_node_internal(
                    VERSE,
                    verse.uid,
                    HashMap::from([(MARKS.to_owned(), value)]),
                    &[],
                    &[],
                    &HashMap::new(),
                    MutationSource::Local,
                    None,
                )
                .map_err(|error| anyhow!("updating marks for verse {number}: {error}"))?;
        }
    }

    println!(
        "Imported marks for {} verse(s) from {}",
        backup.len(),
        file.display()
    );
    Ok(())
}

pub fn run(command: Command) -> Result<()> {
    match command {
        Command::Import(args) => import(args),
        Command::Export(args) => export(args),
    }
}
