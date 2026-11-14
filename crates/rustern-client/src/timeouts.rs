//! Per-attempt timeouts (research R4): exponential from `initial` by 1.5, capped at `max`,
//! never ending (the overall deadline stops the loop), each scaled by ±5% jitter.

use std::time::Duration;

use backon::{BackoffBuilder, ExponentialBuilder};

use crate::config::ReqTimeouts;

/// Timeouts for successive attempts, with jitter from a fresh random seed.
pub fn schedule(t: &ReqTimeouts) -> impl Iterator<Item = Duration> + Send + use<> {
    schedule_with_rng(t, fastrand::Rng::new())
}

/// [`schedule`] with a given random source (deterministic tests).
pub fn schedule_with_rng(
    t: &ReqTimeouts,
    mut rng: fastrand::Rng,
) -> impl Iterator<Item = Duration> + Send + use<> {
    let max = t.max;
    ExponentialBuilder::new()
        .with_min_delay(t.initial)
        .with_max_delay(t.max)
        .with_factor(1.5)
        .without_max_times()
        .build()
        .map(move |d| d.mul_f64(0.95 + rng.f64() * 0.1).min(max))
}

#[cfg(test)]
#[allow(clippy::arithmetic_side_effects)] // test code
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::config::ClientConfig;

    fn near(d: Duration, ms: f64) -> bool {
        let got = d.as_secs_f64() * 1000.0;
        (ms * 0.95..=ms * 1.05).contains(&got)
    }

    #[test]
    fn shard_defaults_grow_by_half() {
        let t = ClientConfig::new("r:1").shard;
        let v: Vec<Duration> = schedule_with_rng(&t, fastrand::Rng::with_seed(7))
            .take(12)
            .collect();
        assert!(
            near(v[0], 100.0) && near(v[1], 150.0) && near(v[2], 225.0),
            "{v:?}"
        );
        assert!(v[11] <= t.max && near(v[11], 2000.0), "capped: {v:?}");
    }

    #[test]
    fn never_ends() {
        let t = ClientConfig::new("r:1").shard;
        assert_eq!(schedule(&t).take(10_000).count(), 10_000);
    }

    proptest! {
        #[test]
        fn within_bounds(initial in 1u64..5000, extra in 0u64..60_000, seed: u64) {
            let t = ReqTimeouts {
                initial: Duration::from_millis(initial),
                max: Duration::from_millis(initial + extra),
                overall: Duration::from_secs(60),
            };
            for d in schedule_with_rng(&t, fastrand::Rng::with_seed(seed)).take(50) {
                prop_assert!(d >= t.initial.mul_f64(0.95) && d <= t.max, "{:?}", d);
            }
        }
    }
}
