use anyhow::Result;
use rusqlite::Connection;

pub const FTS_DATA_TABLE: &str = "fts_data";
pub const FTS_TERM_TABLE: &str = "fts_term_data";
/// Create VardaDB's two native FTS5 tables.
pub fn create_fts_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS {FTS_DATA_TABLE} \
             USING fts5(uid UNINDEXED, field UNINDEXED, text_content, tokenize='porter unicode61');
         CREATE VIRTUAL TABLE IF NOT EXISTS {FTS_TERM_TABLE} \
             USING fts5(uid UNINDEXED, field UNINDEXED, text_content, tokenize='unicode61');"
    ))?;
    Ok(())
}
