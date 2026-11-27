//! Pure time-decayed primitives for conversation flood detectors.
//!
//! These types deliberately do not choose policy thresholds or take actions.
//! Callers provide an event-time timeline in microseconds. Samples must be
//! submitted in nondecreasing event-time order; consumers that accept late
//! Telegram updates must reorder them or mark their evidence incomplete
//! before updating these primitives.

use std::fmt;

/// A timestamp on the caller's monotonic event-time timeline, in microseconds.
pub type EventTimeMicros = u64;

/// Invalid input to a time-decayed statistic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EwmaError {
    /// The decay time constant must be greater than zero.
    ZeroTimeConstant,
    /// An event-rate observation must contain at least one event.
    ZeroEventWeight,
    /// A weighted-mean observation must have finite value and positive finite weight.
    InvalidSample,
    /// Samples were submitted earlier than the last accepted event time.
    OutOfOrder {
        last_accepted: EventTimeMicros,
        received: EventTimeMicros,
    },
    /// An update would produce a non-finite internal value.
    ArithmeticOverflow,
}

impl fmt::Display for EwmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTimeConstant => f.write_str("EWMA time constant must be positive"),
            Self::ZeroEventWeight => f.write_str("EWMA event weight must be positive"),
            Self::InvalidSample => {
                f.write_str("EWMA sample must be finite and its weight positive and finite")
            }
            Self::OutOfOrder {
                last_accepted,
                received,
            } => write!(
                f,
                "EWMA event time {received} precedes last accepted time {last_accepted}"
            ),
            Self::ArithmeticOverflow => f.write_str("EWMA update produced a non-finite value"),
        }
    }
}

impl std::error::Error for EwmaError {}

/// An event rate with exponential decay.
///
/// For time constant `tau`, each observation of weight `w` contributes
/// `w / tau_seconds` events per second, and prior state decays by
/// `exp(-delta / tau)`. This is a rate, not a hard-window count; use an exact
/// window or token bucket for hard limits.
#[derive(Clone, Debug, PartialEq)]
pub struct EwmaRate {
    tau_micros: u64,
    last_at: Option<EventTimeMicros>,
    rate_per_second: f64,
    accepted_events: u64,
}

impl EwmaRate {
    /// Creates an empty rate with a positive time constant in microseconds.
    pub fn new(tau_micros: u64) -> Result<Self, EwmaError> {
        if tau_micros == 0 {
            return Err(EwmaError::ZeroTimeConstant);
        }
        Ok(Self {
            tau_micros,
            last_at: None,
            rate_per_second: 0.0,
            accepted_events: 0,
        })
    }

    /// Adds `weight` logical events at `at`.
    ///
    /// Invalid or out-of-order input leaves the statistic unchanged.
    pub fn observe(&mut self, at: EventTimeMicros, weight: u32) -> Result<(), EwmaError> {
        if weight == 0 {
            return Err(EwmaError::ZeroEventWeight);
        }
        self.validate_time(at)?;

        let decayed = self.rate_at_unchecked(at);
        let contribution = f64::from(weight) * 1_000_000.0 / self.tau_micros as f64;
        let next = decayed + contribution;
        if !next.is_finite() {
            return Err(EwmaError::ArithmeticOverflow);
        }

        self.rate_per_second = next;
        self.last_at = Some(at);
        self.accepted_events = self.accepted_events.saturating_add(u64::from(weight));
        Ok(())
    }

    /// Returns the decayed events-per-second estimate at `at`.
    ///
    /// Reading at a future time does not mutate the statistic. A timestamp
    /// before the last accepted event is rejected rather than clamped.
    pub fn rate_at(&self, at: EventTimeMicros) -> Result<Option<f64>, EwmaError> {
        let Some(last_at) = self.last_at else {
            return Ok(None);
        };
        if at < last_at {
            return Err(EwmaError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }
        Ok(Some(self.rate_at_unchecked(at)))
    }

    /// Number of logical events accepted, including weights.
    pub fn accepted_events(&self) -> u64 {
        self.accepted_events
    }

    /// The configured exponential time constant, in microseconds.
    pub fn tau_micros(&self) -> u64 {
        self.tau_micros
    }

    fn validate_time(&self, at: EventTimeMicros) -> Result<(), EwmaError> {
        if let Some(last_at) = self.last_at
            && at < last_at
        {
            return Err(EwmaError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }
        Ok(())
    }

    fn rate_at_unchecked(&self, at: EventTimeMicros) -> f64 {
        let Some(last_at) = self.last_at else {
            return 0.0;
        };
        let elapsed = at - last_at;
        self.rate_per_second * decay(elapsed, self.tau_micros)
    }
}

/// A time-decayed weighted mean for contextual features.
///
/// The mean is `decayed_weighted_sum / decayed_weight`. Callers choose the
/// minimum effective weight needed before exposing a value; before that, the
/// result is `None` rather than a fabricated zero.
#[derive(Clone, Debug, PartialEq)]
pub struct DecayedMean {
    tau_micros: u64,
    minimum_effective_weight: f64,
    last_at: Option<EventTimeMicros>,
    weighted_sum: f64,
    effective_weight: f64,
    accepted_samples: u64,
}

impl DecayedMean {
    /// Creates a decayed mean with a positive time constant and warmup weight.
    pub fn new(tau_micros: u64, minimum_effective_weight: f64) -> Result<Self, EwmaError> {
        if tau_micros == 0 {
            return Err(EwmaError::ZeroTimeConstant);
        }
        if !minimum_effective_weight.is_finite() || minimum_effective_weight <= 0.0 {
            return Err(EwmaError::InvalidSample);
        }
        Ok(Self {
            tau_micros,
            minimum_effective_weight,
            last_at: None,
            weighted_sum: 0.0,
            effective_weight: 0.0,
            accepted_samples: 0,
        })
    }

    /// Adds one finite value with positive finite weight at event time `at`.
    ///
    /// Invalid, overflowing, or out-of-order input leaves state unchanged.
    pub fn observe(
        &mut self,
        at: EventTimeMicros,
        value: f64,
        weight: f64,
    ) -> Result<(), EwmaError> {
        if !value.is_finite() || !weight.is_finite() || weight <= 0.0 {
            return Err(EwmaError::InvalidSample);
        }
        if let Some(last_at) = self.last_at
            && at < last_at
        {
            return Err(EwmaError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }

        let factor = self
            .last_at
            .map_or(1.0, |last_at| decay(at - last_at, self.tau_micros));
        let next_sum = self.weighted_sum * factor + value * weight;
        let next_weight = self.effective_weight * factor + weight;
        if !next_sum.is_finite() || !next_weight.is_finite() {
            return Err(EwmaError::ArithmeticOverflow);
        }

        self.weighted_sum = next_sum;
        self.effective_weight = next_weight;
        self.last_at = Some(at);
        self.accepted_samples = self.accepted_samples.saturating_add(1);
        Ok(())
    }

    /// Returns the mean at `at` once enough decayed sample weight remains.
    pub fn mean_at(&self, at: EventTimeMicros) -> Result<Option<f64>, EwmaError> {
        let Some(last_at) = self.last_at else {
            return Ok(None);
        };
        if at < last_at {
            return Err(EwmaError::OutOfOrder {
                last_accepted: last_at,
                received: at,
            });
        }

        let factor = decay(at - last_at, self.tau_micros);
        let effective_weight = self.effective_weight * factor;
        if effective_weight < self.minimum_effective_weight {
            return Ok(None);
        }
        let mean = (self.weighted_sum * factor) / effective_weight;
        if !mean.is_finite() {
            return Err(EwmaError::ArithmeticOverflow);
        }
        Ok(Some(mean))
    }

    /// Number of accepted observations before decay.
    pub fn accepted_samples(&self) -> u64 {
        self.accepted_samples
    }

    /// The current effective sample weight at the most recent event time.
    pub fn effective_weight(&self) -> f64 {
        self.effective_weight
    }
}

fn decay(elapsed_micros: u64, tau_micros: u64) -> f64 {
    (-(elapsed_micros as f64) / tau_micros as f64).exp()
}

#[cfg(test)]
mod tests {
    use super::{DecayedMean, EwmaError, EwmaRate};

    #[test]
    fn rate_uses_event_weight_and_decay_at_equal_and_later_timestamps() {
        let mut rate = EwmaRate::new(1_000_000).unwrap();
        assert_eq!(rate.rate_at(0).unwrap(), None);
        rate.observe(10, 1).unwrap();
        rate.observe(10, 2).unwrap();
        assert!((rate.rate_at(10).unwrap().unwrap() - 3.0).abs() < 1e-12);
        let after_one_tau = rate.rate_at(1_000_010).unwrap().unwrap();
        assert!((after_one_tau - 3.0 / std::f64::consts::E).abs() < 1e-12);
        assert_eq!(rate.accepted_events(), 3);
    }

    #[test]
    fn rate_halves_at_the_exponential_half_life() {
        let mut rate = EwmaRate::new(1_000_000).unwrap();
        rate.observe(0, 1).unwrap();
        let half_life_micros = (1_000_000.0 * std::f64::consts::LN_2).round() as u64;
        let observed = rate.rate_at(half_life_micros).unwrap().unwrap();
        assert!((observed - 0.5).abs() < 1e-6);
    }

    #[test]
    fn invalid_rate_updates_do_not_mutate_state() {
        assert_eq!(EwmaRate::new(0), Err(EwmaError::ZeroTimeConstant));
        let mut rate = EwmaRate::new(5).unwrap();
        rate.observe(10, 2).unwrap();
        let before = rate.clone();
        assert_eq!(rate.observe(11, 0), Err(EwmaError::ZeroEventWeight));
        assert_eq!(
            rate.observe(9, 1),
            Err(EwmaError::OutOfOrder {
                last_accepted: 10,
                received: 9,
            })
        );
        assert_eq!(rate, before);
    }

    #[test]
    fn rate_read_does_not_advance_internal_time() {
        let mut observed_after_read = EwmaRate::new(2_000_000).unwrap();
        let mut direct = EwmaRate::new(2_000_000).unwrap();
        observed_after_read.observe(0, 1).unwrap();
        direct.observe(0, 1).unwrap();
        let _ = observed_after_read.rate_at(1_000_000).unwrap();
        observed_after_read.observe(2_000_000, 1).unwrap();
        direct.observe(2_000_000, 1).unwrap();
        assert_eq!(observed_after_read, direct);
    }

    #[test]
    fn rate_matches_independent_full_history_sum() {
        let tau = 7_300_000_u64;
        let mut rate = EwmaRate::new(tau).unwrap();
        let mut observations = Vec::new();
        let mut seed = 0x9e37_79b9_u64;
        let mut at = 0_u64;
        for _ in 0..500 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            at += seed % 31_000 + 1;
            let weight = ((seed >> 24) % 4 + 1) as u32;
            observations.push((at, weight));
            rate.observe(at, weight).unwrap();
        }

        for extra in [0, 1, tau / 2, tau, tau * 7] {
            let now = at + extra;
            let expected = observations
                .iter()
                .map(|(time, weight)| {
                    f64::from(*weight) * 1_000_000.0 / tau as f64
                        * (-((now - time) as f64) / tau as f64).exp()
                })
                .sum::<f64>();
            let actual = rate.rate_at(now).unwrap().unwrap();
            assert!((actual - expected).abs() <= 1e-10 * expected.max(1.0));
        }
    }

    #[test]
    fn weighted_mean_returns_unknown_until_warm_and_preserves_ratio_under_decay() {
        let mut mean = DecayedMean::new(1_000_000, 1.5).unwrap();
        assert_eq!(mean.mean_at(0).unwrap(), None);
        mean.observe(0, 10.0, 1.0).unwrap();
        assert_eq!(mean.mean_at(0).unwrap(), None);
        mean.observe(0, 20.0, 1.0).unwrap();
        assert_eq!(mean.mean_at(0).unwrap(), Some(15.0));
        assert_eq!(mean.mean_at(100_000).unwrap(), Some(15.0));
        assert_eq!(mean.mean_at(2_000_000).unwrap(), None);
        assert_eq!(mean.accepted_samples(), 2);
    }

    #[test]
    fn weighted_mean_rejects_nonfinite_and_out_of_order_input_atomically() {
        assert_eq!(DecayedMean::new(0, 1.0), Err(EwmaError::ZeroTimeConstant));
        assert_eq!(DecayedMean::new(1, 0.0), Err(EwmaError::InvalidSample));
        let mut mean = DecayedMean::new(1_000, 0.1).unwrap();
        mean.observe(100, 3.0, 1.0).unwrap();
        let before = mean.clone();
        assert_eq!(
            mean.observe(100, f64::NAN, 1.0),
            Err(EwmaError::InvalidSample)
        );
        assert_eq!(
            mean.observe(100, 3.0, f64::INFINITY),
            Err(EwmaError::InvalidSample)
        );
        assert_eq!(
            mean.observe(99, 4.0, 1.0),
            Err(EwmaError::OutOfOrder {
                last_accepted: 100,
                received: 99,
            })
        );
        assert_eq!(mean, before);
    }

    #[test]
    fn weighted_mean_matches_full_history_weighted_sum() {
        let tau = 11_000_u64;
        let min_weight = 0.01;
        let mut mean = DecayedMean::new(tau, min_weight).unwrap();
        let mut observations = Vec::new();
        let mut seed = 0x1234_5678_u64;
        let mut at = 0_u64;
        for _ in 0..300 {
            seed = seed
                .wrapping_mul(2_862_933_555_777_941_757)
                .wrapping_add(3_037_000_493);
            at += seed % 900 + 1;
            let value = ((seed >> 18) % 20_000) as f64 / 100.0 - 50.0;
            let weight = ((seed >> 43) % 100 + 1) as f64 / 10.0;
            observations.push((at, value, weight));
            mean.observe(at, value, weight).unwrap();
        }

        let now = at + tau / 3;
        let (sum, weight) = observations
            .iter()
            .map(|(time, value, sample_weight)| {
                let decay = (-((now - time) as f64) / tau as f64).exp();
                (value * sample_weight * decay, sample_weight * decay)
            })
            .fold((0.0, 0.0), |(sum, weight), (s, w)| (sum + s, weight + w));
        assert!((mean.mean_at(now).unwrap().unwrap() - sum / weight).abs() < 1e-10);
    }
}
