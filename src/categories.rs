//! Spam categories: multilabel taxonomy on top of the binary verdict.
//!
//! A category explains *what kind* of abuse a message looks like; it never
//! authorizes enforcement on its own. Scores are independent probabilities
//! in [0, 1] (one-vs-rest), not a softmax: a message can show several
//! funnels at once, or none.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable spam category taxonomy. Variants are append-only: never rename or
/// reuse a serial name, add a new one instead.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SpamCategory {
    JobScam,
    FinanceCryptoPromo,
    AdultFunnel,
    VpnPromo,
    DirectDmFunnel,
    ExternalPromo,
}

impl SpamCategory {
    pub fn all() -> [SpamCategory; 6] {
        [
            SpamCategory::JobScam,
            SpamCategory::FinanceCryptoPromo,
            SpamCategory::AdultFunnel,
            SpamCategory::VpnPromo,
            SpamCategory::DirectDmFunnel,
            SpamCategory::ExternalPromo,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SpamCategory::JobScam => "job_scam",
            SpamCategory::FinanceCryptoPromo => "finance_crypto_promo",
            SpamCategory::AdultFunnel => "adult_funnel",
            SpamCategory::VpnPromo => "vpn_promo",
            SpamCategory::DirectDmFunnel => "direct_dm_funnel",
            SpamCategory::ExternalPromo => "external_promo",
        }
    }
}

/// One independent category probability.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryScore {
    pub category: SpamCategory,
    pub probability: f64,
}

/// Multilabel prediction set with an explicit producer version.
/// `version` distinguishes heuristic marker mapping (`markers-v1`) from a
/// trained head (`embed-multitask-<model>-vX`); consumers must not treat a
/// heuristic version as a learned one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryScores {
    pub version: String,
    pub scores: Vec<CategoryScore>,
}

impl CategoryScores {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version.trim().is_empty() {
            return Err("category scores require a version");
        }
        if self.scores.is_empty() || self.scores.len() > SpamCategory::all().len() {
            return Err("category scores must cover 1..=6 categories");
        }
        let mut seen = BTreeMap::new();
        for score in &self.scores {
            if !score.probability.is_finite() || !(0.0..=1.0).contains(&score.probability) {
                return Err("category probability must be finite and within [0,1]");
            }
            if seen.insert(score.category, 1).is_some() {
                return Err("duplicate category score");
            }
        }
        Ok(())
    }

    /// Highest-probability category. Ties resolve by enum order for
    /// determinism. Returns `None` for invalid sets.
    pub fn top(&self) -> Option<CategoryScore> {
        if self.validate().is_err() {
            return None;
        }
        self.scores
            .iter()
            .max_by(|a, b| {
                a.probability
                    .total_cmp(&b.probability)
                    .then_with(|| a.category.cmp(&b.category))
            })
            .copied()
    }

    /// Probability for one category, or `None` when absent/invalid.
    pub fn probability(&self, category: SpamCategory) -> Option<f64> {
        if self.validate().is_err() {
            return None;
        }
        self.scores
            .iter()
            .find(|score| score.category == category)
            .map(|score| score.probability)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scores(pairs: &[(SpamCategory, f64)]) -> CategoryScores {
        CategoryScores {
            version: "test-v1".to_owned(),
            scores: pairs
                .iter()
                .map(|&(category, probability)| CategoryScore {
                    category,
                    probability,
                })
                .collect(),
        }
    }

    #[test]
    fn top_is_deterministic_and_validated() {
        let set = scores(&[
            (SpamCategory::JobScam, 0.7),
            (SpamCategory::ExternalPromo, 0.7),
        ]);
        // Tie: higher enum variant wins deterministically.
        assert_eq!(
            set.top().map(|s| s.category),
            Some(SpamCategory::ExternalPromo)
        );
        assert_eq!(set.probability(SpamCategory::JobScam), Some(0.7));
    }

    #[test]
    fn invalid_probabilities_reject_top() {
        let bad = scores(&[(SpamCategory::JobScam, f64::NAN)]);
        assert!(bad.validate().is_err());
        assert_eq!(bad.top(), None);
        let dup = CategoryScores {
            version: "test-v1".to_owned(),
            scores: vec![
                CategoryScore {
                    category: SpamCategory::JobScam,
                    probability: 0.5,
                },
                CategoryScore {
                    category: SpamCategory::JobScam,
                    probability: 0.6,
                },
            ],
        };
        assert!(dup.validate().is_err());
    }
}
