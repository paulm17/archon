//! Hosts that are never content: page analytics, ads, and Chromium's own
//! telemetry.
//!
//! With a proxy configured Chromium does not resolve a hostname itself — it
//! sends `CONNECT host:port` and lets the proxy do the lookup. `--host-resolver-rules`
//! therefore cannot keep these requests off the wire, and each one that tunnels
//! out and times out used to count as a failed attempt against a region that was
//! working perfectly well for the chapter being scraped. A live run had those
//! beacons retire all 17 regions before the first chapter finished.
//!
//! Refusing them here is cheap and target-agnostic: no tunnel is opened, no
//! region is blamed, and the browser gets an immediate failure for a request it
//! never needed.

/// Domains whose every subdomain is telemetry or advertising.
const NOISE_SUFFIXES: &[&str] = &[
    "cloudflareinsights.com",
    "google.com",
    "googleapis.com",
    "gstatic.com",
    "google-analytics.com",
    "googletagmanager.com",
    "googleadservices.com",
    "googlesyndication.com",
    "doubleclick.net",
];

/// Whether this host is browser telemetry rather than something worth fetching.
pub fn is_noise(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    NOISE_SUFFIXES.iter().any(|suffix| {
        host == *suffix
            || (host.len() > suffix.len()
                && host.ends_with(suffix)
                && host.as_bytes()[host.len() - suffix.len() - 1] == b'.')
    })
}

#[cfg(test)]
mod tests {
    use super::is_noise;

    #[test]
    fn analytics_and_telemetry_hosts_are_noise() {
        for host in [
            "static.cloudflareinsights.com",
            "www.google.com",
            "accounts.google.com",
            "ogads-pa.clients6.google.com",
            "connectivitycheck.gstatic.com",
            "update.googleapis.com",
            "optimizationguide-pa.googleapis.com",
            "ssl.google-analytics.com",
            "www.googletagmanager.com",
        ] {
            assert!(is_noise(host), "{host} should be refused");
        }
    }

    #[test]
    fn content_hosts_are_not_noise() {
        for host in [
            "freely-given.org",
            "www.bible.com",
            "biblegateway.com",
            "biblehub.com",
            "studybible.info",
            "openenglishbible.org",
            "notgoogle.com",
            "google.com.example.test",
        ] {
            assert!(!is_noise(host), "{host} must still be fetched");
        }
    }

    #[test]
    fn matching_ignores_case_and_a_trailing_dot() {
        assert!(is_noise("WWW.Google.COM."));
        assert!(!is_noise("Freely-Given.ORG"));
    }
}
