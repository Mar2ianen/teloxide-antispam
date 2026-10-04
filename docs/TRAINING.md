# Training without publishing datasets

This repository distributes methods and models, not datasets. Obtain public
data from its upstream publisher under its license, or use a locally owned,
appropriately adjudicated corpus. Keep private exports, labels, moderator
logs, IDs, identifying examples, and per-example predictions outside Git.
Requests for access should be directed to the maintainer and reviewed for
consent, privacy, provenance, and redistribution rights. Requesting access
does not imply that any third-party/private dataset will be provided.

## ALT v2 recipe

The released artifact uses only the published ALT labels linked in the
[model card](ALT_MODEL_CARD.md). Private challenge data never enters fitting.

1. Validate class semantics: 0 = ham, 1 = spam. Drop empty texts.
2. Normalize using `preprocess::prepare_text` / `unicode-spam-v2`. Preserve raw
   text privately. Model input is canonical text plus newline and the compact
   auxiliary view when present. The Rust `score_text` example without a model
   outputs canonical/compact/flags as JSONL; consumers can construct this model
   input without a separate Python normalization implementation.
3. For template grouping only, replace URLs/advertised contacts with a contact
   marker and numbers with a number marker; collapse whitespace and hash the
   result. Exclude groups with conflicting class labels. Never use these masked
   grouping strings as the classifier's input.
4. Sort group IDs. Make a stratified group 60/40 split (`random_state=42`), then
   split the 40% holdout evenly into validation/test (`random_state=43`). Dedup
   canonical model inputs within each split. Assert no group crosses splits.
   For a real community corpus, additionally enforce temporal, author, and
   campaign separation; ALT text-only grouping does not establish those.
5. Fit vocabulary and IDF on training only; never fit preprocessing statistics
   on validation, test, or unreviewed challenge texts.

Given private, already prepared model inputs `x_train` and labels `y_train`,
the fitting parameters are:

```python
from sklearn.feature_extraction.text import TfidfVectorizer
from sklearn.linear_model import LogisticRegression
from sklearn.pipeline import FeatureUnion

common = dict(min_df=3, max_features=60000, lowercase=False,
              norm="l2", sublinear_tf=False)
features = FeatureUnion([
    ("word", TfidfVectorizer(ngram_range=(1, 2),
                            token_pattern=r"(?u)\w+", **common)),
    ("char", TfidfVectorizer(analyzer="char_wb",
                            ngram_range=(3, 5), **common)),
], transformer_weights={"word": 1.0, "char": 1.0})
matrix = features.fit_transform(x_train)
classifier = LogisticRegression(C=4.0, max_iter=1000,
                                solver="liblinear", random_state=42)
classifier.fit(matrix, y_train)
```

Single-letter words are retained. Each TF-IDF block has its own L2 norm;
do not add another norm over the concatenated feature vector. Unicode flags
are excluded from weighted features: they are audit observations only.

## Operating points and export

Compute probabilities on untouched validation/test inputs using the fitted
feature extractors. Keep probability-only JSONL locally (`split`, `label`,
`probability`), then run `tools/calibrate.py` as documented in the README.
Only validation selects thresholds. Test is reporting-only; disclose any prior
test-guided candidate selection. This is operating-point selection, not Platt
scaling, calibrated posterior probabilities, or full-system risk calibration.

For the Rust v2 artifact:

- Set `analyzer="unicode_word_char_v2"`, `preprocessing="unicode-spam-v2"`,
  an explicit model `version`, and the validated `calibration` object.
- For each `word` / `character` block, export `vocab` as term → IDF,
  `coef` in lexicographic term order, and its transformer `weight`.
- Coefficients come from the corresponding segment of the concatenated
  classifier coefficient vector, reordered through the vectorizer's vocabulary
  indices. Do not assume feature-union or vocabulary insertion order is sorted.
- Export the common `intercept` and numeric values as f64. Confirm classes are
  `[0, 1]`, dimensions match, and all weights/IDFs/intercept are finite.
- Compare Python probabilities with the Rust JSONL scorer; also compare
  canonical text, compact view, observation counters, and confusion counts.
- Version the artifact and record its SHA256, upstream provenance, split policy,
  aggregate metrics, limitations, and preprocessing version. Publish weights,
  not the training rows or identifying test examples.

Keep automatic enforcement disabled until separately validated on an
independently adjudicated, profile-aware local holdout. A strong text score is
supporting evidence, never standalone permission to ban an account.

## Embedding multitask heads

Encode texts with the frozen production encoder (record its exact model
version, e.g. `embeddinggemma-q4-768-v1`) and store vectors privately
alongside adjudicated binary labels and category names. L2-normalize the
same way at train and inference time (`normalize: true` unless the encoder
already emits unit vectors). Fit one logistic head for spam and one per
observed category with `tools/train_embedding_multitask.py`; unseen
categories export as silent constant heads. Validate vector dim, finiteness,
and class coverage before export, then compare Python probabilities against
the Rust `score_text` harness with base64 `embedding` rows. Select
supporting thresholds with `tools/calibrate.py` on validation only, exactly
as for the text model. Category heads route review and explain verdicts;
they never add points. Publish head weights and versions, never the
underlying vectors, texts, or labels.
