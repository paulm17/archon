//! Index creation for VardaDB archondb expression indexes.
//!
//! Creates partial expression indexes on JSON fields that are queried via
//! VardaDB's `filter_by_field_value` method. These indexes allow SQLite
//! to use index seeks instead of full table scans.
//!
//! VardaDB Key Encoding (from codec.rs):
//!   Data key:    [0x01][UID:8 BE][field_name]
//!   Value:       [timestamp:16 bytes][json_payload]
//!
//! Expression index must match `filter_by_field_value` query structure exactly
//! for SQLite to use them: `json_extract(CAST(substr(value, 17) AS TEXT), '$.field')`

use rusqlite::Connection;
use std::path::Path;

const MAIN_TABLE: &str = "archondb_main";

/// Fields from schema.graphql that need expression indexes.
/// Each entry: (type_name, field_name, graphql_field_name, key_length)
/// key_length = 1 (prefix) + 8 (UID) + field_name.len()
struct IndexDef {
    type_name: &'static str,
    field_name: &'static str,
    key_length: usize,
}

impl IndexDef {
    fn new(type_name: &'static str, field_name: &'static str) -> Self {
        Self {
            type_name,
            field_name,
            key_length: 1 + 8 + field_name.len(),
        }
    }

    fn index_name(&self) -> String {
        format!(
            "idx_{}_{}",
            self.type_name.to_lowercase(),
            self.field_name.to_lowercase()
        )
    }

    fn create_sql(&self) -> String {
        format!(
            "CREATE INDEX IF NOT EXISTS {} ON {} (
                json_extract(CAST(substr(value, 17) AS TEXT), '$.{}')
            ) WHERE substr(key, 1, 1) = 0x01 AND length(key) = {};",
            self.index_name(),
            MAIN_TABLE,
            self.field_name,
            self.key_length
        )
    }
}

/// Create all expression indexes for archondb fields marked with @search in schema.
///
/// Call this after `create_tables()` and after all data has been ingested.
pub fn create_indexes(db_path: &Path) -> anyhow::Result<()> {
    eprintln!(
        "\n📇 Opening database for index creation: {}",
        db_path.display()
    );
    create_on_connection(&Connection::open(db_path)?)
}

pub(crate) fn create_on_connection(conn: &Connection) -> anyhow::Result<()> {
    eprintln!("📇 Creating expression indexes...");

    // Fields from schema.graphql with @search directives that need expression indexes
    let indexes = vec![
        // Chapter.number: @search(by: [int])
        IndexDef::new("Chapter", "number"),
        // Verse fields with @search
        IndexDef::new("Verse", "number"),
        IndexDef::new("Verse", "logionNumber"),
        IndexDef::new("Verse", "chunkType"),
        IndexDef::new("Verse", "chapterHeading"),
        IndexDef::new("Verse", "sectionHeading"),
        // Chunk fields
        IndexDef::new("Chunk", "chunkType"),
        // Other frequently queried fields
        IndexDef::new("VerseContent", "type"),
    ];

    for index in indexes {
        conn.execute_batch(&index.create_sql())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_def_key_length() {
        let idx = IndexDef::new("Verse", "number");
        assert_eq!(idx.key_length, 15); // 1 + 8 + 6
        assert_eq!(idx.index_name(), "idx_verse_number");
    }

    #[test]
    fn test_index_def_chunk_type() {
        let idx = IndexDef::new("Verse", "chunkType");
        assert_eq!(idx.key_length, 18); // 1 + 8 + 9
        assert_eq!(idx.index_name(), "idx_verse_chunktype");
    }

    #[test]
    fn test_create_sql() {
        let idx = IndexDef::new("Verse", "number");
        let sql = idx.create_sql();
        assert!(sql.contains("json_extract(CAST(substr(value, 17) AS TEXT), '$.number')"));
        assert!(sql.contains("WHERE substr(key, 1, 1) = 0x01 AND length(key) = 15"));
    }
}
