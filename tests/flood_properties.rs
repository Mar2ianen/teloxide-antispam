use proptest::prelude::*;
use teloxide_antispam::flood::{DecayedMean, EwmaRate, ExactWindow};

proptest! {
    #![proptest_config(ProptestConfig::default())]

    #[test]
    fn rate_matches_full_history_sum(
        deltas in prop::collection::vec(1_u32..50, 1..100),
        weights in prop::collection::vec(1_u32..=5, 1..100),
        tau in 100_u64..10_000_000,
        trailing in 0_u32..50,
    ) {
        let len = deltas.len().min(weights.len());
        let mut rate = EwmaRate::new(tau).unwrap();
        let mut events = Vec::with_capacity(len);
        let mut at = 0_u64;
        for (delta, weight) in deltas.into_iter().zip(weights).take(len) {
            at += u64::from(delta);
            events.push((at, weight));
            rate.observe(at, weight).unwrap();
        }
        let now = at + u64::from(trailing);
        let expected = events.iter().map(|(time, weight)| {
            f64::from(*weight) * 1_000_000.0 / tau as f64
                * (-((now - time) as f64) / tau as f64).exp()
        }).sum::<f64>();
        let actual = rate.rate_at(now).unwrap().unwrap();
        prop_assert!((actual - expected).abs() <= 1e-10 * expected.max(1.0));
    }

    #[test]
    fn exact_window_matches_filtering_full_history(
        deltas in prop::collection::vec(1_u32..50, 1..100),
        weights in prop::collection::vec(1_u32..=5, 1..100),
        window_size in 1_u64..100_000,
        trailing in 0_u32..10_000,
    ) {
        let len = deltas.len().min(weights.len());
        let mut window = ExactWindow::new(window_size, len.max(1)).unwrap();
        let mut events = Vec::with_capacity(len);
        let mut at = 0_u64;
        for (delta, weight) in deltas.into_iter().zip(weights).take(len) {
            at += u64::from(delta);
            events.push((at, weight));
            window.observe(at, weight).unwrap();
        }
        let now = at + u64::from(trailing);
        let cutoff = now.checked_sub(window_size);
        let expected = events.iter()
            .filter(|(time, _)| cutoff.is_none_or(|lower| *time > lower) && *time <= now)
            .map(|(_, weight)| u64::from(*weight))
            .sum::<u64>();
        prop_assert_eq!(window.count_at(now).unwrap(), Some(expected));
    }

    #[test]
    fn exact_window_overflow_sequences_never_expose_a_partial_count(
        deltas in prop::collection::vec(0_u32..20, 1..100),
        weights in prop::collection::vec(1_u32..=5, 1..100),
        window_size in 1_u64..100,
        capacity in 1_usize..10,
    ) {
        let len = deltas.len().min(weights.len());
        let mut window = ExactWindow::new(window_size, capacity).unwrap();
        let mut retained = Vec::<(u64, u32)>::new();
        let mut history = Vec::<(u64, u32)>::new();
        let mut dropped = Vec::<u64>::new();
        let mut at = 0_u64;

        for (delta, weight) in deltas.into_iter().zip(weights).take(len) {
            at += u64::from(delta);
            let cutoff = at.checked_sub(window_size);
            retained.retain(|(time, _)| cutoff.is_none_or(|lower| *time > lower));
            history.push((at, weight));

            if retained.len() == capacity {
                prop_assert_eq!(
                    window.observe(at, weight),
                    Err(teloxide_antispam::flood::WindowError::CapacityExceeded)
                );
                dropped.push(at);
            } else {
                retained.push((at, weight));
                let lost_is_live = dropped.iter().any(|time| {
                    cutoff.is_none_or(|lower| *time > lower) && *time <= at
                });
                let expected = if lost_is_live {
                    None
                } else {
                    Some(history.iter()
                        .filter(|(time, _)| cutoff.is_none_or(|lower| *time > lower) && *time <= at)
                        .map(|(_, weight)| u64::from(*weight))
                        .sum::<u64>())
                };
                prop_assert_eq!(window.observe(at, weight), Ok(expected));
            }

            let lost_is_live = dropped.iter().any(|time| {
                cutoff.is_none_or(|lower| *time > lower) && *time <= at
            });
            let expected = if lost_is_live {
                None
            } else {
                Some(history.iter()
                    .filter(|(time, _)| cutoff.is_none_or(|lower| *time > lower) && *time <= at)
                    .map(|(_, weight)| u64::from(*weight))
                    .sum::<u64>())
            };
            prop_assert_eq!(window.count_at(at).unwrap(), expected);
        }
    }

    #[test]
    fn weighted_mean_matches_full_history_oracle(
        deltas in prop::collection::vec(1_u32..10_000, 1..100),
        values in prop::collection::vec(-10_000_i32..10_000, 1..100),
        weights in prop::collection::vec(1_u32..=100, 1..100),
        tau in 100_u64..10_000_000,
        trailing in 0_u32..50,
    ) {
        let len = deltas.len().min(values.len()).min(weights.len());
        let mut mean = DecayedMean::new(tau, f64::MIN_POSITIVE).unwrap();
        let mut events = Vec::with_capacity(len);
        let mut at = 0_u64;
        for ((delta, value), weight) in deltas.into_iter().zip(values).zip(weights).take(len) {
            at += u64::from(delta);
            let value = f64::from(value) / 100.0;
            let weight = f64::from(weight) / 10.0;
            events.push((at, value, weight));
            mean.observe(at, value, weight).unwrap();
        }
        let now = at + u64::from(trailing);
        let (sum, total_weight) = events.iter().map(|(time, value, weight)| {
            let decay = (-((now - time) as f64) / tau as f64).exp();
            (value * weight * decay, weight * decay)
        }).fold((0.0, 0.0), |(sum, total), (next_sum, next_weight)| {
            (sum + next_sum, total + next_weight)
        });
        let actual = mean.mean_at(now).unwrap().unwrap();
        prop_assert!((actual - sum / total_weight).abs() < 1e-8);
    }
}
