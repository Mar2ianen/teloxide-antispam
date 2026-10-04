//! Versioned model view. This module does not replace original text, URLs,
//! or HTML. Unicode observations are not evidence of spam on their own.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};
use unicode_script::{Script, UnicodeScript};

pub const PREPROCESSING_VERSION: &str = "unicode-spam-v2";
const MIN_CHAR_NGRAM: usize = 3;
const MAX_CHAR_NGRAM: usize = 5;
const CONFUSABLES: &str = "ABCEHKMOPTXYaceopxyΑΒΕΗΚΜΟΡΤΥΧορχ";
const CYRILLIC_SKELETON: &str = "АВСЕНКМОРТХУасеорхуАВЕНКМОРТУХорх";

static WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{L}\p{N}_]+").expect("valid static token regex"));
static SPREAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|[^\p{L}\p{N}_])(?P<word>[А-Яа-яЁё](?:[ \t]+[А-Яа-яЁё]){4,}|[А-Яа-яЁё](?:[._·•-][А-Яа-яЁё]){4,})($|[^\p{L}\p{N}_])")
        .expect("valid static spread-word regex")
});

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextObservations {
    pub nfkc_changed: bool,
    pub format_chars: usize,
    pub bidi_controls: usize,
    pub non_ascii_whitespace: usize,
    pub removed_invisible_chars: usize,
    pub removed_word_variation_selectors: usize,
    pub removed_stacked_marks: usize,
    pub mixed_script_words: usize,
    pub homoglyph_chars: usize,
    pub homoglyph_to_latin_chars: usize,
    pub homoglyph_to_cyrillic_chars: usize,
    pub spaced_letter_sequences: usize,
    pub long_repeated_letter_runs: usize,
    pub changed: bool,
}

impl TextObservations {
    /// Emoji ZWJ alone does not count as an observation requiring review.
    /// Even true represents an observation, not a positive spam label.
    pub fn has_observation(&self) -> bool {
        self.nfkc_changed
            || self.mixed_script_words > 0
            || self.bidi_controls > 0
            || self.removed_invisible_chars > 0
            || self.removed_word_variation_selectors > 0
            || self.removed_stacked_marks > 0
            || self.spaced_letter_sequences > 0
            || self.long_repeated_letter_runs > 0
            || self.non_ascii_whitespace > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreparedText {
    pub canonical: String,
    pub compact_variant: Option<String>,
    pub flags: TextObservations,
}

impl PreparedText {
    /// Compaction is an auxiliary view; canonical and original stay separate.
    pub fn model_text(&self) -> String {
        match &self.compact_variant {
            Some(compact) => format!("{}\n{compact}", self.canonical),
            None => self.canonical.clone(),
        }
    }
}

pub fn prepare_text(text: &str) -> PreparedText {
    let mut flags = TextObservations::default();
    for ch in text.chars() {
        flags.format_chars += usize::from(ch.general_category() == GeneralCategory::Format);
        flags.bidi_controls += usize::from(is_bidi_control(ch));
        // Unusual whitespace: anything beyond plain space/tab/newline/CR,
        // including ASCII separators (U+001C-U+001F) and non-ASCII spaces.
        flags.non_ascii_whitespace +=
            usize::from(is_whitespace(ch) && !matches!(ch, ' ' | '\t' | '\n' | '\r'));
    }
    let compatibility: String = text.nfkc().collect();
    flags.nfkc_changed = compatibility != text;
    let cleaned = remove_stacked_marks(&remove_invisibles(&compatibility, &mut flags), &mut flags);
    let mapped = WORDS.replace_all(&cleaned, |captures: &regex::Captures<'_>| {
        normalize_word(&captures[0], &mut flags)
    });
    let canonical = collapse_whitespace(&mapped.to_lowercase());
    let compact_variant = compact_spread(&canonical, &mut flags);
    flags.long_repeated_letter_runs = repeated_letter_runs(&canonical);
    flags.changed = canonical != collapse_whitespace(&text.to_lowercase());
    PreparedText {
        canonical,
        compact_variant,
        flags,
    }
}

fn remove_invisibles(text: &str, flags: &mut TextObservations) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    for (index, &ch) in chars.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        // v2 scope: lone ZWJ handling stays as-is for model-view parity.
        // Tightening this changes canonical text and needs a v3 retrain.
        if ch == '\u{200d}'
            && !previous.is_some_and(is_alphanumeric)
            && !next.is_some_and(is_alphanumeric)
        {
            output.push(ch);
        } else if ch.general_category() == GeneralCategory::Format || ch == '\u{034f}' {
            flags.removed_invisible_chars += 1;
        } else if is_variation_selector(ch) && previous.is_some_and(is_alphanumeric) {
            flags.removed_word_variation_selectors += 1;
        } else {
            output.push(if is_whitespace(ch) { ' ' } else { ch });
        }
    }
    output
}

fn remove_stacked_marks(text: &str, flags: &mut TextObservations) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index].general_category_group() != GeneralCategoryGroup::Mark {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len()
            && chars[index].general_category_group() == GeneralCategoryGroup::Mark
        {
            index += 1;
        }
        let marks = &chars[start..index];
        let ordinary = marks
            .iter()
            .filter(|&&ch| !is_variation_selector(ch))
            .count();
        // v2 scope: single-mark runs stay attached for parity (NFKC already
        // composes и+breve into й). Dangling-mark stripping needs v3.
        if ordinary >= 2 {
            flags.removed_stacked_marks += ordinary;
            output.extend(
                marks
                    .iter()
                    .copied()
                    .filter(|&ch| is_variation_selector(ch)),
            );
        } else {
            output.extend(marks.iter());
        }
    }
    output
}

fn normalize_word(word: &str, flags: &mut TextObservations) -> String {
    let scripts: Vec<(char, Script)> = word
        .chars()
        .filter(|ch| ch.general_category_group() == GeneralCategoryGroup::Letter)
        .map(|ch| (ch, ch.script()))
        .collect();
    let cyrillic: Vec<char> = scripts
        .iter()
        .filter_map(|&(ch, script)| (script == Script::Cyrillic).then_some(ch))
        .collect();
    let foreign: Vec<char> = scripts
        .iter()
        .filter_map(|&(ch, script)| matches!(script, Script::Latin | Script::Greek).then_some(ch))
        .collect();
    // Any second script counts as mixed (Han, Armenian, ...), but homoglyph
    // repair stays v2-conservative: Latin/Greek <-> Cyrillic table only.
    // A Latin-majority token with a Greek letter keeps the v2 mapping
    // direction; changing it needs a v3 preprocessing version + retrain.
    let mut distinct: Vec<&'static str> = scripts
        .iter()
        .map(|&(_, script)| script.short_name())
        .collect();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() >= 2 {
        flags.mixed_script_words += 1;
    }
    if cyrillic.is_empty() || foreign.is_empty() {
        return word.to_owned();
    }
    let all_latin_or_cyrillic = scripts
        .iter()
        .all(|&(_, s)| matches!(s, Script::Latin | Script::Cyrillic));
    if all_latin_or_cyrillic
        && foreign.len() > cyrillic.len()
        && cyrillic.iter().all(|&ch| latin_skeleton(ch).is_some())
    {
        flags.homoglyph_chars += cyrillic.len();
        flags.homoglyph_to_latin_chars += cyrillic.len();
        return word
            .chars()
            .map(|ch| latin_skeleton(ch).unwrap_or(ch))
            .collect();
    }
    if foreign.iter().all(|&ch| cyrillic_skeleton(ch).is_some()) {
        flags.homoglyph_chars += foreign.len();
        flags.homoglyph_to_cyrillic_chars += foreign.len();
        return word
            .chars()
            .map(|ch| cyrillic_skeleton(ch).unwrap_or(ch))
            .collect();
    }
    word.to_owned()
}

fn cyrillic_skeleton(ch: char) -> Option<char> {
    CONFUSABLES
        .chars()
        .zip(CYRILLIC_SKELETON.chars())
        .find_map(|(from, to)| (ch == from).then_some(to))
}

fn latin_skeleton(ch: char) -> Option<char> {
    CONFUSABLES
        .chars()
        .zip(CYRILLIC_SKELETON.chars())
        .find_map(|(from, to)| (from.is_ascii() && ch == to).then_some(from))
}

fn compact_spread(text: &str, flags: &mut TextObservations) -> Option<String> {
    let mut output = String::with_capacity(text.len());
    let mut copied_until = 0;
    let mut search_from = 0;
    while let Some(captures) = SPREAD.captures_at(text, search_from) {
        let word = captures.name("word").expect("regex contains a word group");
        output.push_str(&text[copied_until..word.start()]);
        output.extend(word.as_str().chars().filter(|&ch| is_alphanumeric(ch)));
        flags.spaced_letter_sequences += 1;
        copied_until = word.end();
        // Preserve the right delimiter so adjacent patterns are not skipped.
        search_from = word.end();
    }
    if flags.spaced_letter_sequences == 0 {
        return None;
    }
    output.push_str(&text[copied_until..]);
    Some(output)
}

fn repeated_letter_runs(text: &str) -> usize {
    let mut previous = None;
    let mut run = 0;
    let mut count = 0;
    for ch in text.chars() {
        run = if is_word_char(ch) && !ch.is_numeric() && ch != '_' {
            if previous == Some(ch) { run + 1 } else { 1 }
        } else {
            0
        };
        count += usize::from(run == 4);
        previous = Some(ch);
    }
    count
}

pub(crate) fn is_alphanumeric(ch: char) -> bool {
    matches!(
        ch.general_category_group(),
        GeneralCategoryGroup::Letter | GeneralCategoryGroup::Number
    )
}

pub(crate) fn is_word_char(ch: char) -> bool {
    is_alphanumeric(ch) || ch == '_'
}

fn is_whitespace(ch: char) -> bool {
    // Python str.isspace also includes ASCII information separators.
    ch.is_whitespace() || matches!(ch, '\u{001c}'..='\u{001f}')
}

fn collapse_whitespace(text: &str) -> String {
    text.split(is_whitespace)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_bidi_control(ch: char) -> bool {
    matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn is_variation_selector(ch: char) -> bool {
    // v2 scope: only FE0E/FE0F word selectors. FE00-FE0D and E0100-E01EF are
    // known gaps, deferred to a v3 preprocessing version with retraining.
    matches!(ch, '\u{fe0e}' | '\u{fe0f}')
}

/// Word 1–2 without lemmatization. Single-letter words and underscores remain.
pub fn word_tokens(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|ch| !is_word_char(ch))
        .filter(|word| !word.is_empty())
        .collect();
    let mut tokens: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    tokens.extend(
        words
            .windows(2)
            .map(|pair| format!("{} {}", pair[0], pair[1])),
    );
    tokens
}

/// sklearn char_wb 3–5 using Unicode scalar boundaries and padded words.
pub fn char_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for word in text
        .to_lowercase()
        .split(is_whitespace)
        .filter(|word| !word.is_empty())
    {
        let chars: Vec<char> = std::iter::once(' ')
            .chain(word.chars())
            .chain(std::iter::once(' '))
            .collect();
        for n in MIN_CHAR_NGRAM..=MAX_CHAR_NGRAM {
            if n >= chars.len() {
                tokens.push(chars.iter().collect());
                break;
            }
            tokens.extend(chars.windows(n).map(|window| window.iter().collect()));
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_unicode_and_mixed_tokens_without_corrupting_tech_names() {
        let prepared =
            prepare_text("Ищeшь дoпoлнитeльный дoxoд? OpenAI Rust Bybit Nvidiа Intеl ＤＤＲ５");
        assert_eq!(
            prepared.canonical,
            "ищешь дополнительный доход? openai rust bybit nvidia intel ddr5"
        );
        assert!(prepared.flags.nfkc_changed);
        assert!(prepared.flags.homoglyph_to_latin_chars > 0);
        assert!(prepared.flags.homoglyph_to_cyrillic_chars > 0);
    }

    #[test]
    fn strips_word_invisibles_but_preserves_emoji_and_composed_letters() {
        let prepared =
            prepare_text("до\u{200b}хо\u{202e}д д\u{fe0f}а ❤️ 👩‍👩‍👧‍👦 и\u{0306} е\u{0308} café");
        assert_eq!(prepared.canonical, "доход да ❤️ 👩‍👩‍👧‍👦 й ё café");
        assert_eq!(prepared.flags.removed_invisible_chars, 2);
        assert_eq!(prepared.flags.bidi_controls, 1);
        assert_eq!(prepared.flags.removed_word_variation_selectors, 1);
        assert!(!prepare_text("👩‍👩‍👧‍👦").flags.has_observation());
    }

    #[test]
    fn stacked_marks_are_removed_as_a_whole() {
        let prepared = prepare_text("д\u{0334}\u{0335}\u{0336}оход");
        assert_eq!(prepared.canonical, "доход");
        assert_eq!(prepared.flags.removed_stacked_marks, 3);
    }

    #[test]
    fn stretched_words_are_auxiliary_and_short_acronyms_are_not_glued() {
        let prepared = prepare_text("Нужна п о д р а б о т к а; п.о.д.р.а.б.о.т.к.а");
        assert_eq!(
            prepared.canonical,
            "нужна п о д р а б о т к а; п.о.д.р.а.б.о.т.к.а"
        );
        assert_eq!(
            prepared.compact_variant.as_deref(),
            Some("нужна подработка; подработка")
        );
        assert_eq!(prepared.flags.spaced_letter_sequences, 2);
        assert!(
            prepare_text("Я и ты. А Б В. Р Г Б. GPU RTX 5090")
                .compact_variant
                .is_none()
        );
    }

    #[test]
    fn tokenizers_include_short_words_and_unicode_char_boundaries() {
        assert_eq!(
            word_tokens("доход в ЛС"),
            ["доход", "в", "лс", "доход в", "в лс"]
        );
        assert_eq!(char_tokens("я"), [" я "]);
        assert_eq!(char_tokens("лс"), [" лс", "лс ", " лс "]);
        assert!(char_tokens("доход").contains(&"оход".to_owned()));
    }

    #[test]
    fn canonical_is_idempotent_for_synthetic_unicode_cases() {
        for text in [
            "дοхοд",
            "до\u{200b}ход",
            "Ｂｙｂｉｔ",
            "Семья 👩‍👩‍👧‍👦",
            "п о д р а б о т к а",
        ] {
            let once = prepare_text(text).canonical;
            assert_eq!(prepare_text(&once).canonical, once);
        }
    }
}
