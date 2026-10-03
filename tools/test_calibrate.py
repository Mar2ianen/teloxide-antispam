import unittest
from calibrate import calibrate


class CalibrationTests(unittest.TestCase):
    def rows(self):
        return [dict(split=s, label=label, probability=p)
                for s in ("validation", "test") for label, p in
                [(0, 0.95), (0, 0.93), (0, 0.4), (1, 0.99), (1, 0.92)]]

    def test_strong_excludes_validation_ham(self):
        result = calibrate(self.rows(), "synthetic-v1")
        self.assertEqual(result["calibration"]["supporting_threshold"], 0.9)
        self.assertGreater(result["calibration"]["strong_threshold"], 0.95)
        self.assertEqual(result["metrics"]["validation"]["strong"]["fp"], 0)

    def test_test_probabilities_do_not_select_thresholds(self):
        rows = self.rows()
        reference = calibrate(rows, "synthetic-v1")["calibration"]
        for row in rows:
            if row["split"] == "test": row["probability"] = 0.999
        self.assertEqual(calibrate(rows, "synthetic-v1")["calibration"], reference)

    def test_rejects_nonfinite_and_missing_class(self):
        rows = self.rows()
        rows[0]["probability"] = float("nan")
        with self.assertRaises(ValueError): calibrate(rows, "v1")
        with self.assertRaises(ValueError): calibrate([r for r in self.rows() if r["label"]], "v1")


if __name__ == "__main__":
    unittest.main()
