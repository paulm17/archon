use crate::codec::Codec;
use crate::fts_sqlite::create_fts_tables;
use crate::schema::{FtsStrategy, IngestFile};
use anyhow::{Context, Result};
use byteorder::ByteOrder;
use indicatif::{ProgressBar, ProgressStyle};
use rusqlite::Connection;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Instant;

fn page_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA page_count", [], |r| r.get(0))?)
}

const MAIN_TABLE: &str = "archondb_main";
const HISTORY_TABLE: &str = "archondb_history";

/// Global state maintained across all file ingestions.
pub struct IngestState {
    /// Maps JSONL string IDs to assigned numeric UIDs.
    pub id_map: HashMap<String, u64>,
    /// Monotonically incrementing UID counter.
    pub uid_counter: u64,
}

impl IngestState {
    pub fn new() -> Self {
        Self {
            id_map: HashMap::with_capacity(500_000),
            uid_counter: 1, // Start at 1 (0 is reserved)
        }
    }

    pub fn next_uid(&mut self) -> u64 {
        let uid = self.uid_counter;
        self.uid_counter += 1;
        uid
    }
}

fn write_list_inverse_edges(
    edge_stmt: &mut rusqlite::CachedStatement<'_>,
    record: &serde_json::Value,
    jsonl_field: &str,
    inverse_field: &str,
    uid: u64,
    id_map: &HashMap<String, u64>,
    ts_bytes: &[u8],
) -> Result<()> {
    let Some(values) = record.get(jsonl_field).and_then(|v| v.as_array()) else {
        return Ok(());
    };

    for value in values {
        let Some(ref_id) = value.as_str() else {
            continue;
        };
        let Some(&target_uid) = id_map.get(ref_id) else {
            continue;
        };

        let edge_key = Codec::encode_edge_key(target_uid, inverse_field, uid);
        let rev_key = Codec::encode_reverse_edge_key(uid, target_uid, inverse_field);
        edge_stmt.execute(rusqlite::params![edge_key, &[] as &[u8], ts_bytes])?;
        edge_stmt.execute(rusqlite::params![rev_key, &[] as &[u8], ts_bytes])?;
    }

    Ok(())
}

// ─── Progress tracking ───────────────────────────────────────────────

/// Open VardaDB SQLite database with performance PRAGMAs.
pub fn open_db(db_path: &Path) -> Result<Connection> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    eprintln!("Opening database: {}", db_path.display());

    let conn = Connection::open(db_path)?;

    // Fast ingest settings apply only to the disposable staging database.
    conn.execute_batch(
        "PRAGMA journal_mode = MEMORY;
         PRAGMA synchronous = OFF;
         PRAGMA mmap_size = 30000000000;
         PRAGMA cache_size = -2097152;
         PRAGMA temp_store = MEMORY;
         PRAGMA locking_mode = EXCLUSIVE;",
    )?;

    Ok(conn)
}

/// Create all required tables in the database.
pub fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS \"{}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL, ts BLOB NOT NULL) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS \"{}\" (key BLOB PRIMARY KEY, value BLOB NOT NULL) WITHOUT ROWID;",
        MAIN_TABLE, HISTORY_TABLE
    ))?;

    create_fts_tables(conn)?;

    eprintln!("Tables created");
    Ok(())
}

fn index_native_fts(
    conn: &Connection,
    file_def: &IngestFile,
    records: &[(u64, serde_json::Value)],
) -> Result<()> {
    if file_def.fts_fields.is_empty() {
        return Ok(());
    }

    {
        let mut fulltext = conn
            .prepare_cached("INSERT INTO fts_data(uid, field, text_content) VALUES (?1, ?2, ?3)")?;
        let mut term = conn.prepare_cached(
            "INSERT INTO fts_term_data(uid, field, text_content) VALUES (?1, ?2, ?3)",
        )?;

        for (uid, record) in records {
            for fts_field in &file_def.fts_fields {
                let jsonl_key = file_def
                    .field_map
                    .iter()
                    .find(|(_, gql)| *gql == fts_field.graphql_name)
                    .map(|(jk, _)| *jk)
                    .unwrap_or(fts_field.graphql_name);
                let Some(text) = record
                    .get(jsonl_key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| !text.is_empty())
                else {
                    continue;
                };

                for strategy in &fts_field.strategies {
                    match strategy {
                        FtsStrategy::Fulltext => fulltext.execute(rusqlite::params![
                            *uid as i64,
                            format!("{}.fulltext", fts_field.graphql_name),
                            text,
                        ])?,
                        FtsStrategy::Term => term.execute(rusqlite::params![
                            *uid as i64,
                            fts_field.graphql_name,
                            text,
                        ])?,
                    };
                }
            }
        }
    }
    Ok(())
}

// ─── Ingest ─────────────────────────────────────────────────────────

/// Ingest a single JSONL file into the database.
pub fn ingest_file(
    conn: &Connection,
    ingest_dir: &Path,
    file_def: &IngestFile,
    state: &mut IngestState,
    index_node_types: bool,
) -> Result<u64> {
    let file_path = ingest_dir.join(file_def.filename);
    // Count lines first for progress bar
    let line_count = BufReader::new(std::fs::File::open(&file_path)?)
        .lines()
        .count() as u64;

    if line_count == 0 {
        eprintln!("   ⚠️  {} is empty — skipping", file_def.filename);
        return Ok(0);
    }

    let pb = ProgressBar::new(line_count);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("   {msg} [{elapsed_precise}] [{bar:40}] {pos}/{len}")
            .unwrap()
            .progress_chars("█▉▊▋▌▍▎▏ "),
    );
    pb.set_message(file_def.filename.to_string());

    // Timestamp bytes — all zeros is fine for ingest
    let ts_bytes = [0u8; 16];

    // Phase 1: Read all records and assign UIDs
    let t_parse = Instant::now();
    let file = std::fs::File::open(&file_path)?;
    let reader = BufReader::new(file);
    let mut records: Vec<(u64, serde_json::Value)> = Vec::with_capacity(line_count as usize);

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            pb.inc(1);
            continue;
        }
        let record: serde_json::Value = serde_json::from_str(&line)
            .with_context(|| format!("Failed to parse line in {}", file_def.filename))?;

        let uid = state.next_uid();

        let id = record
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .with_context(|| format!("Missing record ID in {}", file_def.filename))?;
        anyhow::ensure!(
            state.id_map.insert(id.to_owned(), uid).is_none(),
            "Duplicate ID {id} in {}",
            file_def.filename
        );

        records.push((uid, record));
    }

    let parse_secs = t_parse.elapsed().as_secs_f64();

    let id_map = &state.id_map;
    let timing_summary = {
        // Phase 2: Write all records in a single transaction
        let pages_before = page_count(conn)?;
        let t_kv = Instant::now();
        let kv_secs;
        {
            let mut data_stmt = conn.prepare_cached(&format!(
                "INSERT INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, ts = excluded.ts",
                MAIN_TABLE
            ))?;
            let mut edge_stmt = conn.prepare_cached(&format!(
                "INSERT OR IGNORE INTO \"{}\" (key, value, ts) VALUES (?1, ?2, ?3)",
                MAIN_TABLE
            ))?;

            let mut node_type_stmt = if index_node_types {
                Some(conn.prepare_cached(
                    "INSERT OR REPLACE INTO node_types(uid, type_name) VALUES (?1, ?2)",
                )?)
            } else {
                None
            };
            for (uid, record) in &records {
                let uid = *uid;
                if let Some(stmt) = &mut node_type_stmt {
                    stmt.execute(rusqlite::params![uid as i64, file_def.type_name])?;
                }

                let type_idx_key = Codec::encode_type_index_key(file_def.type_name, uid);
                edge_stmt.execute(rusqlite::params![type_idx_key, &[] as &[u8], &ts_bytes[..]])?;

                let type_val =
                    serde_json::to_vec(&serde_json::Value::String(file_def.type_name.to_string()))?;
                let type_data_key = Codec::encode_data_key(uid, "_type");
                let mut type_buf = Vec::with_capacity(16 + type_val.len());
                type_buf.extend_from_slice(&ts_bytes);
                type_buf.extend_from_slice(&type_val);
                data_stmt.execute(rusqlite::params![type_data_key, type_buf, &ts_bytes[..]])?;

                for (jsonl_key, graphql_name) in &file_def.field_map {
                    if let Some(val) = record.get(*jsonl_key) {
                        let stored_val = if file_def.ref_fields.iter().any(|(f, _)| f == jsonl_key)
                        {
                            if let Some(ref_id) = val.as_str() {
                                if let Some(&target_uid) = id_map.get(ref_id) {
                                    serde_json::Value::String(target_uid.to_string())
                                } else {
                                    val.clone()
                                }
                            } else {
                                val.clone()
                            }
                        } else {
                            val.clone()
                        };

                        let val_bytes = serde_json::to_vec(&stored_val)?;
                        let key = Codec::encode_data_key(uid, graphql_name);
                        let mut val_buf = Vec::with_capacity(16 + val_bytes.len());
                        val_buf.extend_from_slice(&ts_bytes);
                        val_buf.extend_from_slice(&val_bytes);
                        data_stmt.execute(rusqlite::params![key, val_buf, &ts_bytes[..]])?;

                        if let Some(asc_value) = Codec::encode_order_index_value(&stored_val, false)
                        {
                            let asc_key = Codec::encode_order_index_key(
                                file_def.type_name,
                                graphql_name,
                                false,
                                &asc_value,
                                uid,
                            );
                            edge_stmt.execute(rusqlite::params![
                                asc_key,
                                &[] as &[u8],
                                &ts_bytes[..]
                            ])?;

                            if let Some(desc_value) =
                                Codec::encode_order_index_value(&stored_val, true)
                            {
                                let desc_key = Codec::encode_order_index_key(
                                    file_def.type_name,
                                    graphql_name,
                                    true,
                                    &desc_value,
                                    uid,
                                );
                                edge_stmt.execute(rusqlite::params![
                                    desc_key,
                                    &[] as &[u8],
                                    &ts_bytes[..]
                                ])?;
                            }
                        }
                    }
                }

                for unique_field in &file_def.unique_fields {
                    let graphql_name = file_def
                        .field_map
                        .iter()
                        .find(|(_, gql)| gql == unique_field)
                        .map(|(_, gql)| *gql)
                        .unwrap_or(unique_field);

                    if let Some(val) = record.get(graphql_name).or_else(|| {
                        file_def
                            .field_map
                            .iter()
                            .find(|(_, gql)| gql == unique_field)
                            .and_then(|(jk, _)| record.get(*jk))
                    }) {
                        let index_pred = format!("{}.{}", file_def.type_name, unique_field);
                        let val_str = serde_json::to_string(val)?;
                        let idx_key = Codec::encode_unique_index_key(&index_pred, &val_str);
                        let mut uid_bytes = vec![0u8; 8];
                        byteorder::BigEndian::write_u64(&mut uid_bytes, uid);
                        edge_stmt.execute(rusqlite::params![idx_key, uid_bytes, &ts_bytes[..]])?;
                    }
                }

                for inv in &file_def.inverses {
                    if let Some(val) = record.get(inv.jsonl_field) {
                        if let Some(ref_id) = val.as_str() {
                            if let Some(&target_uid) = id_map.get(ref_id) {
                                if inv.inverse_is_list {
                                    let edge_key =
                                        Codec::encode_edge_key(target_uid, inv.inverse_field, uid);
                                    let rev_key = Codec::encode_reverse_edge_key(
                                        uid,
                                        target_uid,
                                        inv.inverse_field,
                                    );
                                    edge_stmt.execute(rusqlite::params![
                                        edge_key,
                                        &[] as &[u8],
                                        &ts_bytes[..]
                                    ])?;
                                    edge_stmt.execute(rusqlite::params![
                                        rev_key,
                                        &[] as &[u8],
                                        &ts_bytes[..]
                                    ])?;
                                } else {
                                    let inv_val = serde_json::Value::String(uid.to_string());
                                    let inv_val_bytes = serde_json::to_vec(&inv_val)?;
                                    let inv_key =
                                        Codec::encode_data_key(target_uid, inv.inverse_field);
                                    let mut inv_buf = Vec::with_capacity(16 + inv_val_bytes.len());
                                    inv_buf.extend_from_slice(&ts_bytes);
                                    inv_buf.extend_from_slice(&inv_val_bytes);
                                    data_stmt.execute(rusqlite::params![
                                        inv_key,
                                        inv_buf,
                                        &ts_bytes[..]
                                    ])?;
                                }
                            }
                        }
                    }
                }

                if file_def.filename == "chunks.jsonl" {
                    write_list_inverse_edges(
                        &mut edge_stmt,
                        record,
                        "verse_ids",
                        "chunks",
                        uid,
                        id_map,
                        &ts_bytes[..],
                    )?;
                }

                pb.inc(1);
            }

            kv_secs = t_kv.elapsed().as_secs_f64();
        }
        index_native_fts(conn, file_def, &records)?;
        let pages_after = page_count(conn)?;

        format!(
            "   ⏱  parse {parse_secs:.1}s | kv {kv_secs:.1}s | +{} pages",
            (pages_after - pages_before).max(0),
        )
    };

    let count = records.len() as u64;
    pb.finish_with_message(format!("✅ {} ({} rows)", file_def.filename, count));
    eprintln!("{timing_summary}");

    Ok(count)
}

pub(crate) fn load(ingest_dir: &Path, db_path: &Path) -> Result<u64> {
    let conn = open_db(db_path)?;
    create_tables(&conn)?;
    let mut state = IngestState::new();
    let mut rows = 0;
    for file in crate::schema::ingest_files() {
        let tx = conn.unchecked_transaction()?;
        rows += ingest_file(&tx, ingest_dir, &file, &mut state, false)?;
        tx.commit()?;
    }
    Ok(rows)
}

/// Bulk-load into a provisioned VardaDB database in one rollback-safe transaction.
pub(crate) fn load_existing(ingest_dir: &Path, db_path: &Path) -> Result<u64> {
    let mut conn =
        Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let populated: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM archondb_main) OR EXISTS(SELECT 1 FROM archondb_history)",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!populated, "{} already contains data", db_path.display());

    let mut state = IngestState::new();
    let mut rows = 0;
    for file in crate::schema::ingest_files() {
        rows += ingest_file(&tx, ingest_dir, &file, &mut state, true)?;
    }
    // VardaDB may already have marked its index migrations complete on the empty database.
    tx.execute_batch(
        "INSERT OR REPLACE INTO fts_scope_data(rowid, uid, field, text_content)
         SELECT rowid, 'u' || uid, field, text_content FROM fts_data;
         INSERT OR REPLACE INTO fts_scope_term_data(rowid, uid, field, text_content)
         SELECT rowid, 'u' || uid, field, text_content FROM fts_term_data;",
    )?;
    crate::index::create_on_connection(&tx)?;
    tx.commit()?;
    Ok(rows)
}
