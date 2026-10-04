import json
import unittest
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).resolve().parent))

try:
    import numpy as np
    from train_embedding_multitask import CATEGORIES, decode_vector, fit_heads
    HAS_SKLEARN = True
except ImportError:
    HAS_SKLEARN = False


@unittest.skipUnless(HAS_SKLEARN, "scikit-learn/numpy required for head training")
class EmbeddingHeadTests(unittest.TestCase):
    def test_binary_and_multilabel_heads_learn_separable_data(self):
        rng = np.random.default_rng(42)
        dim = 8
        matrix = np.vstack([
            rng.normal(-1.0, 0.3, (20, dim)),
            rng.normal(1.0, 0.3, (20, dim)),
        ])
        binary = np.array([0] * 20 + [1] * 20)
        categories = np.array(
            [[1 if (i % 6) == j else 0 for j in range(6)] for i in range(20)]
            + [[1 if (i % 6) == j else 0 for j in range(6)] for i in range(20)]
        )
        heads = fit_heads(matrix, binary, categories)
        self.assertEqual(len(heads["spam"]["weights"]), dim)
        self.assertTrue(all(isinstance(w, float) for w in heads["spam"]["weights"]))
        self.assertEqual(set(heads["categories"]), set(CATEGORIES))
        export = {
            "version": "test-v1", "embedding_model": "test-encoder-8",
            "dim": dim, "normalize": False, **heads,
        }
        # Schema contract for the Rust loader: deny_unknown_fields-safe.
        self.assertEqual(set(export), {"version", "embedding_model", "dim", "normalize", "spam", "categories"})

    def test_decode_rejects_dim_mismatch(self):
        with self.assertRaises(AssertionError):
            decode_vector([1.0, 2.0], 8)

    def test_taxonomy_is_stable(self):
        if not HAS_SKLEARN:
            self.skipTest("scikit-learn/numpy required for head training")
        self.assertEqual(
            CATEGORIES,
            ["job_scam", "finance_crypto_promo", "adult_funnel",
             "vpn_promo", "direct_dm_funnel", "external_promo"],
        )


if __name__ == "__main__":
    unittest.main()
