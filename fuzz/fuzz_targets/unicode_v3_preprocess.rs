#![no_main]

use libfuzzer_sys::fuzz_target;
use teloxide_antispam::preprocess::prepare_text_v3;

const MAX_INPUT_BYTES: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let input = &data[..data.len().min(MAX_INPUT_BYTES)];
    let text = String::from_utf8_lossy(input);
    let prepared = prepare_text_v3(&text);
    let scalar_count = text.chars().count();
    assert!(prepared.flags.format_chars <= scalar_count);
    assert!(prepared.flags.bidi_controls <= scalar_count);
    assert!(prepared.flags.non_ascii_whitespace <= scalar_count);
    assert!(prepared.flags.removed_invisible_chars <= scalar_count);
    assert!(prepared.flags.removed_word_variation_selectors <= scalar_count);
    assert!(prepared.flags.removed_stacked_marks <= scalar_count);
});
