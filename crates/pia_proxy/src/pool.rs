//! Round-robin upstream pool that retires upstreams only after repeated failures.
//!
//! A single blip must never take a region (or the proxy) down, so an upstream is
//! retired only after `failures_before_retire` consecutive failures, and a
//! retired upstream is probed again once `revive_after` has passed. Retirement
//! only takes a region out of the *rotation*: the listener keeps accepting, and
//! when nothing is in rotation an incoming connection force-probes the upstream
//! that has been out longest.
//!
//! A pinned pool (`pinned`) is a one-region view onto the same state, so a
//! region's health is the same whether it is reached through the shared listener
//! or its own port.

use crate::error::{ProxyError, Result};
use crate::noise;
use crate::upstream::{Stream, Upstream};
use crate::Target;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Consecutive failures tolerated before an upstream leaves rotation.
const DEFAULT_FAILURES_BEFORE_RETIRE: u32 = 3;
/// How long a retired upstream stays out before it is probed again.
const DEFAULT_REVIVE_AFTER: Duration = Duration::from_secs(60);
/// Total time one client connection may spend looking for an upstream.
///
/// A blackholed tunnel or a hung SOCKS5 upstream never fails fast, it only runs
/// out the clock, and the browser navigation waiting on this connection has its
/// own deadline. The budget is shared between attempts, so failover across many
/// dead regions can never outlast the navigation that is waiting for it.
const DEFAULT_CONNECT_BUDGET: Duration = Duration::from_secs(12);
/// Never start an attempt with less time than this left in the budget.
const MIN_ATTEMPT_TIME: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Default)]
struct UpstreamState {
    /// Consecutive failures since the last success.
    strikes: u32,
    /// When it was retired, if it is currently out of rotation.
    retired_at: Option<Instant>,
}

/// Rotates across upstreams, retiring ones that keep failing.
pub struct Pool {
    upstreams: Vec<Arc<Upstream>>,
    cursor: AtomicUsize,
    state: Arc<Mutex<Vec<UpstreamState>>>,
    /// When set, this pool is a one-region view onto `state[index]`: the health
    /// of a pinned port's region is the health the shared pool sees.
    pin: Option<usize>,
    /// How many upstreams one client connection may try before giving up.
    max_attempts: usize,
    failures_before_retire: u32,
    revive_after: Duration,
    connect_budget: Duration,
    /// Whether the "nothing in rotation" warning has already been said, so it
    /// is a transition rather than one line per client connection.
    exhausted: AtomicBool,
}

impl Pool {
    pub fn new(upstreams: Vec<Arc<Upstream>>, max_attempts: usize) -> Result<Self> {
        if upstreams.is_empty() {
            return Err(ProxyError::NoUpstream(
                "pool needs at least one upstream".into(),
            ));
        }
        let len = upstreams.len();
        Ok(Self {
            upstreams,
            cursor: AtomicUsize::new(0),
            state: Arc::new(Mutex::new(vec![UpstreamState::default(); len])),
            pin: None,
            max_attempts: max_attempts.max(1),
            failures_before_retire: DEFAULT_FAILURES_BEFORE_RETIRE,
            revive_after: DEFAULT_REVIVE_AFTER,
            connect_budget: DEFAULT_CONNECT_BUDGET,
            exhausted: AtomicBool::new(false),
        })
    }

    /// A one-region view that shares this pool's health state for `index`.
    ///
    /// A pinned port exists to serve one region, but it must not judge that
    /// region on its own: with separate strikes a pin could retire a region the
    /// shared listener was still using (and the other way round), so the two
    /// disagreed and a page sent to the pin kept failing on a region the rest of
    /// the proxy considered fine.
    pub fn pinned(&self, index: usize) -> Result<Arc<Pool>> {
        let upstream = self
            .upstreams
            .get(index)
            .cloned()
            .ok_or_else(|| ProxyError::Config(format!("no upstream at index {index}")))?;
        Ok(Arc::new(Self {
            upstreams: vec![upstream],
            cursor: AtomicUsize::new(0),
            state: Arc::clone(&self.state),
            pin: Some(index),
            max_attempts: 1,
            failures_before_retire: self.failures_before_retire,
            revive_after: self.revive_after,
            connect_budget: self.connect_budget,
            exhausted: AtomicBool::new(false),
        }))
    }

    /// Where this pool's `index`-th upstream lives in the shared state.
    fn slot(&self, index: usize) -> usize {
        self.pin.unwrap_or(index)
    }

    /// Tune when an upstream leaves rotation and when it gets probed again.
    pub fn with_retirement(mut self, failures_before_retire: u32, revive_after: Duration) -> Self {
        self.failures_before_retire = failures_before_retire.max(1);
        self.revive_after = revive_after;
        self
    }

    /// Total time one client connection may spend looking for an upstream.
    pub fn with_connect_budget(mut self, budget: Duration) -> Self {
        self.connect_budget = budget.max(MIN_ATTEMPT_TIME * 2);
        self
    }

    pub fn len(&self) -> usize {
        self.upstreams.len()
    }

    pub fn is_empty(&self) -> bool {
        self.upstreams.is_empty()
    }

    /// Upstreams currently in rotation.
    pub fn live(&self) -> usize {
        let guard = self.state.lock().expect("upstream state poisoned");
        (0..self.upstreams.len())
            .filter(|index| guard[self.slot(*index)].retired_at.is_none())
            .count()
    }

    /// Names currently in rotation.
    pub fn names(&self) -> Vec<String> {
        let guard = self.state.lock().expect("upstream state poisoned");
        (0..self.upstreams.len())
            .filter(|index| guard[self.slot(*index)].retired_at.is_none())
            .map(|index| self.upstreams[index].name().to_string())
            .collect()
    }

    /// Names currently out of rotation.
    pub fn retired_names(&self) -> Vec<String> {
        let guard = self.state.lock().expect("upstream state poisoned");
        (0..self.upstreams.len())
            .filter(|index| guard[self.slot(*index)].retired_at.is_some())
            .map(|index| self.upstreams[index].name().to_string())
            .collect()
    }

    pub fn is_exhausted(&self) -> bool {
        self.live() == 0
    }

    /// Upstreams to try for one client connection, in order.
    ///
    /// Everything in rotation comes first, in round-robin order. Retired
    /// upstreams follow as forced probes, oldest retirement first: a pool with
    /// nothing in rotation must still answer, and that probe is what brings a
    /// recovered region back without needing a restart.
    fn candidate_order(&self, start: usize, limit: usize) -> Vec<(usize, bool)> {
        let total = self.upstreams.len();
        let mut due = Vec::new();
        let mut retired = Vec::new();
        {
            let guard = self.state.lock().expect("upstream state poisoned");
            for offset in 0..total {
                let index = (start + offset) % total;
                match guard[self.slot(index)].retired_at {
                    None => due.push(index),
                    Some(retired_at) => retired.push((index, retired_at.elapsed())),
                }
            }
        }
        // Longest out of rotation first.
        retired.sort_by_key(|(_, age)| std::cmp::Reverse(*age));

        let mut order: Vec<(usize, bool)> = due
            .into_iter()
            .take(limit)
            .map(|index| (index, false))
            .collect();
        if order.len() < limit {
            // With nothing in rotation every remaining slot probes a retired
            // upstream, oldest first.
            for (index, _) in retired.iter().take(limit - order.len()) {
                order.push((*index, true));
            }
        }
        order.truncate(limit.max(1));
        order
    }

    /// Count a failed attempt against an upstream, or note that it was the
    /// target's fault and must not count.
    fn penalise(&self, index: usize, target: &Target, failure: &ProxyError) {
        if retires_upstream(failure) {
            let strikes = self.record_failure(index);
            let threshold = self.failures_before_retire;
            if strikes == threshold {
                // Only the transition is worth a warning: a retired region keeps
                // failing until its next probe, and logging every one of those
                // failures buries the run's own output.
                warn!(
                    upstream = %self.upstreams[index].name(),
                    target = %target,
                    failures = strikes,
                    retry_in_s = self.revive_after.as_secs(),
                    remaining = self.live(),
                    error = %failure,
                    "upstream keeps failing; retired from rotation"
                );
            } else if strikes < threshold {
                debug!(
                    upstream = %self.upstreams[index].name(),
                    target = %target,
                    failures = strikes,
                    threshold,
                    error = %failure,
                    "upstream failed; staying in rotation"
                );
            }
        } else {
            debug!(
                upstream = %self.upstreams[index].name(),
                target = %target,
                error = %failure,
                "upstream could not reach the target; not counted against it"
            );
        }
    }

    /// Count a failure. Returns the new strike count.
    fn record_failure(&self, index: usize) -> u32 {
        let slot = self.slot(index);
        let mut guard = self.state.lock().expect("upstream state poisoned");
        let entry = &mut guard[slot];
        entry.strikes += 1;
        if entry.strikes >= self.failures_before_retire {
            // Re-arm the probe timer, so an upstream that keeps failing is not
            // retried on every single request.
            entry.retired_at = Some(Instant::now());
        }
        entry.strikes
    }

    fn record_success(&self, index: usize) -> bool {
        let slot = self.slot(index);
        let mut guard = self.state.lock().expect("upstream state poisoned");
        let entry = &mut guard[slot];
        let was_retired = entry.retired_at.is_some();
        entry.strikes = 0;
        entry.retired_at = None;
        was_retired
    }

    /// Connect to `target` through the next upstream in rotation.
    ///
    /// Returns the upstream name that answered, so callers can log the region.
    pub async fn connect(&self, target: &Target) -> Result<(String, Stream)> {
        let total = self.upstreams.len();
        if total == 0 {
            return Err(ProxyError::NoUpstream("pool has no upstreams".into()));
        }

        // A page's analytics beacon and Chromium's telemetry are not content.
        // Chromium sends the hostname to the proxy unresolved, so this is the
        // only place they can be stopped — and every one that tunnelled out and
        // timed out counted as a strike against a region that was serving the
        // chapter fine.
        if noise::is_noise(&target.host) {
            debug!(target = %target, "refused a telemetry host without using an upstream");
            return Err(ProxyError::Blocked(format!(
                "{target} is telemetry, not content; refused locally"
            )));
        }

        let start = self.cursor.fetch_add(1, Ordering::Relaxed) % total;
        let attempts = self.max_attempts.min(total).max(1);
        let order = self.candidate_order(start, attempts);
        let planned = order.len();
        let mut last_error: Option<ProxyError> = None;
        let deadline = Instant::now() + self.connect_budget;

        for (position, (index, forced)) in order.into_iter().enumerate() {
            let name = self.upstreams[index].name().to_string();
            if forced {
                debug!(
                    upstream = %name,
                    target = %target,
                    "nothing is in rotation; probing a retired upstream"
                );
            }

            // Share what is left of the budget with the attempts still to come,
            // so one silent upstream can neither hold the client connection nor
            // starve the regions behind it of their chance to answer.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining < MIN_ATTEMPT_TIME {
                last_error.get_or_insert_with(|| {
                    ProxyError::Tunnel(format!(
                        "connection to {target} ran out of its {:?} upstream budget",
                        self.connect_budget
                    ))
                });
                break;
            }
            let attempt_timeout = remaining / (planned - position) as u32;

            let outcome = tokio::time::timeout(
                attempt_timeout,
                self.upstreams[index].connect(target),
            )
            .await;

            let failure = match outcome {
                Ok(Ok(stream)) => {
                    if self.record_success(index) {
                        info!(
                            upstream = %name,
                            "upstream is answering again; back in rotation"
                        );
                    }
                    self.exhausted.store(false, Ordering::Relaxed);
                    return Ok((name, stream));
                }
                Ok(Err(failure)) => failure,
                Err(_) => ProxyError::Tunnel(format!(
                    "connection to {target} via {name} timed out after {:?}",
                    attempt_timeout
                )),
            };

            self.penalise(index, target, &failure);
            last_error = Some(failure);
        }

        if self.is_exhausted() && !self.exhausted.swap(true, Ordering::Relaxed) {
            warn!(
                "every upstream is out of rotation; the listener keeps accepting and probing them"
            );
        }

        Err(ProxyError::NoUpstream(format!(
            "no upstream could reach {target} after {planned} attempt(s) ({} of {} in rotation): {}",
            self.live(),
            self.upstreams.len(),
            last_error
                .map(|failure| failure.to_string())
                .unwrap_or_else(|| "nothing was tried".to_string())
        )))
    }
}

/// Whether a failure counts against the upstream, rather than being the target's
/// fault. A client asking for a host that does not resolve must not count
/// against the region it happened to be routed through.
fn retires_upstream(failure: &ProxyError) -> bool {
    match failure {
        // Account level: PIA rejected us or rate limited us.
        ProxyError::Pia(message) => {
            message.contains("429") || message.to_lowercase().contains("rate limit")
        }
        // Reaching the SOCKS5 proxy or authenticating to it is ours to fix; a
        // refusal from the far side ("CONNECT to ... refused") is the target's.
        ProxyError::Socks5(message) => !message.starts_with("SOCKS5 CONNECT to"),
        ProxyError::Tunnel(message) => tunnel_fault(message),
        ProxyError::Io(_) | ProxyError::Http(_) => true,
        ProxyError::Protocol(_) | ProxyError::Config(_) | ProxyError::NoUpstream(_) => false,
        // A refusal of our own must not be blamed on a region.
        ProxyError::Blocked(_) => false,
    }
}

/// Classify a tunnel error.
///
/// A dead tunnel blackholes packets, which shows up only as timeouts: DNS and
/// TCP connects never fail fast, they just run out the clock. Those must count
/// against the region or a silently broken tunnel is never retired. Only a
/// definitive answer from the far side ("refused or dropped") is the target's.
fn tunnel_fault(message: &str) -> bool {
    if message.contains("was refused or dropped") {
        return false;
    }
    message.contains("not running")
        || message.contains("dropped the request")
        || message.contains("cannot reach DNS resolver")
        || message.contains("DNS send failed")
        || message.contains("timed out")
        || message.contains("cannot start TCP connect")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_faults_do_not_count_against_an_upstream() {
        assert!(!retires_upstream(&ProxyError::Tunnel(
            "connection to 93.184.216.34:443 was refused or dropped".into()
        )));
        assert!(!retires_upstream(&ProxyError::Protocol(
            "DNS server returned error code 3".into()
        )));
        assert!(!retires_upstream(&ProxyError::Socks5(
            "SOCKS5 CONNECT to example.com:80 refused (reply code 5)".into()
        )));
    }

    #[test]
    fn a_tunnel_that_only_times_out_counts_against_it() {
        // A blackholed tunnel never errors, it just runs out the clock.
        assert!(retires_upstream(&ProxyError::Tunnel(
            "connection to 93.184.216.34:443 timed out".into()
        )));
        assert!(retires_upstream(&ProxyError::Tunnel(
            "timed out resolving example.com".into()
        )));
        assert!(retires_upstream(&ProxyError::Tunnel(
            "timed out sending DNS query for example.com".into()
        )));
        assert!(retires_upstream(&ProxyError::Tunnel(
            "cannot reach DNS resolver 10.0.0.242: connection to 10.0.0.242:53 timed out".into()
        )));
    }

    #[test]
    fn upstream_faults_count_against_an_upstream() {
        assert!(retires_upstream(&ProxyError::Pia(
            "addKey on x returned HTTP 429".into()
        )));
        assert!(retires_upstream(&ProxyError::Tunnel(
            "tunnel for region us_chicago is not running".into()
        )));
        assert!(retires_upstream(&ProxyError::Socks5(
            "cannot reach SOCKS5 proxy 1.2.3.4:1080: connection refused".into()
        )));
    }
}
