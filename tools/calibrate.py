"""Select thresholds on validation only; test never selects operating points.

JSONL: split=validation|test, label=0|1, probability=number.
No text, IDs, or sources required. Operating-point selection, not Platt scaling.
"""
import argparse
import json
import math
from pathlib import Path


def metrics(rows, threshold):
    result = dict(tp=0, fp=0, fn=0, tn=0)
    for row in rows:
        key = ("tp" if row["label"] else "fp") if row["probability"] >= threshold else ("fn" if row["label"] else "tn")
        result[key] += 1
    ham = result["tn"] + result["fp"]
    spam = result["tp"] + result["fn"]
    result.update(threshold=threshold, recall=result["tp"] / spam, fpr=result["fp"] / ham)
    if not result["fp"]:
        result["zero_fp_one_sided_95_upper_fpr"] = -math.expm1(math.log(0.05) / ham)
    return result


def choose(rows, budget, floor):
    ham = sorted((r["probability"] for r in rows if r["label"] == 0), reverse=True)
    if len(ham) <= budget:
        raise ValueError("insufficient validation ham for requested FP budget")
    # Leave a numerical margin for independent inference implementations.
    threshold = max(floor, ham[budget] + 1e-9)
    if threshold > 1:
        raise ValueError("no feasible threshold with requested FP budget")
    return threshold


def calibrate(rows, version):
    for row in rows:
        if row["split"] not in {"validation", "test"} or type(row["label"]) is not int or row["label"] not in (0, 1):
            raise ValueError("invalid label/split")
        p = row["probability"]
        if not math.isfinite(p) or not 0 <= p <= 1:
            raise ValueError("probability must be finite and within [0,1]")
    cohorts = {s: [r for r in rows if r["split"] == s] for s in ("validation", "test")}
    if any({r["label"] for r in rs} != {0, 1} for rs in cohorts.values()):
        raise ValueError("both classes required in validation and test")
    weak = choose(cohorts["validation"], budget=2, floor=0.9)
    strong = choose(cohorts["validation"], budget=0, floor=weak)
    return {
        "calibration": dict(version=version, supporting_threshold=weak, strong_threshold=strong,
                            supporting_score=10, strong_score=18),
        "selection": "validation only; <=2 FP supporting with floor 0.9; zero FP strong",
        "metrics": {s: {"supporting": metrics(rs, weak), "strong": metrics(rs, strong)} for s, rs in cohorts.items()},
        "limitations": "Text-only operating points, not calibrated posterior probabilities or full-system guarantees. Correlated samples and prior test inspection limit independence. No auto-ban authorization.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("predictions", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    rows = [json.loads(line) for line in args.predictions.read_text().splitlines() if line.strip()]
    args.output.write_text(json.dumps(calibrate(rows, args.version), indent=2) + "\n")


if __name__ == "__main__":
    main()
