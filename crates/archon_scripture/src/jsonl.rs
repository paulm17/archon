use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

/// Write a single JSONL record to a buffered writer.
pub fn write_record<W: Write, T: Serialize>(writer: &mut W, record: &T) -> Result<()> {
    let line = serde_json::to_string(record)?;
    writer.write_all(line.as_bytes())?;
    writer.write_all(b"\n")?;
    Ok(())
}

/// Create a buffered writer for a JSONL output file.
/// Creates parent directories as needed.
pub fn open_writer(path: &Path) -> Result<BufWriter<fs::File>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    let file =
        fs::File::create(path).with_context(|| format!("Failed to create {}", path.display()))?;
    Ok(BufWriter::new(file))
}
