//! Exponential backoff with jitter, and the retry loop every fetch goes through.

use anyhow::{Result, bail};
use rand::Rng;
use std::future::Future;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
    pub jitter_factor: f64,
}

impl RetryPolicy {
    pub fn new(
        max_attempts: u32,
        initial_delay_ms: u64,
        max_delay_ms: u64,
        backoff_multiplier: f64,
        jitter_factor: f64,
    ) -> Result<Self> {
        let policy = Self {
            max_attempts,
            initial_delay_ms,
            max_delay_ms,
            backoff_multiplier,
            jitter_factor,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<()> {
        bail_if(self.max_attempts == 0, "max_attempts must be greater than zero")?;
        bail_if(self.backoff_multiplier < 1.0, "backoff_multiplier must be at least 1.0")?;
        bail_if(
            !(0.0..=1.0).contains(&self.jitter_factor),
            "jitter_factor must be between 0.0 and 1.0",
        )?;
        Ok(())
    }

    /// Delay before the next attempt, without jitter. `attempt` starts at 1.
    pub fn base_delay(&self, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(1) as i32;
        let millis = self.initial_delay_ms as f64 * self.backoff_multiplier.powi(exponent);
        Duration::from_millis(millis.min(self.max_delay_ms as f64) as u64)
    }

    /// Delay with jitter applied, drawn from `rng`.
    pub fn delay_with<R: Rng + ?Sized>(&self, attempt: u32, rng: &mut R) -> Duration {
        let base = self.base_delay(attempt).as_millis() as f64;
        if self.jitter_factor == 0.0 {
            return Duration::from_millis(base as u64);
        }
        let low = 1.0 - self.jitter_factor;
        let high = 1.0 + self.jitter_factor;
        Duration::from_millis((base * rng.gen_range(low..=high)) as u64)
    }

    pub fn delay(&self, attempt: u32) -> Duration {
        self.delay_with(attempt, &mut rand::thread_rng())
    }
}

fn bail_if(condition: bool, message: &str) -> Result<()> {
    if condition {
        bail!("{message}");
    }
    Ok(())
}

/// Run `op` until it succeeds, the attempts run out, or it returns a fatal error.
///
/// The operation receives the attempt number, so it can rotate proxies or user
/// agents between tries.
pub async fn with_retry<T, F, Fut>(policy: &RetryPolicy, label: &str, mut op: F) -> Result<T>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<T>>,
{
    policy.validate()?;
    let mut last_error = None;
    for attempt in 1..=policy.max_attempts {
        match op(attempt).await {
            Ok(value) => return Ok(value),
            Err(error) => {
                eprintln!(
                    "Attempt {attempt}/{} failed for {label}: {error:#}",
                    policy.max_attempts
                );
                last_error = Some(error);
                if attempt < policy.max_attempts {
                    tokio::time::sleep(policy.delay(attempt)).await;
                }
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("{label}: no attempt was made")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn policy() -> RetryPolicy {
        RetryPolicy::new(5, 1000, 8000, 2.0, 0.1).unwrap()
    }

    #[test]
    fn delays_double_and_cap() {
        let policy = policy();
        let delays: Vec<u64> = (1..=6)
            .map(|attempt| policy.base_delay(attempt).as_millis() as u64)
            .collect();
        assert_eq!(delays, vec![1000, 2000, 4000, 8000, 8000, 8000]);
    }

    #[test]
    fn jitter_stays_inside_its_band() {
        let policy = policy();
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        for attempt in 1..=4 {
            let base = policy.base_delay(attempt).as_millis() as f64;
            for _ in 0..200 {
                let delay = policy.delay_with(attempt, &mut rng).as_millis() as f64;
                assert!(delay >= base * 0.9 - 1.0, "{delay} below band for {base}");
                assert!(delay <= base * 1.1 + 1.0, "{delay} above band for {base}");
            }
        }
    }

    #[test]
    fn zero_jitter_is_deterministic() {
        let policy = RetryPolicy::new(3, 500, 4000, 2.0, 0.0).unwrap();
        let mut rng = rand::rngs::StdRng::seed_from_u64(1);
        assert_eq!(
            policy.delay_with(2, &mut rng),
            policy.base_delay(2),
            "zero jitter must not vary"
        );
    }

    #[test]
    fn rejects_invalid_policies() {
        assert!(RetryPolicy::new(0, 1000, 8000, 2.0, 0.1).is_err());
        assert!(RetryPolicy::new(5, 1000, 8000, 0.5, 0.1).is_err());
        assert!(RetryPolicy::new(5, 1000, 8000, 2.0, 1.5).is_err());
    }

    #[tokio::test]
    async fn retries_until_success() {
        let policy = RetryPolicy::new(4, 1, 4, 2.0, 0.0).unwrap();
        let mut calls = 0;
        let value = with_retry(&policy, "test", |attempt| {
            calls += 1;
            async move {
                if attempt < 3 {
                    bail!("not yet")
                } else {
                    Ok(attempt)
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(value, 3);
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn gives_up_after_max_attempts() {
        let policy = RetryPolicy::new(3, 1, 4, 2.0, 0.0).unwrap();
        let mut calls = 0;
        let result: Result<()> = with_retry(&policy, "test", |_| {
            calls += 1;
            async { bail!("always fails") }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(calls, 3);
    }
}
