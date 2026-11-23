//! Run accounting: what was saved, skipped and failed.

use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

use crate::engine::store::{Store, now};

#[derive(Debug, Default, Serialize)]
pub struct Report {
    pub started_at: u64,
    pub finished_at: u64,
    pub saved: usize,
    pub skipped: usize,
    pub failed: Vec<Failure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub url: String,
    pub path: String,
    pub error: String,
}

impl Report {
    pub fn new() -> Self {
        Self {
            started_at: now(),
            ..Self::default()
        }
    }

    pub fn record_saved(&mut self) {
        self.saved += 1;
    }

    pub fn record_skipped(&mut self) {
        self.skipped += 1;
    }

    pub fn record_failure(&mut self, url: impl Into<String>, path: impl Into<String>, error: impl std::fmt::Display) {
        self.failed.push(Failure {
            url: url.into(),
            path: path.into(),
            error: error.to_string(),
        });
    }

    pub fn finish(&mut self) {
        self.finished_at = now();
    }

    pub fn summary(&self) -> String {
        let elapsed = self.finished_at.saturating_sub(self.started_at);
        format!(
            "saved {}, skipped {}, failed {} in {}s",
            self.saved,
            self.skipped,
            self.failed.len(),
            elapsed
        )
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("Failed to encode the run report")
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        Store::save_atomic(path, self.to_json()?.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_summary() {
        let mut report = Report::new();
        report.record_saved();
        report.record_saved();
        report.record_skipped();
        report.record_failure("https://example.test/1", "/tmp/1.html", "boom");
        report.finish();

        assert_eq!(report.saved, 2);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.failed.len(), 1);
        assert!(report.summary().contains("saved 2, skipped 1, failed 1"));
    }

    #[test]
    fn serialises_failures() {
        let mut report = Report::new();
        report.record_failure("https://example.test/2", "/tmp/2.html", "timeout");
        report.finish();
        let json = report.to_json().unwrap();
        assert!(json.contains("\"url\": \"https://example.test/2\""));
        assert!(json.contains("\"error\": \"timeout\""));
    }
}
