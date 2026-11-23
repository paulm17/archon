use archon_scripture::{DATABASE_FILENAME, generate_jsonl, load_database};
use serde_json::{Value, json};
use std::fs;

#[test]
fn exports_legacy_records_and_retries_failed_load_without_overwriting() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    let input = source.join("Gen/NIV/chapter_1.json");
    let ingest = temp.path().join("INGEST");
    let storage = temp.path().join("storage");
    let database = storage.join(DATABASE_FILENAME);

    fs::create_dir_all(input.parent().unwrap())?;
    fs::write(&input, b"{")?;
    assert!(generate_jsonl(&source, &ingest).is_err());
    assert!(load_database(&ingest, &storage).is_err());
    assert!(!database.exists());

    fs::write(
        &input,
        serde_json::to_vec(&json!({
            "translation_code": "NIV",
            "translation_name": "New International Version",
            "book_code": "Gen",
            "book_name": "Genesis",
            "chapter": 1,
            "verses": [{
                "verse": 1,
                "header": "The Beginning",
                "paragraph_start": true,
                "content": [{"type": "text", "text": "God created the heavens."}]
            }]
        }))?,
    )?;
    generate_jsonl(&source, &ingest)?;
    let verse: Value = serde_json::from_str(&fs::read_to_string(ingest.join("verses.jsonl"))?)?;
    assert_eq!(verse["id"], "verse::Gen::NIV::1::1");
    assert_eq!(verse["chapter_id"], "chapter::Gen::NIV::1");
    assert_eq!(verse["chapter_heading"], "The Beginning");
    let contents = ingest.join("verse_contents.jsonl");
    let original = fs::read(&contents)?;
    let record: Value = serde_json::from_slice(&original)?;
    assert_eq!(record["verse_id"], verse["id"]);
    assert_eq!(record["translation_id"], "translation::NIV");
    assert_eq!(record["text"], "God created the heavens.");

    fs::write(&contents, b"{")?;
    assert!(load_database(&ingest, &storage).is_err());
    assert!(!database.exists());
    fs::write(&contents, original)?;

    assert!(load_database(&ingest, &storage)? > 0);
    let conn = rusqlite::Connection::open(&database)?;
    let matches: u64 = conn.query_row(
        "SELECT count(*) FROM fts_data WHERE text_content MATCH 'heavens' AND field = 'text.fulltext'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(matches, 2); // VerseContent and Chunk both index the verse text.
    assert!(load_database(&ingest, &storage).is_err());
    Ok(())
}

#[test]
fn interlinear_chapter_filenames_and_repeated_verse_numbers_have_distinct_ids() -> anyhow::Result<()>
{
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    let chapters = source.join("en/BIBLEHUB/1CH");
    let ingest = temp.path().join("INGEST");
    fs::create_dir_all(&chapters)?;

    for (number, texts) in [(1, &["Adam"][..]), (2, &["Israel", "Alternate text"][..])] {
        let verses: Vec<_> = texts
            .iter()
            .map(|text| {
                json!({
                    "verse": 1,
                    "words": [{"english": text}]
                })
            })
            .collect();
        fs::write(
            chapters.join(format!("chapter_{number}.json")),
            serde_json::to_vec(&json!({
                "translation_code": "interlinear",
                "translation_name": "Interlinear",
                "book_code": "1Chr",
                "book_name": "1 Chronicles",
                "chapter": 1,
                "verses": verses
            }))?,
        )?;
    }

    generate_jsonl(&source, &ingest)?;
    let lines = fs::read_to_string(ingest.join("verses.jsonl"))?;
    let ids: Vec<_> = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).map(|record| record["id"].clone()))
        .collect::<Result<_, _>>()?;
    assert_eq!(
        ids,
        [
            json!("verse::1Chr::interlinear::1::1"),
            json!("verse::1Chr::interlinear::2::1"),
            json!("verse::1Chr::interlinear::2::1::2")
        ]
    );
    assert!(load_database(&ingest, temp.path())? > 0);
    Ok(())
}
