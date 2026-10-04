"""Fit binary + multilabel logistic heads on FROZEN embeddings.

Inputs are private and never published: a JSONL with base64/float embedding
vectors, a binary spam label, and optional category labels from the
SpamCategory taxonomy (job_scam, finance_crypto_promo, adult_funnel,
vpn_promo, direct_dm_funnel, external_promo).

Output is a versioned JSON head pack loadable by
`teloxide_antispam::embedding::EmbeddingSpamModel`. The encoder is never
fine-tuned here; only linear heads are learned. Thresholds still come from
`tools/calibrate.py` on validation probabilities.
"""

import argparse
import base64
import json
import struct
from pathlib import Path

import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.multiclass import OneVsRestClassifier
from sklearn.preprocessing import normalize


CATEGORIES = [
    "job_scam",
    "finance_crypto_promo",
    "adult_funnel",
    "vpn_promo",
    "direct_dm_funnel",
    "external_promo",
]


def load_rows(path):
    rows = []
    for line in Path(path).read_text().splitlines():
        if not line.strip():
            continue
        rows.append(json.loads(line))
    return rows


def decode_vector(raw, dim):
    if isinstance(raw, dict) and raw.get("b64"):
        values = struct.unpack(f"<{dim}f", base64.b64decode(raw["b64"]))
        return np.array(values, dtype=np.float64)
    vector = np.asarray(raw, dtype=np.float64)
    assert vector.shape == (dim,), f"bad dim {vector.shape}"
    return vector


def fit_heads(matrix, binary, categories=None, seed=42):
    binary_head = LogisticRegression(max_iter=1000, random_state=seed)
    binary_head.fit(matrix, binary)
    category_heads = {}
    if categories is not None:
        per_class = OneVsRestClassifier(
            LogisticRegression(max_iter=1000, random_state=seed)
        )
        per_class.fit(matrix, categories)
        for name, estimator in zip(CATEGORIES, per_class.estimators_):
            if hasattr(estimator, "coef_"):
                category_heads[name] = {
                    "weights": [float(w) for w in estimator.coef_[0]],
                    "intercept": float(estimator.intercept_[0]),
                }
            else:
                # Single-class column (e.g. never observed): a strongly
                # negative constant head keeps the export loadable and silent.
                category_heads[name] = {
                    "weights": [0.0] * matrix.shape[1],
                    "intercept": -10.0,
                }
    return {
        "spam": {
            "weights": [float(w) for w in binary_head.coef_[0]],
            "intercept": float(binary_head.intercept_[0]),
        },
        "categories": category_heads,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rows", type=Path, help="private JSONL, see module docstring")
    parser.add_argument("--dim", type=int, required=True)
    parser.add_argument("--embedding-model", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--normalize", action="store_true", default=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--calibration",
        default=None,
        help="optional JSON object with version/supporting_threshold/strong_threshold/supporting_score/strong_score from tools/calibrate.py; omitted for loader-default legacy calibration",
    )
    args = parser.parse_args()

    rows = load_rows(args.rows)
    assert len(rows) >= 10, "need at least a few adjudicated rows"
    matrix = normalize(
        np.stack([decode_vector(r["embedding"], args.dim) for r in rows])
    ) if args.normalize else np.stack(
        [decode_vector(r["embedding"], args.dim) for r in rows]
    )
    assert np.isfinite(matrix).all()
    binary = np.array([r["label"] for r in rows])
    assert set(np.unique(binary)) == {0, 1}, "both classes required"
    categories = None
    if any("categories" in r for r in rows):
        categories = np.array(
            [[1 if name in r.get("categories", []) else 0 for name in CATEGORIES]
             for r in rows]
        )
        assert categories.shape == (len(rows), len(CATEGORIES))
    heads = fit_heads(matrix, binary, categories)
    export = {
        "version": args.version,
        "embedding_model": args.embedding_model,
        "dim": args.dim,
        "normalize": args.normalize,
        **heads,
    }
    if args.calibration is not None:
        export["calibration"] = json.loads(args.calibration)
    args.output.write_text(json.dumps(export) + "\n")


if __name__ == "__main__":
    main()
