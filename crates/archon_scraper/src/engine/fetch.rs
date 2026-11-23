//! Load a chapter page and wait until its content is complete and stable.

use anyhow::{Context, Result, anyhow, bail};
use chromiumoxide::Page;
use std::time::{Duration, Instant};

use crate::config::Timing;
use crate::sites::{ChapterRef, Site};

/// A failure retrying cannot fix: the site answered with a permanent HTTP error
/// page (404/410) instead of the chapter, so another attempt would only wait for
/// the same page again.
#[derive(Debug)]
pub struct PermanentFailure(pub String);

impl std::fmt::Display for PermanentFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PermanentFailure {}

/// `404 Not Found` → 404. Only statuses that will never change with a retry.
/// Chrome puts the status of an error page in the document title, which is the
/// only place a plain navigation exposes it.
fn permanent_status(title: &str) -> Option<u16> {
    let status: u16 = title.split_whitespace().next()?.parse().ok()?;
    matches!(status, 404 | 410).then_some(status)
}

#[derive(Debug, Clone, Copy)]
pub struct FetchTiming {
    pub page_load: Duration,
    pub navigation: Duration,
    pub poll: Duration,
    /// Consecutive polls with an unchanged verse count before a page is trusted.
    pub stable_polls: usize,
}

impl FetchTiming {
    pub fn from_config(timing: &Timing) -> Self {
        Self {
            page_load: Duration::from_millis(timing.page_load),
            navigation: Duration::from_millis(timing.navigation),
            poll: Duration::from_millis(timing.element_click),
            stable_polls: 3,
        }
    }
}

/// Navigate to `url`, then poll until the site's content selector is present,
/// the site's own validator accepts the page, and the verse count has stopped
/// changing for `stable_polls` consecutive polls.
pub async fn load_chapter(
    page: &Page,
    url: &str,
    ready: &str,
    site: &dyn Site,
    chapter: &ChapterRef<'_>,
    timing: &FetchTiming,
) -> Result<String> {
    // A stalled proxy or a page that never fires its load event would
    // otherwise block here forever: the navigation gets the same deadline as
    // the content wait below.
    tokio::time::timeout(timing.navigation, page.goto(url))
        .await
        .map_err(|_| anyhow!("Timed out navigating to {url}"))?
        .with_context(|| format!("Failed to navigate to {url}"))?;

    tokio::time::sleep(timing.page_load).await;

    // A permanent error page is not the chapter and never will be. Waiting the
    // whole content deadline for it — and then retrying it — is how one missing
    // chapter used to cost ten minutes of a run.
    if let Ok(Some(title)) = page.get_title().await
        && let Some(status) = permanent_status(&title)
    {
        return Err(anyhow::Error::new(PermanentFailure(format!(
            "site answered HTTP {status} for {url} ({title})"
        ))));
    }

    let deadline = Instant::now() + timing.navigation;
    let mut previous_count = usize::MAX;
    let mut stable = 0usize;

    loop {
        let ready_found = if ready.trim().is_empty() {
            true
        } else {
            page.find_element(ready).await.is_ok()
        };
        let html = tokio::time::timeout(timing.navigation, page.content())
            .await
            .map_err(|_| anyhow!("Timed out reading the page at {url}"))?
            .context("Failed to read page HTML")?;

        if ready_found && site.validate(&html, chapter).is_ok() {
            let count = site.verse_count(&html, chapter);
            if count == previous_count {
                stable += 1;
            } else {
                stable = 1;
                previous_count = count;
            }
            if stable >= timing.stable_polls.max(1) {
                return Ok(html);
            }
        } else {
            stable = 0;
            previous_count = usize::MAX;
        }

        if Instant::now() >= deadline {
            bail!(
                "Timed out waiting for complete content at {url} \
                 (selector_found={ready_found}, verses_found={})",
                site.verse_count(&html, chapter)
            );
        }
        tokio::time::sleep(timing.poll).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_permanent_error_titles() {
        assert_eq!(permanent_status("404 Not Found"), Some(404));
        assert_eq!(permanent_status("410 Gone"), Some(410));
        // A transient failure must still be retried, and a real page title is
        // not a status at all.
        assert_eq!(permanent_status("503 Service Unavailable"), None);
        assert_eq!(permanent_status("Genesis 1 | T4T"), None);
        assert_eq!(permanent_status(""), None);
    }

    #[test]
    fn timing_is_converted_from_config() {
        let timing = FetchTiming::from_config(&Timing {
            page_load: 1500,
            navigation: 20000,
            element_click: 250,
        });
        assert_eq!(timing.page_load, Duration::from_millis(1500));
        assert_eq!(timing.navigation, Duration::from_millis(20000));
        assert_eq!(timing.poll, Duration::from_millis(250));
        assert_eq!(timing.stable_polls, 3);
    }
}
