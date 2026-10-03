//! Supporting-signal thresholds, not authorization for enforcement.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearScoreCalibration {
    pub version: String,
    pub supporting_threshold: f64,
    pub strong_threshold: f64,
    pub supporting_score: i32,
    pub strong_score: i32,
}

impl Default for LinearScoreCalibration {
    fn default() -> Self {
        Self {
            version: "legacy-support-v1".to_owned(),
            supporting_threshold: 0.75,
            strong_threshold: 0.9,
            supporting_score: 10,
            strong_score: 18,
        }
    }
}

impl LinearScoreCalibration {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version.trim().is_empty()
            || !self.supporting_threshold.is_finite()
            || !self.strong_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.supporting_threshold)
            || !(self.supporting_threshold..=1.0).contains(&self.strong_threshold)
            || !(0..=45).contains(&self.supporting_score)
            || !(self.supporting_score..=45).contains(&self.strong_score)
        {
            return Err("invalid linear spam score calibration");
        }
        Ok(())
    }

    /// Invalid configuration or probability never increases risk.
    pub fn score(&self, probability: Option<f64>) -> i32 {
        let Some(probability) = probability.filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
        else {
            return 0;
        };
        if self.validate().is_err() {
            return 0;
        }
        if probability >= self.strong_threshold {
            self.strong_score
        } else if probability >= self.supporting_threshold {
            self.supporting_score
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_boundaries_and_invalid_values_are_safe() {
        let calibration = LinearScoreCalibration::default();
        for (p, expected) in [(0.749, 0), (0.75, 10), (0.899, 10), (0.9, 18), (1.0, 18)] {
            assert_eq!(calibration.score(Some(p)), expected);
        }
        for p in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
            assert_eq!(calibration.score(Some(p)), 0);
        }
        let invalid = LinearScoreCalibration {
            strong_threshold: 0.5,
            ..calibration
        };
        assert!(invalid.validate().is_err());
        assert_eq!(invalid.score(Some(0.99)), 0);
    }
}
