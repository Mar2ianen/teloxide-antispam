# teloxide-antispam

An independent Rust anti-spam library for Telegram and community bots.
No dependency on a specific bot, teloxide fork, SQL schema, LLM, or chat.
Consumers supply typed snapshots and receive risk scores and observable
signals. Data collection, persistence, authorization, and enforcement remain
the consuming application's responsibility.

## Features

| Feature | Functionality |
|---|---|
| No defaults | Text helpers, calibration, action policy, CAS response parsing |
| `unicode` | Versioned Unicode model view and word/character tokenization |
| `classifier` | TF-IDF/LogReg v1/v2 loading and inference; enables `unicode` |
| `scoring` | Assessment, profile/ID/channel signals, component scoring; enables `unicode` |
| `cas` | Optional CAS HTTP lookup; no networking dependency otherwise |

Default features: `scoring`, `classifier`. Minimal classifier integration:

```toml
teloxide-antispam = { git = "https://github.com/Mar2ianen/teloxide-antispam", tag = "v0.2.0", default-features = false, features = ["classifier"] }
```

Consumers configure review thresholds, ID model parameters, snapshots, and
reputation. Assessment DTOs do not require an LLM. CAS requires an explicit
lookup; the library never starts jobs or workers. `policy::decide_auto_action`
returns a proposed action without deleting messages or banning accounts.
The inherited baseline policy is heuristic; consumers must validate it for
their own community instead of treating it as a universal calibrated detector.

## Text models

`logreg::load_model` distinguishes legacy `word_12_lower` from
`unicode_word_char_v2` / `unicode-spam-v2`. Legacy inference preserves raw
word 1–2 features and f32 weights. V2 uses NFKC, token-local homoglyph handling,
invisible/bidi/Zalgo cleanup, an auxiliary spread-word view, and separately
L2-normalized word 1–2 and char_wb 3–5 blocks. Original text is untouched.
Unicode observations **never add risk points**. Emoji ZWJ sequences and
ordinary composed letters/accents are preserved.

V2 requires an explicit `calibration` object: `version`, `supporting_threshold`,
`strong_threshold`, `supporting_score`, and `strong_score`. Invalid versions,
weights, or thresholds are rejected. Selecting operating points does not
calibrate posterior probabilities or establish full-system performance.
The classifier remains supporting evidence, not a ban authorization.

JSONL parity harness (`id` and `text` on stdin):

```sh
cargo run --example score_text -- path/to/model.json
```

Without a model argument it outputs preprocessing only. No Telegram or SQL
requests are made. Models and private corpora are not embedded in the library.
For the ported v2 candidate, Python/Rust parity was checked on 32,253 texts:
zero model-view differences and maximum probability error 3.11e-15. This is
an artifact-specific result, not a guarantee for every Unicode code point.

## Operating-point selection

The optional public ALT model is distributed as a release asset, not embedded
in the crate. See [its model card](docs/ALT_MODEL_CARD.md) for provenance,
checksum, metrics, and limitations.

```sh
python3 tools/calibrate.py predictions.jsonl --version my-validation-v1 --output calibration.json
```

Input rows contain `split` (`validation`/`test`), `label` (0 ham / 1 spam),
and `probability`. Thresholds use validation only: supporting allows at most
two false positives with a floor of 0.9; strong requires zero false positives.
Test is used exclusively for reporting. Zero errors on a small holdout do not
establish FPR ≤0.0001%. Consumers still need an independently adjudicated,
temporal, author/campaign-separated, profile-aware local holdout.

## Development

```sh
cargo fmt --all -- --check
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
python3 -m unittest discover -s tools -p 'test_*.py'
```

CI also tests each feature independently. Licensed under MIT.
