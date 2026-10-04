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
    let model = std::env::args()
        .nth(1)
        .map(|path| {
            std::fs::read_to_string(path)
                .map_err(anyhow::Error::from)
                .and_then(|json| logreg::load_model(&json))
        })
        .transpose()?;
    let embedding_head = std::env::args()
        .nth(2)
        .map(|path| {
            std::fs::read_to_string(path)
                .map_err(anyhow::Error::from)
                .and_then(|json| embedding::EmbeddingSpamModel::load(&json))
        })
        .transpose()?;
    let stdin = io::stdin();
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let input: Input = serde_json::from_str(&line?)?;
        let prepared = preprocess::prepare_text(&input.text);
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
                "probability": text_probability,
                "embedding_probability": embedding_probability,
                "fused_probability": embedding::fuse_probabilities(text_probability, embedding_probability, 1.0, 1.0, 0.0),
            }),
        )?;
        writeln!(output)?;
    }
    Ok(())
}
