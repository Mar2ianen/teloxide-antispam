#![no_main]

use libfuzzer_sys::fuzz_target;
use teloxide_antispam::preprocess::prepare_text_v3;

const MAX_INPUT_BYTES: usize = 1 << 20;
// The maximum recursive canonical/compatibility decomposition length in the
// pinned Unicode 18 UnicodeData.txt is 18 (U+FDFA). NFKC cannot produce a
// longer string than full decomposition, so post-NFKC counters use this bound.
const MAX_NFKC_SCALARS_PER_SOURCE_SCALAR: usize = 18;

fuzz_target!(|data: &[u8]| {
    let input = &data[..data.len().min(MAX_INPUT_BYTES)];
    let text = String::from_utf8_lossy(input);
    let prepared = prepare_text_v3(&text);
    let scalar_count = text.chars().count();
    assert!(prepared.flags.format_chars <= scalar_count);
    assert!(prepared.flags.bidi_controls <= scalar_count);
    assert!(prepared.flags.non_ascii_whitespace <= scalar_count);
    let normalized_scalar_budget = scalar_count.saturating_mul(MAX_NFKC_SCALARS_PER_SOURCE_SCALAR);
    assert!(prepared.flags.removed_invisible_chars <= normalized_scalar_budget);
    assert!(prepared.flags.removed_word_variation_selectors <= normalized_scalar_budget);
    assert!(prepared.flags.removed_stacked_marks <= normalized_scalar_budget);
});
