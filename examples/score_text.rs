//! Local parity harness: JSONL stdin to JSONL stdout; no Telegram or SQL.
//! No input logging. Without a model argument, outputs preprocessing only.

use std::io::{self, BufRead, Write};

use serde::Deserialize;
use serde_json::{Value, json};
use teloxide_antispam::{logreg, preprocess};

#[derive(Deserialize)]
struct Input {
    id: Value,
    text: String,
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
    let stdin = io::stdin();
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let input: Input = serde_json::from_str(&line?)?;
        let prepared = preprocess::prepare_text(&input.text);
        serde_json::to_writer(
            &mut output,
            &json!({
                "id": input.id,
                "prepared": prepared,
                "probability": model.as_ref().map(|model| logreg::spam_probability(model, &input.text)),
            }),
        )?;
        writeln!(output)?;
    }
    Ok(())
}
