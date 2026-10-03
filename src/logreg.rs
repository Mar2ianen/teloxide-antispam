use std::collections::HashMap;

use crate::calibration::LinearScoreCalibration;
use serde::Deserialize;

use crate::preprocess::{PREPROCESSING_VERSION, char_tokens, prepare_text, word_tokens};

pub const UNICODE_WORD_CHAR_ANALYZER: &str = "unicode_word_char_v2";

/// Versioned TF-IDF/LogReg model: legacy word 1–2 or Unicode word/char.
///
/// Feature extraction must match the exporter: lowercase, word 1–2,
/// raw term counts, exported IDF, block-wise L2 normalization, and
/// sigmoid(dot + intercept). Unicode v2 also uses char_wb 3–5.
#[derive(Debug, Clone)]
pub struct LinearSpamModel {
    pub version: String,
    pub calibration: LinearScoreCalibration,
    intercept: f64,
    features: FeatureModel,
}

#[derive(Debug, Clone)]
enum FeatureModel {
    Legacy(FeatureBlock),
    UnicodeWordChar {
        word: FeatureBlock,
        character: FeatureBlock,
    },
}

#[derive(Debug, Clone)]
struct FeatureBlock {
    terms: HashMap<String, usize>,
    idf: Vec<f64>,
    coef: Vec<f64>,
    weight: f64,
}

#[derive(Deserialize)]
struct ModelHeader {
    analyzer: String,
}

#[derive(Debug, Deserialize)]
struct LinearSpamExport {
    version: String,
    vocab: HashMap<String, f32>,
    coef: Vec<f32>,
    intercept: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnicodeWordCharExport {
    analyzer: String,
    version: String,
    preprocessing: String,
    word: FeatureBlockExport,
    character: FeatureBlockExport,
    intercept: f64,
    calibration: LinearScoreCalibration,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FeatureBlockExport {
    vocab: HashMap<String, f64>,
    coef: Vec<f64>,
    weight: f64,
}

pub fn load_model(json: &str) -> anyhow::Result<LinearSpamModel> {
    let header: ModelHeader = serde_json::from_str(json)?;
    match header.analyzer.as_str() {
        "word_12_lower" => load_legacy_model(json),
        UNICODE_WORD_CHAR_ANALYZER => load_unicode_model(json),
        _ => anyhow::bail!("unsupported linear spam analyzer"),
    }
}

fn load_legacy_model(json: &str) -> anyhow::Result<LinearSpamModel> {
    let export: LinearSpamExport = serde_json::from_str(json)?;
    if !export.intercept.is_finite() {
        anyhow::bail!("linear spam intercept must be finite");
    }
    let vocab = export
        .vocab
        .into_iter()
        .map(|(name, idf)| (name, f64::from(idf)))
        .collect();
    let coef = export.coef.into_iter().map(f64::from).collect();
    Ok(LinearSpamModel {
        version: export.version,
        intercept: f64::from(export.intercept),
        calibration: LinearScoreCalibration::default(),
        features: FeatureModel::Legacy(load_block(FeatureBlockExport {
            vocab,
            coef,
            weight: 1.0,
        })?),
    })
}

fn load_unicode_model(json: &str) -> anyhow::Result<LinearSpamModel> {
    let export: UnicodeWordCharExport = serde_json::from_str(json)?;
    export.calibration.validate().map_err(anyhow::Error::msg)?;
    if export.analyzer != UNICODE_WORD_CHAR_ANALYZER
        || export.preprocessing != PREPROCESSING_VERSION
    {
        anyhow::bail!("unsupported linear spam preprocessing version");
    }
    if export.version.trim().is_empty() || !export.intercept.is_finite() {
        anyhow::bail!("linear spam model requires a version and finite intercept");
    }
    if export.word.vocab.is_empty() || export.character.vocab.is_empty() {
        anyhow::bail!("unicode word/char model requires both nonempty feature blocks");
    }
    Ok(LinearSpamModel {
        version: export.version,
        intercept: export.intercept,
        calibration: export.calibration,
        features: FeatureModel::UnicodeWordChar {
            word: load_block(export.word)?,
            character: load_block(export.character)?,
        },
    })
}

fn load_block(export: FeatureBlockExport) -> anyhow::Result<FeatureBlock> {
    if export.vocab.len() != export.coef.len() {
        anyhow::bail!("linear spam vocab/coef length mismatch");
    }
    if !export.coef.iter().all(|value| value.is_finite())
        || !export.weight.is_finite()
        || export.weight < 0.0
    {
        anyhow::bail!("linear spam weights must be finite and block weight nonnegative");
    }
    let mut names: Vec<&String> = export.vocab.keys().collect();
    names.sort();
    // Coefficients follow get_feature_names_out (lexicographic order),
    // so term indices match coefficient positions.
    let mut terms = HashMap::with_capacity(names.len());
    let mut idf = Vec::with_capacity(names.len());
    for name in names {
        let weight = export.vocab[name];
        if !weight.is_finite() || weight < 0.0 {
            anyhow::bail!("linear spam idf is not a valid weight");
        }
        terms.insert((*name).clone(), idf.len());
        idf.push(weight);
    }
    Ok(FeatureBlock {
        terms,
        idf,
        coef: export.coef,
        weight: export.weight,
    })
}

fn tokenize(text: &str) -> Vec<String> {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|character: char| !(character.is_alphanumeric() || character == '_'))
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    let mut tokens = Vec::with_capacity(words.len() * 2);
    for word in &words {
        tokens.push(word.clone());
    }
    for pair in words.windows(2) {
        tokens.push(format!("{} {}", pair[0], pair[1]));
    }
    tokens
}

pub fn spam_probability(model: &LinearSpamModel, text: &str) -> f64 {
    let dot = match &model.features {
        FeatureModel::Legacy(block) => block_dot(block, tokenize(text)),
        FeatureModel::UnicodeWordChar { word, character } => {
            let text = prepare_text(text).model_text();
            block_dot(word, word_tokens(&text)) + block_dot(character, char_tokens(&text))
        }
    };
    sigmoid(model.intercept + dot)
}

fn block_dot(block: &FeatureBlock, tokens: Vec<String>) -> f64 {
    let mut counts: HashMap<usize, f64> = HashMap::new();
    for token in tokens {
        if let Some(index) = block.terms.get(&token) {
            *counts.entry(*index).or_default() += 1.0;
        }
    }
    if counts.is_empty() {
        return 0.0;
    }
    let mut weighted: Vec<(usize, f64)> = Vec::with_capacity(counts.len());
    for (index, count) in counts {
        let value = count * block.idf[index];
        weighted.push((index, value));
    }
    weighted.sort_by_key(|&(index, _)| index);
    let norm_sq: f64 = weighted.iter().map(|(_, value)| value * value).sum();
    let norm = norm_sq.sqrt();
    if norm == 0.0 {
        return 0.0;
    }
    let mut dot = 0.0;
    for (index, value) in weighted {
        dot += block.coef[index] * (value / norm);
    }
    dot * block.weight
}

fn sigmoid(value: f64) -> f64 {
    1.0 / (1.0 + (-value).exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenization_matches_sklearn_word_12() {
        let tokens = tokenize("Недавно ехал за рулем слушал книгу Время - деньги");
        assert_eq!(
            &tokens[..12],
            &[
                "недавно",
                "ехал",
                "за",
                "рулем",
                "слушал",
                "книгу",
                "время",
                "деньги",
                "недавно ехал",
                "ехал за",
                "за рулем",
                "рулем слушал"
            ]
        );
    }

    #[test]
    fn empty_text_falls_back_to_intercept() {
        let model = tiny_model();
        assert!((spam_probability(&model, "") - sigmoid(0.5)).abs() < 1e-9);
    }

    fn tiny_model() -> LinearSpamModel {
        // Coefficients follow sorted vocabulary (get_feature_names_out):
        // ["мир", "привет"] -> [-1.0, 1.0].
        load_model(
            r#"{"version":"test","analyzer":"word_12_lower","vocab":{"привет":1.5,"мир":2.0},"coef":[-1.0,1.0],"intercept":0.5}"#,
        )
        .expect("tiny model must load")
    }

    #[test]
    fn tiny_model_scores_by_hand() {
        // For "привет мир": pre-normalization TF-IDF [1.5, 2.0], norm 2.5,
        // dot = 1.5/2.5*1.0 + 2.0/2.5*(-1.0) + 0.5 = 0.3
        let model = tiny_model();
        let expected = 1.0 / (1.0 + (-0.3f64).exp());
        assert!((spam_probability(&model, "привет мир") - expected).abs() < 1e-9);
    }

    #[test]
    fn rejects_mismatched_exports() {
        assert!(
            load_model(
                r#"{"version":"x","analyzer":"other","vocab":{},"coef":[],"intercept":0.0}"#
            )
            .is_err()
        );
        assert!(load_model(r#"{"version":"x","analyzer":"word_12_lower","vocab":{"a":1.0},"coef":[],"intercept":0.0}"#).is_err());
    }

    fn unicode_export() -> serde_json::Value {
        serde_json::json!({
            "version": "synthetic-word-char-v2",
            "analyzer": UNICODE_WORD_CHAR_ANALYZER,
            "preprocessing": PREPROCESSING_VERSION,
            "calibration": LinearScoreCalibration::default(),
            "word": {"vocab": {"доход": 1.0, "лс": 2.0}, "coef": [1.0, -1.0], "weight": 1.0},
            "character": {"vocab": {"дох": 3.0, "лс ": 4.0}, "coef": [2.0, 0.0], "weight": 0.5},
            "intercept": 0.2,
        })
    }

    #[test]
    fn unicode_blocks_have_separate_l2_norms_and_normalize_before_inference() {
        let model = load_model(&unicode_export().to_string()).unwrap();
        let expected = sigmoid(0.2 - 1.0 / 5.0_f64.sqrt() + 0.6);
        assert!((spam_probability(&model, "дoход лс") - expected).abs() < 1e-12);
        assert!((spam_probability(&model, "") - sigmoid(0.2)).abs() < 1e-12);
    }

    #[test]
    fn unicode_loader_rejects_unknown_preprocessing_extra_features_and_bad_weights() {
        let mut invalid = unicode_export();
        invalid["preprocessing"] = serde_json::json!("unknown-version");
        assert!(load_model(&invalid.to_string()).is_err());
        let mut invalid = unicode_export();
        invalid["flags"] = serde_json::json!([1.0]);
        assert!(load_model(&invalid.to_string()).is_err());
        let mut invalid = unicode_export();
        invalid["character"]["weight"] = serde_json::json!(-1.0);
        assert!(load_model(&invalid.to_string()).is_err());
        let mut invalid = unicode_export();
        invalid["word"]["coef"] = serde_json::json!([1.0]);
        assert!(load_model(&invalid.to_string()).is_err());
    }

    #[test]
    fn legacy_model_keeps_raw_word_view_and_zero_idf_stays_finite() {
        let model = load_model(r#"{"version":"legacy","analyzer":"word_12_lower","vocab":{"доход":1.0},"coef":[1.0],"intercept":0.0}"#).unwrap();
        assert_eq!(spam_probability(&model, "дoход"), 0.5);
        let zero = load_model(r#"{"version":"zero","analyzer":"word_12_lower","vocab":{"доход":0.0},"coef":[1.0],"intercept":0.0}"#).unwrap();
        assert_eq!(spam_probability(&zero, "доход"), 0.5);
        let constant = load_model(r#"{"version":"constant","analyzer":"word_12_lower","vocab":{},"coef":[],"intercept":0.5}"#).unwrap();
        assert_eq!(spam_probability(&constant, "текст"), sigmoid(0.5));
    }
}
