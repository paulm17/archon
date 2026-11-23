use archon_scripture::{DATABASE_NAME, generate_jsonl, load_database};
use std::{fs, sync::Arc};
use vardadb::{
    bridge::sqlite_resolver::SqliteResolver,
    engine::schema::Schema,
    storage::{backend::Storage, codec::Codec},
};

#[test]
fn varda_reads_and_searches_bulk_imported_scripture() -> anyhow::Result<()> {
    verify_import(false)?;
    verify_import(true)
}

fn verify_import(existing: bool) -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("source");
    let chapters = source.join("en/NIV/GEN");
    fs::create_dir_all(&chapters)?;
    fs::write(
        chapters.join("chapter_1.json"),
        r#"{
            "translation_code": "NIV",
            "translation_name": "New International Version",
            "book_code": "Gen",
            "book_name": "Genesis",
            "chapter": 1,
            "verses": [{
                "verse": 1,
                "content": [{"type": "text", "text": "God created the heavens."}]
            }]
        }"#,
    )?;
    let ingest = temp.path().join("INGEST");
    let database_dir = temp.path().join("db");
    generate_jsonl(&source, &ingest)?;
    fs::create_dir_all(&database_dir)?;
    let storage = Arc::new(Storage::new(&database_dir, None)?);
    if existing {
        storage.create_database(DATABASE_NAME)?;
    }
    load_database(&ingest, &database_dir)?;
    if !existing {
        storage.create_database(DATABASE_NAME)?;
    }
    let resolver = SqliteResolver::new(storage.clone(), DATABASE_NAME);
    Schema::load_with_resolver(include_str!("../schema/archon.graphql"), resolver.clone())
        .map_err(anyhow::Error::msg)?;
    let matches = resolver.search_text_bm25_for_type(
        "heavens",
        "text",
        "fulltext",
        "VerseContent",
        10,
        true,
    )?;
    assert_eq!(matches.len(), 1);

    let content_uid = matches[0].0;
    let read_field = |uid, field| -> anyhow::Result<serde_json::Value> {
        let key = Codec::encode_data_key(uid, field);
        let value = storage
            .get(DATABASE_NAME, &key)?
            .ok_or_else(|| anyhow::anyhow!("Missing {field} for UID {uid}"))?;
        Ok(serde_json::from_slice(&value[16..])?)
    };
    assert_eq!(read_field(content_uid, "text")?, "God created the heavens.");
    let verse = read_field(content_uid, "verse")?;
    let verse_uid = verse.as_str().unwrap().parse()?;
    assert_eq!(read_field(verse_uid, "number")?, 1);
    Ok(())
}
