pub mod catalog;
pub mod config;
pub mod model;
pub mod sites;
pub mod engine {
    pub mod store;
}

use anyhow::Result;
use std::path::{Path, PathBuf};

pub const CATALOG: &str = include_str!("../catalog.toml");
pub fn resolve_path(root: &Path, value: &str) -> PathBuf {
    root.join(value)
}

pub fn settings() -> Result<(config::Config, catalog::Catalog)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = config::Config::load(&root.join("config.toml"))?;
    let catalog = match &config.catalog.path {
        Some(path) => catalog::Catalog::load(&root.join(path))?,
        None => catalog::Catalog::from_str(CATALOG)?,
    };
    Ok((config, catalog))
}
