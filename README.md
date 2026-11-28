# teloxide-antispam

An independent Rust anti-spam library for Telegram and community bots.
No dependency on a specific bot, teloxide fork, SQL schema, LLM, or chat.
Consumers supply typed snapshots and receive risk scores and observable
signals. Data collection, persistence, authorization, and enforcement remain
the consuming application's responsibility.

## Features

| Feature | Functionality |
|---|---|
| No defaults | Text helpers, calibration, categories taxonomy, action policy, CAS response parsing |
| `unicode` | Versioned Unicode model view and word/character tokenization |
| `unicode-v3` | Opt-in ICU4X 2.4 / Unicode 18 preprocessing and UTS #39 skeleton, while retaining V2 for existing model artifacts |
| `classifier` | TF-IDF/LogReg v1/v2 loading and inference; enables `unicode` |
| `embedding` | Frozen-encoder linear heads: binary spam plus multilabel categories |
| `scoring` | Assessment, profile/ID/channel signals, component scoring; enables `unicode` |
| `cas` | Optional CAS HTTP lookup; no networking dependency otherwise |
| `url` | Optional `teloxide-url` parser; URL data is not scored automatically |
| `flood` | Pure event-time EWMA and bounded conversation-window metrics; no Telegram or enforcement |

ICU4X 2.4.0 requires Rust 1.88 or newer when `unicode-v3` is enabled. Other
feature sets do not activate the direct ICU4X dependencies.

Default features: `scoring`, `classifier`, `embedding`. Minimal classifier integration:

The opt-in `flood` feature contains evidence-only temporal primitives and a
bounded conversation snapshot analyzer. It does not persist state, select
moderation thresholds, or enforce actions. See [FLOOD.md](docs/FLOOD.md) for
event-time, completeness, and resource-bound contracts.

```toml
teloxide-antispam = { git = "https://github.com/Mar2ianen/teloxide-antispam", tag = "v0.2.0", default-features = false, features = ["classifier"] }
```

Consumers configure review thresholds, ID model parameters, snapshots, and
reputation. Assessment DTOs do not require an LLM. CAS requires an explicit
lookup; the library never starts jobs or workers. `policy::decide_auto_action`
returns a proposed action without deleting messages or banning accounts.
The inherited baseline policy is heuristic; consumers must validate it for
their own community instead of treating it as a universal calibrated detector.

`NewUserFeatures::local_spammer_identity` represents a current confirmed local
label, not a Telegram ban. It adds 70 baseline points and keeps profile/message
analysis enabled even for an established participant. Local and shared identity
confirmations count once; clearing the local label removes its contribution.

## Text models

`logreg::load_model` distinguishes legacy `word_12_lower` from
`unicode_word_char_v2` / `unicode-spam-v2`. The opt-in `unicode-v3` feature
adds `unicode_word_char_v3` / `unicode-spam-v3-icu4x-2.4-uts39-18-fold-first`; its model
metadata must match exactly. V3 uses ICU4X 2.4.0 Unicode 18 properties,
normalization, full default case folding, and ICU 18 word/whitespace
tokenization; it folds case before applying UTS #39 confusable skeletons. It
intentionally changes model text (for example, `Straße` folds
to `strasse`, and mixed-script confusables can map to a common skeleton).
Existing V2 artifacts must continue to use V2 and cannot be relabeled or
converted by changing metadata. V3 requires training and validating a new
model against V3 model views. The `unicode-v3` feature retains V2 support for
old artifacts and therefore currently compiles both Unicode stacks.

Legacy inference preserves raw
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

Without a model argument it defaults to V2; select V3 with
`--preprocessing v3`. It emits the selected preprocessing view and exact
word/character tokens. With a text model it follows that model's preprocessing
metadata and emits the version and actual tokens used for scoring. With an
embedding head it also outputs `embedding_probability` and `fused_probability`
(equal-weight logit fusion). No Telegram or SQL requests are made. Models and
private corpora are not embedded in the library.
For the ported v2 candidate, Python/Rust parity was checked on 32,253 texts:
zero model-view differences and maximum probability error 3.11e-15. This is
an artifact-specific result, not a guarantee for every Unicode code point.

## Operating-point selection

The optional public ALT model is distributed as a release asset, not embedded
in the crate. See [its model card](docs/ALT_MODEL_CARD.md) for provenance,
checksum, metrics, and limitations. The optional Gemma embedding head is
distributed the same way; see [its model card](docs/GEMMA_HEAD_CARD.md).
It requires document-prefixed vectors and an `embedding_model` match.

The [training methodology](docs/TRAINING.md) documents preprocessing, split
policy, feature extraction, fitting, and export without redistributing data.
Datasets, chat exports, moderator logs, raw predictions, and identifying
examples are not published. Public materials are code, methodology, aggregate
metrics, and model weights. Contact the maintainer for data-access questions;
access is reviewed case by case and is not an automatic redistribution grant.

```sh
python3 tools/calibrate.py predictions.jsonl --version my-validation-v1 --output calibration.json
```

Input rows contain `split` (`validation`/`test`), `label` (0 ham / 1 spam),
and `probability`. Thresholds use validation only: supporting allows at most
two false positives with a floor of 0.9; strong requires zero false positives.
Test is used exclusively for reporting. Zero errors on a small holdout do not
establish FPR ≤0.0001%. Consumers still need an independently adjudicated,
temporal, author/campaign-separated, profile-aware local holdout.

## Categories and embedding heads

`categories` defines the stable append-only taxonomy: `job_scam`,
`finance_crypto_promo`, `adult_funnel`, `vpn_promo`, `direct_dm_funnel`,
`external_promo`. Scores are independent probabilities, not a softmax.
Categories are observation-only: they route review and explain a verdict,
they never add points or authorize a ban. Without a trained head, scoring
reports heuristic `suspected_categories` names from assessment markers;
names are not probabilities and must not be treated as learned outputs.

`embedding` scores frozen encoder vectors (e.g. EmbeddingGemma) with a
versioned linear layer per head: one binary spam head plus optional
per-category heads. The encoder runs outside this library; only the heads
are learned here. In scoring, text and embedding heads share a single
supporting slot via `max()`, so two weak models cannot stack into a ban.
`fuse_probabilities` offers an alternative equal-footing logit combiner for
offline analysis. Train heads with:

```sh
python3 tools/train_embedding_multitask.py rows.jsonl --dim 768 \
  --embedding-model embeddinggemma-q4-768-v1 --version my-head-v1 \
  --output head.json
```

Input rows are private (`embedding` floats or base64 f32, `label` 0/1,
optional `categories` names). Keep datasets, exports, and per-example
predictions out of Git; publish the head weights, versions, aggregate
metrics, and methodology instead.

## Development

```sh
cargo fmt --all -- --check
cargo test --all-targets --all-features
cargo clippy --all-targets --all-features -- -D warnings
python3 -m unittest discover -s tools -p 'test_*.py'
cargo test --no-default-features --features unicode-v3 --lib
cargo check --manifest-path fuzz/Cargo.toml --locked
```

The Unicode 18 conformance tests are fully offline and use pinned official
Unicode data under `test-data/unicode/18.0.0/`; source URLs, licenses, and
SHA-256 digests are recorded in that directory. The `unicode-v3` CI job runs
the same corpus checks and a 512-case property test. V2 remains covered by
the ordinary `unicode` and default feature jobs.

The pinned Unicode 18 differential run currently finds 270 NFKC input-column
differences out of 100,855 normalization-corpus inputs versus
`unicode-normalization` 0.1.25, and 212 differing mappings out of 1,606 versus
Rust `to_lowercase()`. These are expected Unicode-version/full-case-fold
changes; ICU4X output is checked against the official corpus. Exhaustive
scalar comparison also finds four V2/Unicode 18 whitespace differences
(Python-compatible U+001C–U+001F) and 258 variation-selector differences
(V2 only strips FE0E/FE0F in words; V3 uses the full Unicode property); the
Bidi_Control set has zero differences. V3 fixes the sequence as case-fold then UTS #39 skeleton, so `Þ` maps to
`p` in one pass. Do not feed already-preprocessed model text back as if it
were raw input.

`examples/bench_preprocess.rs` compares complete `prepare_text` V2 and V3
calls on identical generated corpora with Rust 1.100.0. It takes the median of
19 samples; each sample processes approximately 2 MiB (two operations for
the 1 MiB inputs). V3 uses the current fold-first contract. Reproduce with
`cargo run --release --example bench_preprocess --no-default-features
--features classifier,unicode-v3 --locked`.

| Corpus | Bytes | V2 latency / MiB/s | V3 latency / MiB/s |
|---|---:|---:|---:|
| ASCII | 8,184 | 253,978,630 ns / 7.87 | 230,017,207 ns / 8.69 |
| Cyrillic | 28,600 | 177,156,678 ns / 11.24 | 144,601,208 ns / 13.77 |
| Mixed Unicode | 16,506 | 176,345,189 ns / 11.34 | 146,779,605 ns / 13.62 |
| Emoji-heavy | 18,426 | 76,995,467 ns / 25.79 | 57,065,440 ns / 34.80 |
| Indic adversarial, near 1 MiB | 1,048,575 | 119,520,780 ns / 16.73 | 85,355,446 ns / 23.43 |
| Rich-like, near 1 MiB | 1,048,560 | 201,513,130 ns / 9.92 | 174,579,463 ns / 11.46 |

These are local microbenchmark results, not a bot-level latency or production
memory guarantee. V3 remains opt-in pending training on its own model view and
consumer-level evaluation.
Separate isolated release consumers (one enabling only `unicode`, one only
`unicode-v3`) measured clean builds at 5.926 s / 8.513 s and no-op incremental
builds at 0.016 s / 0.022 s. Their binaries were 3,039,272 / 3,555,536 bytes;
`.text` was 1,120,024 / 1,152,920 bytes and `.rodata` 492,588 / 634,628 bytes.
V3 therefore adds about 516 KiB to this small executable while it retains V2
support and its dependencies. These are standalone consumer measurements,
not full application build or size numbers.

CI also tests each feature independently. Licensed under MIT.
