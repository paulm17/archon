//! Convert legacy chapter JSON to scripture JSONL and bulk-load it into VardaDB SQLite.

mod codec;
mod export;
mod fts_sqlite;
mod ids;
mod index;
mod ingest;
mod jsonl;
mod progress;
mod schema;
mod scripture_contract;

use anyhow::{Context, Result, ensure};
use std::fs::{self, File, OpenOptions};
use std::path::Path;

pub const DATABASE_NAME: &str = "archondb";
pub const DATABASE_FILENAME: &str = "archondb.db";

/// Generate the legacy scripture JSONL files from the current or legacy chapter tree.
pub fn generate_jsonl(translations_dir: &Path, ingest_dir: &Path) -> Result<()> {
    let _lock = lock_ingest_dir(ingest_dir)?;
    generate_locked(translations_dir, ingest_dir)
}

/// Run both steps while reserving the JSONL directory for this import.
pub fn import(translations_dir: &Path, ingest_dir: &Path, storage_dir: &Path) -> Result<u64> {
    let destination = storage_dir.join(DATABASE_FILENAME);
    let _lock = lock_ingest_dir(ingest_dir)?;
    generate_locked(translations_dir, ingest_dir)?;
    load_locked(ingest_dir, &destination)
}

fn generate_locked(translations_dir: &Path, ingest_dir: &Path) -> Result<()> {
    let staging = tempfile::tempdir_in(ingest_dir)?;
    export::export(translations_dir, staging.path())?;

    for filename in scripture_contract::OUTPUT_FILES {
        File::open(staging.path().join(filename))?.sync_all()?;
    }

    let marker = scripture_contract::marker_path(ingest_dir);
    match fs::remove_file(&marker) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    for filename in scripture_contract::OUTPUT_FILES {
        fs::rename(staging.path().join(filename), ingest_dir.join(filename))?;
    }
    scripture_contract::publish(ingest_dir, scripture_contract::OUTPUT_FILES, &[])
}

/// Load a completed export into a new or empty archondb.db and return the imported row count.
pub fn load_database(ingest_dir: &Path, storage_dir: &Path) -> Result<u64> {
    let destination = storage_dir.join(DATABASE_FILENAME);
    let _lock = lock_ingest_dir(ingest_dir)?;
    load_locked(ingest_dir, &destination)
}

fn load_locked(ingest_dir: &Path, destination: &Path) -> Result<u64> {
    ensure!(
        scripture_contract::is_valid(ingest_dir, scripture_contract::OUTPUT_FILES, &[]),
        "No complete scripture export at {}",
        ingest_dir.display()
    );

    if destination.exists() {
        return ingest::load_existing(ingest_dir, destination);
    }

    let storage_dir = destination
        .parent()
        .context("Database path has no parent")?;
    fs::create_dir_all(storage_dir)?;
    let staging = tempfile::tempdir_in(storage_dir)?;
    let database = staging.path().join(DATABASE_FILENAME);
    let rows = ingest::load(ingest_dir, &database)?;
    index::create_indexes(&database)?;
    File::open(&database)?.sync_all()?;
    // An atomic link publishes the completed file only if the destination is still free.
    fs::hard_link(&database, destination)
        .with_context(|| format!("Could not publish {}", destination.display()))?;
    Ok(rows)
}

fn lock_ingest_dir(ingest_dir: &Path) -> Result<File> {
    fs::create_dir_all(ingest_dir)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(ingest_dir.join(".scripture-import.lock"))?;
    lock.lock()?;
    Ok(lock)
}
