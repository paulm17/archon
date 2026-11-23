//! Crash-safe completion marker for the Scripture output family.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MARKER_FILE: &str = "scripture_output_contract.json";
pub const OUTPUT_FILES: &[&str] = &[
    "books.jsonl",
    "languages.jsonl",
    "translations.jsonl",
    "categories.jsonl",
    "book_translations.jsonl",
    "book_categories.jsonl",
    "chapters.jsonl",
    "verses.jsonl",
    "verse_contents.jsonl",
    "interlinear_tokens.jsonl",
    "chunks.jsonl",
];
const MARKER_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct OutputFamilyContract {
    pub version: u32,
    pub outputs: Vec<String>,
    pub greek_scopes: Vec<String>,
}

pub fn marker_path(ingest_dir: &Path) -> PathBuf {
    ingest_dir.join(MARKER_FILE)
}

/// A completed export contains every file. The token file can be empty.
pub fn is_valid(ingest_dir: &Path, outputs: &[&str], greek_scopes: &[&str]) -> bool {
    let path = marker_path(ingest_dir);
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok(contract) = serde_json::from_slice::<OutputFamilyContract>(&bytes) else {
        return false;
    };
    contract.version == MARKER_VERSION
        && contract.outputs
            == outputs
                .iter()
                .map(|output| (*output).to_string())
                .collect::<Vec<_>>()
        && contract.greek_scopes
            == greek_scopes
                .iter()
                .map(|scope| (*scope).to_string())
                .collect::<Vec<_>>()
        && outputs.iter().all(|output| {
            let path = ingest_dir.join(output);
            if !path.is_file() {
                return false;
            }
            *output == "interlinear_tokens.jsonl"
                || fs::metadata(path)
                    .map(|metadata| metadata.len() > 0)
                    .unwrap_or(false)
        })
}

/// Publish the marker with a same-directory rename so a crash exposes either
/// the previous marker or no marker, never a partial JSON document.
pub fn publish(ingest_dir: &Path, outputs: &[&str], greek_scopes: &[&str]) -> Result<()> {
    if !outputs
        .iter()
        .all(|output| ingest_dir.join(output).is_file())
    {
        bail!("cannot publish Scripture output-family contract before all artifacts are complete");
    }
    let path = marker_path(ingest_dir);
    let temporary = ingest_dir.join(format!(".{MARKER_FILE}.tmp-{}", std::process::id()));
    let contract = OutputFamilyContract {
        version: MARKER_VERSION,
        outputs: outputs.iter().map(|output| (*output).to_string()).collect(),
        greek_scopes: greek_scopes
            .iter()
            .map(|scope| (*scope).to_string())
            .collect(),
    };

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, &contract)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, &path)?;

    // Best-effort directory durability is intentionally kept inside the
    // atomic publication boundary; unsupported directory sync is harmless.
    if let Ok(directory) = File::open(ingest_dir) {
        let _ = directory.sync_all();
    }
    Ok(())
}
