# Conversation flood primitives

The opt-in `flood` feature provides pure event-time math and bounded snapshot
metrics. It does not call Telegram, load data, persist state, choose an action,
or classify etiquette/flood/scam. The consuming application owns scope,
authorization, persistence, thresholds, audit, and rollout.

## Time contracts

- Timestamps are unsigned microseconds on one caller-defined event-time
  timeline. Use Telegram message time for replayable snapshots; do not mix wall
  time and monotonic process time in one statistic.
- `EwmaRate` and `DecayedMean` accept nondecreasing event times. Invalid,
  nonfinite, overflowing, or out-of-order samples are rejected without
  updating their stored statistic. A consumer that accepts late updates must
  reorder them or mark the resulting evidence incomplete.
- For time constant `tau`, a weighted event contributes `weight / tau_seconds`
  to the rate and prior rate decays by `exp(-delta / tau)`. EWMA is a
  descriptive signal. Use `ExactWindow` or a token bucket for a hard limit.
- `ExactWindow` counts events in `(now - window, now]`. Its memory is bounded.
  Capacity overflow drops the new event and makes the answer `None` until that
  omitted event must have expired. Never treat overflow as a zero count.

## Conversation snapshots

`analyze_window` accepts a bounded snapshot of the latest message projection
for exactly one instance, chat, and topic. The caller must deduplicate edits,
preserve only the current projection, and choose the appropriate `complete`
value. If history is truncated, a scope was evicted, or input could not be
loaded, pass `complete = false`; every metric then carries that state.

Bounds are 2,048 messages, 512 tokens per message, 16,384 tokens total, and
256 UTF-8 bytes per token. The analyzer rejects oversize or duplicate-ID input
instead of silently truncating it. Timestamps are sorted before calculating
turns. Messages in one album count as one logical send, while every album item
still contributes media/emoji/mention counts. Exact duplicate text is grouped
with its reply target, so identical short replies to different messages are
not treated as the same conversational act.

Metrics include message/media/emoji/mention rates and counts, largest actor
share, consecutive actor runs, conversation turns, token entropy, exact
duplicate pairs, maximum token-trigram Jaccard, novelty against earlier
messages in the window, reply-target diversity, messages per token, and
cross-actor token overlap. Empty or insufficient samples are `None`, not
fabricated zeros. All values are evidence, not a verdict; in particular, a
high actor share or repeated response is not a moderation action.

The application should run this analysis in shadow mode first, compare it with
chronological replay and false-positive fixtures, and choose per-chat policy
only after review. The crate intentionally has no universal production
threshold.

## Verification

`tests/flood_properties.rs` compares EWMA, decayed means, and exact windows to
independent full-history formulas. The pull-request feature matrix runs 2,048
property cases with a fixed seed. `quality-nightly.yml` runs 20,000 cases.
