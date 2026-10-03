# ALT word/character v2 — evaluation and operating points

Model: `alt-word-char-v2-2026-10-03`.
Analyzer: `unicode_word_char_v2`; preprocessing: `unicode-spam-v2`.
Calibration: `alt-word-char-validation-2026-10-03-v1`.
Release asset: `alt_word_char_v2_2026-10-03.json`.
SHA256: `b90d9ebae22abde31238269e595164e41025ddf4e0d1c40d5ba1810f387b2d63`.

## Data and selection

Training uses only published labels from
[`alt-gnome/telegram-spam-20251030`](https://huggingface.co/datasets/alt-gnome/telegram-spam-20251030),
declared CC0. No private chat exports, moderator logs, or account-ban-derived
labels were used. Source records: 32,763; source row digest:
`1aff8d6dd8e74c3adc3517f6879c5f8e41d78c7ebacae6970cda454bb8f26a00`.

Grouped 60/20/20 split, contacts/numbers masked for template grouping;
canonical duplicates deduplicated within split; one conflicting group excluded.
Train: 19,471 (9,556 spam / 9,915 ham); validation: 6,469 (3,164 / 3,305);
test: 6,485 (3,180 / 3,305). Semantically related templates can still cross
groups. Author/time isolation cannot be established from these text-only data.

The selected candidate has 85,927 word/character features. Unicode flags are
recorded for audit, not supplied as weighted model features. Candidate choice
was informed by earlier test/robustness inspection: this is not a pristine
independent final holdout. Thresholds below were selected on validation only.

## Supporting-score thresholds

| Band | Threshold | Points | Validation TP / FP / FN / TN | Test TP / FP / FN / TN |
|---|---:|---:|---|---|
| Supporting | 0.9 | 10 | 2958 / 2 / 206 / 3303 | 2985 / 2 / 195 / 3303 |
| Strong supporting | 0.9747144300743944 | 18 | 2694 / 0 / 470 / 3305 | 2687 / 0 / 493 / 3305 |

Supporting test recall: **93.87%**, FPR **0.0605%**.
Strong supporting test recall: **84.50%**, zero observed FP among 3,305 ham.
The one-sided 95% zero-FP bound is approximately **0.0906%**, assuming
independent representative examples; correlation weakens that assumption.
At threshold 0.5, test recall is 98.65% with 7 FP. None of these numbers
establishes 99.9% recall or FPR ≤0.0001%, nor full moderation performance.

`tools/calibrate.py` reproduces operating-point selection from probability-only
validation/test rows: supporting permits ≤2 validation FP with floor 0.9;
strong permits zero. A 1e-9 threshold margin absorbs inference rounding.
These are score-band operating points, not posterior-probability calibration.
The 10/18-point magnitudes retain the existing supporting-score policy; they
are not learned or claimed to be calibrated for the full profile-aware system.

## Intended use and limitations

Dry-run scoring, audit, and manually reviewed evidence. Not standalone bans.
Consumers need independently adjudicated, profile-aware, temporal/author/
campaign-separated local evaluation before permitting automatic enforcement.
Unreviewed exports are challenge inputs, not presumed ham. Account bans do
not make every account message spam; external detector verdicts are not gold.

Rust/Python parity: 32,253 texts; zero canonical/compact/flag discrepancies;
maximum probability error 3.11e-15 before adding calibration metadata (weights
unchanged). The release asset is separately checked on validation and test.
No private texts or prototype experiment scripts are included in the release.
