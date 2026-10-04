//! Dense-embedding spam heads: binary verdict plus multilabel categories.
//!
//! The embedding model itself (e.g. EmbeddingGemma) runs outside this
//! library. Here a *frozen* embedding vector is scored by a small versioned
//! linear layer: `sigmoid(dot(weights, x) + intercept)` per head. Training
//! fits only these heads; the encoder is never fine-tuned here.
//!
//! Scores are supporting evidence like the TF-IDF probability, never a
//! standalone ban authorization. Invalid vectors or heads yield no points.

use serde::Deserialize;
use std::collections::BTreeMap;

use crate::calibration::LinearScoreCalibration;
use crate::categories::{CategoryScore, CategoryScores, SpamCategory};

/// Versioned multitask head over a frozen embedding.
#[derive(Debug, Clone)]
pub struct EmbeddingSpamModel {
    pub version: String,
    pub embedding_model: String,
    pub dim: usize,
    pub normalize: bool,
    pub spam_weights: Vec<f64>,
    pub spam_intercept: f64,
    pub category_heads: BTreeMap<SpamCategory, CategoryHead>,
    pub calibration: LinearScoreCalibration,
}

#[derive(Debug, Clone)]
pub struct CategoryHead {
    pub weights: Vec<f64>,
    pub intercept: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbeddingExport {
    version: String,
    embedding_model: String,
    dim: usize,
    #[serde(default = "default_normalize")]
    normalize: bool,
    spam: DenseHeadExport,
    #[serde(default)]
    categories: BTreeMap<SpamCategory, DenseHeadExport>,
    /// Supporting-score operating points selected on validation.
    /// Absent in older packs: defaults to the legacy text calibration.
    #[serde(default)]
    calibration: LinearScoreCalibration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DenseHeadExport {
    weights: Vec<f64>,
    intercept: f64,
}

fn default_normalize() -> bool {
    true
}

impl EmbeddingSpamModel {
    pub fn load(json: &str) -> anyhow::Result<Self> {
        let export: EmbeddingExport = serde_json::from_str(json)?;
        if export.version.trim().is_empty() || export.embedding_model.trim().is_empty() {
            anyhow::bail!("embedding head requires version and embedding model");
        }
        if export.dim == 0 || export.dim > 8192 {
            anyhow::bail!("embedding head dim out of range");
        }
        export.calibration.validate().map_err(anyhow::Error::msg)?;
        if !export.spam.intercept.is_finite()
            || !export.spam.weights.iter().all(|w| w.is_finite())
            || export.spam.weights.len() != export.dim
        {
            anyhow::bail!("invalid binary spam head");
        }
        let mut category_heads = BTreeMap::new();
        for (category, head) in export.categories {
            if !head.intercept.is_finite()
                || !head.weights.iter().all(|w| w.is_finite())
                || head.weights.len() != export.dim
            {
                anyhow::bail!("invalid category head");
            }
            category_heads.insert(
                category,
                CategoryHead {
                    weights: head.weights,
                    intercept: head.intercept,
                },
            );
        }
        Ok(Self {
            version: export.version,
            embedding_model: export.embedding_model,
            dim: export.dim,
            normalize: export.normalize,
            spam_weights: export.spam.weights,
            spam_intercept: export.spam.intercept,
            category_heads,
            calibration: export.calibration,
        })
    }

    fn validated_vector(&self, embedding: &[f32]) -> Option<Vec<f64>> {
        if embedding.len() != self.dim {
            return None;
        }
        let mut vector: Vec<f64> = embedding.iter().map(|&x| f64::from(x)).collect();
        if vector.iter().any(|x| !x.is_finite()) {
            return None;
        }
        if self.normalize {
            let norm = vector.iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm == 0.0 {
                return None;
            }
            for x in &mut vector {
                *x /= norm;
            }
        }
        Some(vector)
    }

    fn logit(weights: &[f64], intercept: f64, vector: &[f64]) -> f64 {
        weights.iter().zip(vector).map(|(w, x)| w * x).sum::<f64>() + intercept
    }

    fn sigmoid(value: f64) -> f64 {
        1.0 / (1.0 + (-value).exp())
    }

    /// Binary spam probability, or `None` for dim mismatch / non-finite input.
    pub fn spam_probability(&self, embedding: &[f32]) -> Option<f64> {
        let vector = self.validated_vector(embedding)?;
        Some(Self::sigmoid(Self::logit(
            &self.spam_weights,
            self.spam_intercept,
            &vector,
        )))
    }

    /// Multilabel category probabilities. `None` when the vector is unusable.
    /// A head-less model yields an empty (invalid) set; callers must handle
    /// `None`/invalid as "no category evidence".
    pub fn category_scores(&self, embedding: &[f32]) -> Option<CategoryScores> {
        let vector = self.validated_vector(embedding)?;
        let scores = self
            .category_heads
            .iter()
            .map(|(&category, head)| CategoryScore {
                category,
                probability: Self::sigmoid(Self::logit(&head.weights, head.intercept, &vector)),
            })
            .collect();
        Some(CategoryScores {
            version: format!("embed-{}-{}", self.embedding_model, self.version),
            scores,
        })
    }
}

/// Late fusion of two supporting probabilities in logit space:
/// `sigmoid(w_text * logit(text) + w_emb * logit(emb) + bias)`.
/// Weights are non-negative; invalid inputs yield `None` (no evidence).
/// This is an operating-point combiner, not posterior calibration.
pub fn fuse_probabilities(
    text_probability: Option<f64>,
    embedding_probability: Option<f64>,
    text_weight: f64,
    embedding_weight: f64,
    bias: f64,
) -> Option<f64> {
    if !text_weight.is_finite() || !embedding_weight.is_finite() || !bias.is_finite() {
        return None;
    }
    if text_weight < 0.0 || embedding_weight < 0.0 {
        return None;
    }
    let logit = |p: f64| (p / (1.0 - p)).ln();
    let mut combined = bias;
    let mut used = false;
    if let Some(p) = text_probability.filter(|p| p.is_finite() && (0.0..=1.0).contains(p)) {
        let clamped = p.clamp(1e-6, 1.0 - 1e-6);
        combined += text_weight * logit(clamped);
        used = true;
    }
    if let Some(p) = embedding_probability.filter(|p| p.is_finite() && (0.0..=1.0).contains(p)) {
        let clamped = p.clamp(1e-6, 1.0 - 1e-6);
        combined += embedding_weight * logit(clamped);
        used = true;
    }
    if !used {
        return None;
    }
    Some(1.0 / (1.0 + (-combined).exp()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> EmbeddingSpamModel {
        EmbeddingSpamModel::load(
            r#"{
                "version": "test-v1", "embedding_model": "test-encoder-3",
                "dim": 3, "normalize": true,
                "spam": {"weights": [1.0, 0.0, 0.0], "intercept": 0.0},
                "categories": {"job_scam": {"weights": [0.0, 2.0, 0.0], "intercept": -1.0}},
                "calibration": {"version": "test-cal-v1", "supporting_threshold": 0.75, "strong_threshold": 0.9, "supporting_score": 10, "strong_score": 18}
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn binary_and_category_heads_score_l2_normalized_vectors() {
        let model = model();
        let prob = model.spam_probability(&[2.0, 0.0, 0.0]).unwrap();
        assert!((prob - 1.0 / (1.0 + (-1.0_f64).exp())).abs() < 1e-12);
        let categories = model.category_scores(&[0.0, 3.0, 0.0]).unwrap();
        let job = categories.probability(SpamCategory::JobScam).unwrap();
        assert!((job - 1.0 / (1.0 + (-1.0_f64).exp())).abs() < 1e-12);
    }

    #[test]
    fn dim_mismatch_and_zero_vectors_yield_no_evidence() {
        let model = model();
        assert_eq!(model.spam_probability(&[1.0, 0.0]), None);
        assert_eq!(model.spam_probability(&[0.0, 0.0, 0.0]), None);
        assert_eq!(model.spam_probability(&[f32::NAN, 0.0, 0.0]), None);
    }

    #[test]
    fn loader_rejects_bad_dims_and_nonfinite_weights() {
        assert!(EmbeddingSpamModel::load(r#"{"version":"v","embedding_model":"m","dim":0,"spam":{"weights":[],"intercept":0.0}}"#).is_err());
        assert!(EmbeddingSpamModel::load(r#"{"version":"v","embedding_model":"m","dim":1,"spam":{"weights":[1.0],"intercept":0.0},"extra":1}"#).is_err());
        assert!(EmbeddingSpamModel::load(r#"{"version":"v","embedding_model":"m","dim":1,"spam":{"weights":[null],"intercept":0.0}}"#).is_err());
    }

    #[test]
    fn loader_rejects_invalid_calibration() {
        assert!(EmbeddingSpamModel::load(r#"{"version":"v","embedding_model":"m","dim":1,"spam":{"weights":[1.0],"intercept":0.0},"calibration":{"version":"c","supporting_threshold":0.9,"strong_threshold":0.5,"supporting_score":10,"strong_score":18}}"#).is_err());
    }

    #[test]
    fn fusion_is_fail_closed_and_monotone() {
        assert_eq!(fuse_probabilities(None, None, 1.0, 1.0, 0.0), None);
        assert_eq!(fuse_probabilities(Some(0.9), None, -1.0, 1.0, 0.0), None);
        let low = fuse_probabilities(Some(0.6), Some(0.6), 1.0, 1.0, 0.0).unwrap();
        let high = fuse_probabilities(Some(0.9), Some(0.9), 1.0, 1.0, 0.0).unwrap();
        assert!(high > low);
    }
}
