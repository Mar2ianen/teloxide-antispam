# Gemma prefixed head — evaluation and operating points

Model: `gemma-768-fx-2026-10-04`.
Encoder: `ggml-org/embeddinggemma-300M-qat-q4_0-GGUF` (frozen, 768 dim,
mean pooling, L2-normalized vectors). The encoder runs outside this library.
Head input must be the **document-prefixed** text
(`title: none | text: {text}`), matching the consumer's stored retrieval
vectors; raw-text vectors are a different distribution for this head.
Calibration: `gemma-fx-validation-2026-10-04-v1`.
Release asset: `gemma_768_fx_2026-10-04.json`.
SHA256: `123665884661c0628b2a3ea9ac75e2f69ecb784ab61ec0fb55c3e09636490823`.

## Data and selection

Training uses only published labels from
[`alt-gnome/telegram-spam-20251030`](https://huggingface.co/datasets/alt-gnome/telegram-spam-20251030),
declared CC0, on the exact grouped 60/20/20 split of the TF-IDF ablation
(train 19,471 / validation 6,469 / test 6,485). Only a binary spam head was
fitted (`LogisticRegression`, default `C=1.0`, `max_iter=1000`); category
heads require adjudicated category labels that do not exist yet, so the
artifact ships with an empty category set. Thresholds below were selected on
validation only with `tools/calibrate.py` (supporting: at most 2 FP, floor
0.9; strong: zero FP).

## Test metrics (3,180 spam / 3,305 ham)

| Band | Threshold | Points | Validation TP / FP / FN / TN | Test TP / FP / FN / TN |
|---|---:|---:|---|---|
| Supporting | 0.9 | 10 | 3015 / 0 / 149 / 3305 | 3037 / 0 / 143 / 3305 |
| Strong supporting | 0.9 | 18 | 3015 / 0 / 149 / 3305 | 3037 / 0 / 143 / 3305 |

Supporting test recall: **95.50%** with **0 FP**. Both bands coincide
because validation showed zero false positives at the 0.9 floor. For
comparison on the same split: TF-IDF word/char 93.87% / 2 FP supporting and
84.50% / 0 FP strong; equal-weight logit fusion of TF-IDF and this head
reaches 98.52% / 2 FP supporting and 95.38% / 0 FP strong (offline analysis
only, not a deployed combiner).

The one-sided 95% zero-FP bound over 3,305 ham is approximately **0.0906%**
assuming independent representative examples; correlation weakens that
assumption. None of these numbers establishes 99.9% recall or FPR ≤0.0001%,
nor full moderation performance. The TF-IDF candidate was test-inspected
during selection, so this test set is not pristine.

## Intended use and limitations

Dry-run supporting evidence (at most 18 points shared with the text head via
`max()`, never stacking), audit, and manually reviewed decisions. Not
standalone bans. Consumers must check the stored vector's `embedding_model`
before scoring and treat a missing or mismatched vector as no evidence.
Vectors reflect the message text at encode time; edited messages keep stale
vectors. Local and production encoder builds agree to cosine 0.99986 (median
over 64 probes); the head was trained on local-build vectors, so borderline
probabilities may shift slightly under the production build — dry-run
observation absorbs this. Consumers still need an independently adjudicated,
profile-aware, temporal/author/campaign-separated local holdout before
permitting automatic enforcement.

Rust/Python parity: 300 test vectors, maximum probability error 5.6e-16.
No private texts, vectors, or labels are included in the release.
