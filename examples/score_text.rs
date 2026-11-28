//! Local parity harness: JSONL stdin to JSONL stdout; no Telegram or SQL.
//! No input logging. Without a model argument, outputs preprocessing only.
//! Optional second argument scores a frozen-embedding head from base64
//! `embedding` rows: `{"id":…, "text":…, "embedding": "<b64 little-endian f32>"}`.

use std::io::{self, BufRead, Write};

use serde::Deserialize;
use serde_json::{Value, json};
use teloxide_antispam::{embedding, logreg, preprocess};

#[derive(Deserialize)]
struct Input {
    id: Value,
    text: String,
    embedding: Option<String>,
}

fn decode_b64_f32(value: &str) -> anyhow::Result<Vec<f32>> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(value)?;
    if bytes.len() % 4 != 0 {
        anyhow::bail!("embedding bytes are not f32-aligned");
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect())
}

fn main() -> anyhow::Result<()> {
    let mut positional = Vec::new();
    let mut requested_preprocessing = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--preprocessing" {
            requested_preprocessing = Some(
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("--preprocessing requires v2 or v3"))?,
            );
        } else if let Some(value) = arg.strip_prefix("--preprocessing=") {
            requested_preprocessing = Some(value.to_owned());
        } else {
            positional.push(arg);
        }
    }
    let model = positional
        .first()
        .map(|path| {
            std::fs::read_to_string(path)
                .map_err(anyhow::Error::from)
                .and_then(|json| logreg::load_model(&json))
        })
        .transpose()?;
    let embedding_head = positional
        .get(1)
        .map(|path| {
            std::fs::read_to_string(path)
                .map_err(anyhow::Error::from)
                .and_then(|json| embedding::EmbeddingSpamModel::load(&json))
        })
        .transpose()?;
    let preprocessing = if let Some(model) = &model {
        let model_version = model
            .preprocessing_version()
            .unwrap_or("legacy-word-12-lower");
        if let Some(requested) = requested_preprocessing.as_deref()
            && requested != "v2"
            && requested != "v3"
        {
            anyhow::bail!("--preprocessing must be v2 or v3");
        }
        if let Some(requested) = requested_preprocessing.as_deref()
            && model_version != "legacy-word-12-lower"
            && !((requested == "v2" && model_version == preprocess::PREPROCESSING_VERSION)
                || (requested == "v3" && model_version == preprocess::PREPROCESSING_VERSION_V3))
        {
            anyhow::bail!("--preprocessing conflicts with the model's preprocessing metadata");
        }
        model_version.to_owned()
    } else {
        requested_preprocessing.unwrap_or_else(|| "v2".to_owned())
    };
    if preprocessing != "legacy-word-12-lower"
        && preprocessing != "v2"
        && preprocessing != "v3"
        && preprocessing != preprocess::PREPROCESSING_VERSION
        && preprocessing != preprocess::PREPROCESSING_VERSION_V3
    {
        anyhow::bail!("--preprocessing must be v2 or v3");
    }
    let stdin = io::stdin();
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let input: Input = serde_json::from_str(&line?)?;
        let prepared = match model
            .as_ref()
            .and_then(|model| model.prepare_text(&input.text))
        {
            Some(prepared) => Some(prepared),
            None if preprocessing == "v2" || preprocessing == preprocess::PREPROCESSING_VERSION => {
                Some(preprocess::prepare_text(&input.text))
            }
            #[cfg(feature = "unicode-v3")]
            None if preprocessing == "v3"
                || preprocessing == preprocess::PREPROCESSING_VERSION_V3 =>
            {
                Some(preprocess::prepare_text_v3(&input.text))
            }
            #[cfg(not(feature = "unicode-v3"))]
            None if preprocessing == "v3" => {
                anyhow::bail!("V3 preprocessing requires the unicode-v3 feature")
            }
            None => None,
        };
        let tokens = model.as_ref().map_or_else(
            || {
                let prepared_text = prepared
                    .as_ref()
                    .map(preprocess::PreparedText::model_text)
                    .unwrap_or_default();
                match preprocessing.as_str() {
                    "v2" | preprocess::PREPROCESSING_VERSION => logreg::ModelTokens {
                        word: preprocess::word_tokens(&prepared_text),
                        character: preprocess::char_tokens(&prepared_text),
                    },
                    #[cfg(feature = "unicode-v3")]
                    "v3" | preprocess::PREPROCESSING_VERSION_V3 => logreg::ModelTokens {
                        word: preprocess::word_tokens_v3(&prepared_text),
                        character: preprocess::char_tokens_v3(&prepared_text),
                    },
                    _ => logreg::ModelTokens {
                        word: Vec::new(),
                        character: Vec::new(),
                    },
                }
            },
            |model| model.tokens(&input.text),
        );
        let text_probability = model
            .as_ref()
            .map(|model| logreg::spam_probability(model, &input.text));
        let vector = input.embedding.as_deref().map(decode_b64_f32).transpose()?;
        let embedding_probability = vector.as_deref().and_then(|vector| {
            embedding_head
                .as_ref()
                .and_then(|head| head.spam_probability(vector))
        });
        serde_json::to_writer(
            &mut output,
            &json!({
                "id": input.id,
                "prepared": prepared,
                "preprocessing": preprocessing,
                "tokens": tokens,
                "probability": text_probability,
                "embedding_probability": embedding_probability,
                "fused_probability": embedding::fuse_probabilities(text_probability, embedding_probability, 1.0, 1.0, 0.0),
            }),
        )?;
        writeln!(output)?;
    }
    Ok(())
}
