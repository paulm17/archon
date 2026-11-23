#[path = "../notes/mod.rs"]
mod notes;

use std::path::Path;

#[tokio::main]
async fn main() {
    if std::env::args().len() != 1 {
        eprintln!("Usage: cargo run --bin notes-import");
        std::process::exit(1);
    }

    if let Err(error) = notes::importer::run(Path::new("config.toml"), None).await {
        eprintln!("notes-import failed: {error:#}");
        std::process::exit(1);
    }
}
