//! Versioned model view. This module does not replace original text, URLs,
//! or HTML. Unicode observations are not evidence of spam on their own.

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, GeneralCategoryGroup, UnicodeGeneralCategory};
use unicode_script::{Script, UnicodeScript};

#[cfg(feature = "unicode-v3")]
use icu_casemap::CaseMapper;
#[cfg(feature = "unicode-v3")]
use icu_normalizer::ComposingNormalizer;
#[cfg(feature = "unicode-v3")]
use icu_properties::{
    CodePointMapData, CodePointSetData,
    props::{
        BidiControl, DefaultIgnorableCodePoint, GeneralCategory as IcuGeneralCategory,
        GeneralCategoryGroup as IcuGeneralCategoryGroup, Script as IcuScript, VariationSelector,
        WhiteSpace,
    },
};

pub const PREPROCESSING_VERSION: &str = "unicode-spam-v2";
#[cfg(feature = "unicode-v3")]
pub const PREPROCESSING_VERSION_V3: &str = "unicode-spam-v3-icu4x-2.4-uts39-18";
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

/// New Unicode 18 model view. Kept separate from V2 because the property data,
/// case folding, and confusable skeleton intentionally change canonical text.
#[cfg(feature = "unicode-v3")]
pub fn prepare_text_v3(text: &str) -> PreparedText {
    let mut flags = TextObservations::default();
    let nfkc = ComposingNormalizer::new_nfkc();
    let case_mapper = CaseMapper::new();
    let categories = CodePointMapData::<IcuGeneralCategory>::new();
    let whitespace = CodePointSetData::new::<WhiteSpace>();
    let ignorable = CodePointSetData::new::<DefaultIgnorableCodePoint>();
    let variation_selector = CodePointSetData::new::<VariationSelector>();
    let bidi_control = CodePointSetData::new::<BidiControl>();

    for ch in text.chars() {
        flags.format_chars += usize::from(categories.get(ch) == IcuGeneralCategory::Format);
        flags.bidi_controls += usize::from(bidi_control.contains(ch));
        flags.non_ascii_whitespace += usize::from(
            (whitespace.contains(ch) || matches!(ch, '\u{001c}'..='\u{001f}'))
                && !matches!(ch, ' ' | '\t' | '\n' | '\r'),
        );
    }

    let compatibility = nfkc.normalize(text).into_owned();
    flags.nfkc_changed = compatibility != text;
    let cleaned = remove_stacked_marks_v3(
        &remove_invisibles_v3(
            &compatibility,
            &mut flags,
            whitespace,
            ignorable,
            variation_selector,
        ),
        &mut flags,
        &categories,
    );
    let mapped = normalize_word_runs_v3(&cleaned, &mut flags, &categories);
    let folded = case_mapper.fold_string(&mapped);
    let canonical = collapse_whitespace_v3(&folded, whitespace);
    let compact_variant = compact_spread(&canonical, &mut flags);
    flags.long_repeated_letter_runs = repeated_letter_runs_v3(&canonical, &categories);
    let original_folded = case_mapper.fold_string(text);
    flags.changed = canonical != collapse_whitespace_v3(&original_folded, whitespace);
    PreparedText {
        canonical,
        compact_variant,
        flags,
    }
}

#[cfg(feature = "unicode-v3")]
fn remove_invisibles_v3(
    text: &str,
    flags: &mut TextObservations,
    whitespace: icu_properties::CodePointSetDataBorrowed<'static>,
    ignorable: icu_properties::CodePointSetDataBorrowed<'static>,
    variation_selector: icu_properties::CodePointSetDataBorrowed<'static>,
) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    for (index, &ch) in chars.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        if ch == '\u{200d}'
            && !previous.is_some_and(is_alphanumeric_v3)
            && !next.is_some_and(is_alphanumeric_v3)
        {
            output.push(ch);
        } else if ignorable.contains(ch) && !variation_selector.contains(ch) {
            flags.removed_invisible_chars += 1;
        } else if variation_selector.contains(ch) && previous.is_some_and(is_alphanumeric_v3) {
            flags.removed_word_variation_selectors += 1;
        } else {
            output.push(if is_whitespace_v3(ch, whitespace) {
                ' '
            } else {
                ch
            });
        }
    }
    output
}

#[cfg(feature = "unicode-v3")]
fn remove_stacked_marks_v3(
    text: &str,
    flags: &mut TextObservations,
    categories: &icu_properties::CodePointMapDataBorrowed<'static, IcuGeneralCategory>,
) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if !IcuGeneralCategoryGroup::Mark.contains(categories.get(chars[index])) {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len()
            && IcuGeneralCategoryGroup::Mark.contains(categories.get(chars[index]))
        {
            index += 1;
        }
        let marks = &chars[start..index];
        let ordinary = marks
            .iter()
            .filter(|&&ch| !matches!(ch as u32, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef))
            .count();
        if ordinary >= 2 {
            flags.removed_stacked_marks += ordinary;
            output.extend(
                marks
                    .iter()
                    .copied()
                    .filter(|&ch| matches!(ch as u32, 0xfe00..=0xfe0f | 0xe0100..=0xe01ef)),
            );
        } else {
            output.extend(marks.iter());
        }
    }
    output
}

#[cfg(feature = "unicode-v3")]
fn normalize_word_runs_v3(
    text: &str,
    flags: &mut TextObservations,
    categories: &icu_properties::CodePointMapDataBorrowed<'static, IcuGeneralCategory>,
) -> String {
    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    while cursor < text.len() {
        let ch = text[cursor..]
            .chars()
            .next()
            .expect("cursor is a char boundary");
        if !is_word_core_v3(ch, categories) {
            output.push(ch);
            cursor += ch.len_utf8();
            continue;
        }
        let start = cursor;
        cursor += ch.len_utf8();
        while cursor < text.len() {
            let next = text[cursor..]
                .chars()
                .next()
                .expect("cursor is a char boundary");
            if !is_word_core_v3(next, categories) {
                break;
            }
            cursor += next.len_utf8();
        }
        output.push_str(&normalize_word_v3(&text[start..cursor], flags, categories));
    }
    output
}

#[cfg(feature = "unicode-v3")]
fn normalize_word_v3(
    word: &str,
    flags: &mut TextObservations,
    categories: &icu_properties::CodePointMapDataBorrowed<'static, IcuGeneralCategory>,
) -> String {
    let scripts: Vec<(char, IcuScript)> = word
        .chars()
        .filter(|&ch| IcuGeneralCategoryGroup::Letter.contains(categories.get(ch)))
        .map(|ch| (ch, CodePointMapData::<IcuScript>::new().get(ch)))
        .collect();
    let mut distinct: Vec<IcuScript> = scripts.iter().map(|&(_, script)| script).collect();
    distinct.sort_unstable();
    distinct.dedup();
    let has_cyrillic = scripts
        .iter()
        .any(|&(_, script)| script == IcuScript::Cyrillic);
    let has_latin_or_greek = scripts
        .iter()
        .any(|&(_, script)| matches!(script, IcuScript::Latin | IcuScript::Greek));
    if distinct.len() >= 2 {
        flags.mixed_script_words += 1;
    }

    let skeleton: String = unicode_security::skeleton(word).collect();
    if skeleton == word {
        return word.to_owned();
    }
    let changed = word
        .chars()
        .zip(skeleton.chars())
        .filter(|(before, after)| before != after)
        .count()
        + word.chars().count().abs_diff(skeleton.chars().count());
    flags.homoglyph_chars += changed;
    if has_cyrillic && skeleton.is_ascii() {
        flags.homoglyph_to_latin_chars += changed;
    } else if has_latin_or_greek
        && skeleton
            .chars()
            .any(|ch| CodePointMapData::<IcuScript>::new().get(ch) == IcuScript::Cyrillic)
    {
        flags.homoglyph_to_cyrillic_chars += changed;
    }
    skeleton
}

#[cfg(feature = "unicode-v3")]
fn is_word_core_v3(
    ch: char,
    categories: &icu_properties::CodePointMapDataBorrowed<'static, IcuGeneralCategory>,
) -> bool {
    is_alphanumeric_v3(ch)
        || ch == '_'
        || IcuGeneralCategoryGroup::Letter.contains(categories.get(ch))
}

#[cfg(feature = "unicode-v3")]
fn is_alphanumeric_v3(ch: char) -> bool {
    let category = CodePointMapData::<IcuGeneralCategory>::new().get(ch);
    IcuGeneralCategoryGroup::Letter.contains(category)
        || IcuGeneralCategoryGroup::Number.contains(category)
}

#[cfg(feature = "unicode-v3")]
fn is_whitespace_v3(
    ch: char,
    whitespace: icu_properties::CodePointSetDataBorrowed<'static>,
) -> bool {
    whitespace.contains(ch) || matches!(ch, '\u{001c}'..='\u{001f}')
}

#[cfg(feature = "unicode-v3")]
fn collapse_whitespace_v3(
    text: &str,
    whitespace: icu_properties::CodePointSetDataBorrowed<'static>,
) -> String {
    text.split(|ch| is_whitespace_v3(ch, whitespace))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(feature = "unicode-v3")]
fn repeated_letter_runs_v3(
    text: &str,
    categories: &icu_properties::CodePointMapDataBorrowed<'static, IcuGeneralCategory>,
) -> usize {
    let mut previous = None;
    let mut run = 0;
    let mut count = 0;
    for ch in text.chars() {
        run = if is_word_core_v3(ch, categories)
            && !IcuGeneralCategoryGroup::Number.contains(categories.get(ch))
            && ch != '_'
        {
            if previous == Some(ch) { run + 1 } else { 1 }
        } else {
            0
        };
        count += usize::from(run == 4);
        previous = Some(ch);
    }
    count
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

#[cfg(all(test, feature = "unicode-v3"))]
mod unicode_v3_conformance_tests {
    use std::collections::HashMap;

    use icu_casemap::CaseMapper;
    use icu_normalizer::ComposingNormalizer;
    use icu_properties::{
        CodePointMapData, CodePointSetData,
        props::{
            DefaultIgnorableCodePoint, GeneralCategory as IcuGeneralCategory, Script as IcuScript,
        },
    };
    use proptest::prelude::{any, prop_assert_eq};
    use unicode_normalization::UnicodeNormalization;

    use super::*;

    fn decode_codepoints(input: &str) -> String {
        input
            .split_whitespace()
            .map(|point| char::from_u32(u32::from_str_radix(point, 16).unwrap()).unwrap())
            .collect()
    }

    #[test]
    fn icu_nfkc_passes_unicode_18_normalization_test() {
        let nfkc = ComposingNormalizer::new_nfkc();
        let mut rows = 0;
        for line in include_str!("../test-data/unicode/18.0.0/NormalizationTest.txt").lines() {
            let data = line.split('#').next().unwrap_or_default().trim();
            if data.is_empty() || data.starts_with('@') {
                continue;
            }
            let columns: Vec<_> = data.split(';').take(5).map(decode_codepoints).collect();
            assert_eq!(columns.len(), 5, "malformed NormalizationTest row {data}");
            let expected = &columns[3];
            for input in &columns {
                assert_eq!(nfkc.normalize(input).as_ref(), expected, "NFKC of {data}");
            }
            rows += 1;
        }
        assert!(rows > 20_000, "official normalization corpus is truncated");
    }

    #[test]
    fn v2_v3_normalization_differential_is_explicit() {
        let nfkc = ComposingNormalizer::new_nfkc();
        let mut rows = 0;
        let mut changed = 0;
        let mut examples = Vec::new();
        for line in include_str!("../test-data/unicode/18.0.0/NormalizationTest.txt").lines() {
            let data = line.split('#').next().unwrap_or_default().trim();
            if data.is_empty() || data.starts_with('@') {
                continue;
            }
            let columns: Vec<_> = data.split(';').take(5).map(decode_codepoints).collect();
            let expected = &columns[3];
            for input in &columns {
                let old: String = input.nfkc().collect();
                let new = nfkc.normalize(input).into_owned();
                assert_eq!(new, *expected, "ICU4X Unicode 18 oracle for {data}");
                if old != new {
                    changed += 1;
                    if examples.len() < 5 {
                        examples.push(data.to_owned());
                    }
                }
                rows += 1;
            }
        }
        eprintln!(
            "V2 unicode-normalization vs ICU4X 2.4 NFKC: {changed}/{rows} corpus inputs differ; examples={examples:?}"
        );
        assert!(
            rows > 100_000,
            "normalization differential corpus truncated"
        );
    }

    #[test]
    fn icu_casefold_passes_unicode_18_default_case_folding() {
        let mut mappings = HashMap::<u32, (u8, String)>::new();
        for line in include_str!("../test-data/unicode/18.0.0/CaseFolding.txt").lines() {
            let data = line.split('#').next().unwrap_or_default().trim();
            if data.is_empty() {
                continue;
            }
            let fields: Vec<_> = data.split(';').map(str::trim).collect();
            let status = match fields[1] {
                "C" => 1,
                "F" => 2,
                _ => continue,
            };
            let codepoint = u32::from_str_radix(fields[0], 16).unwrap();
            let mapping = (status, decode_codepoints(fields[2]));
            if mappings
                .get(&codepoint)
                .is_none_or(|previous| previous.0 < status)
            {
                mappings.insert(codepoint, mapping);
            }
        }
        let mapper = CaseMapper::new();
        for (&codepoint, (_, expected)) in &mappings {
            let source = char::from_u32(codepoint).unwrap().to_string();
            assert_eq!(
                mapper.fold_string(&source).as_ref(),
                expected,
                "case fold U+{codepoint:04X}"
            );
        }
        assert!(
            mappings.len() > 1_500,
            "official case-folding corpus is truncated"
        );
    }

    #[test]
    fn v2_v3_casefold_differential_is_explicit() {
        let mut mappings = HashMap::<u32, (u8, String)>::new();
        for line in include_str!("../test-data/unicode/18.0.0/CaseFolding.txt").lines() {
            let data = line.split('#').next().unwrap_or_default().trim();
            if data.is_empty() {
                continue;
            }
            let fields: Vec<_> = data.split(';').map(str::trim).collect();
            let status = match fields[1] {
                "C" => 1,
                "F" => 2,
                _ => continue,
            };
            let codepoint = u32::from_str_radix(fields[0], 16).unwrap();
            let mapping = (status, decode_codepoints(fields[2]));
            if mappings
                .get(&codepoint)
                .is_none_or(|previous| previous.0 < status)
            {
                mappings.insert(codepoint, mapping);
            }
        }
        let mapper = CaseMapper::new();
        let mut changed = 0;
        let mut examples = Vec::new();
        for (&codepoint, (_, expected)) in &mappings {
            let source = char::from_u32(codepoint).unwrap().to_string();
            assert_eq!(mapper.fold_string(&source), *expected);
            let old = source.to_lowercase();
            if old != *expected {
                changed += 1;
                if examples.len() < 8 {
                    examples.push((format!("U+{codepoint:04X}"), old, expected.clone()));
                }
            }
        }
        eprintln!(
            "Rust lowercase vs ICU4X Unicode 18 default case fold: {changed}/{} mappings differ; examples={examples:?}",
            mappings.len()
        );
        assert!(changed > 0, "expected documented full-case-fold changes");
    }

    #[test]
    fn icu_default_ignorables_match_unicode_18_for_every_scalar_value() {
        let mut expected = vec![false; 0x11_0000];
        for line in include_str!("../test-data/unicode/18.0.0/DerivedCoreProperties.txt").lines() {
            let data = line.split('#').next().unwrap_or_default().trim();
            let Some((range, property)) = data.split_once(';') else {
                continue;
            };
            if property.trim() != "Default_Ignorable_Code_Point" {
                continue;
            }
            let (start, end) = match range.trim().split_once("..") {
                Some((start, end)) => (
                    u32::from_str_radix(start, 16).unwrap(),
                    u32::from_str_radix(end, 16).unwrap(),
                ),
                None => {
                    let point = u32::from_str_radix(range.trim(), 16).unwrap();
                    (point, point)
                }
            };
            expected[start as usize..=end as usize].fill(true);
        }
        let actual = CodePointSetData::new::<DefaultIgnorableCodePoint>();
        let mismatches: Vec<_> = expected
            .iter()
            .enumerate()
            .filter_map(|(point, expected)| {
                char::from_u32(point as u32)
                    .filter(|ch| actual.contains(*ch) != *expected)
                    .map(|_| point)
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "Default_Ignorable mismatches: {mismatches:?}"
        );
    }

    #[test]
    fn icu_prop_list_properties_match_unicode_18_for_every_scalar_value() {
        fn expected_property(name: &str) -> Vec<bool> {
            let mut expected = vec![false; 0x11_0000];
            for line in include_str!("../test-data/unicode/18.0.0/PropList.txt").lines() {
                let data = line.split('#').next().unwrap_or_default().trim();
                let Some((range, property)) = data.split_once(';') else {
                    continue;
                };
                if property.trim() != name {
                    continue;
                }
                let (start, end) = match range.trim().split_once("..") {
                    Some((start, end)) => (
                        u32::from_str_radix(start, 16).unwrap(),
                        u32::from_str_radix(end, 16).unwrap(),
                    ),
                    None => {
                        let point = u32::from_str_radix(range.trim(), 16).unwrap();
                        (point, point)
                    }
                };
                expected[start as usize..=end as usize].fill(true);
            }
            expected
        }

        let white_space = CodePointSetData::new::<WhiteSpace>();
        let bidi_control = CodePointSetData::new::<BidiControl>();
        let variation_selector = CodePointSetData::new::<VariationSelector>();
        let properties = [
            ("White_Space", expected_property("White_Space"), white_space),
            (
                "Bidi_Control",
                expected_property("Bidi_Control"),
                bidi_control,
            ),
            (
                "Variation_Selector",
                expected_property("Variation_Selector"),
                variation_selector,
            ),
        ];
        for (name, expected, actual) in properties {
            let mismatches: Vec<_> = expected
                .iter()
                .enumerate()
                .filter_map(|(point, expected)| {
                    char::from_u32(point as u32)
                        .filter(|ch| actual.contains(*ch) != *expected)
                        .map(|_| point)
                })
                .collect();
            assert!(mismatches.is_empty(), "{name} mismatches: {mismatches:?}");
        }
    }

    #[test]
    fn v2_v3_binary_property_deltas_are_explicit_for_every_scalar() {
        let white_space = CodePointSetData::new::<WhiteSpace>();
        let bidi_control = CodePointSetData::new::<BidiControl>();
        let variation_selector = CodePointSetData::new::<VariationSelector>();
        let deltas =
            (0..=0x10_ffff)
                .filter_map(char::from_u32)
                .fold([0usize; 3], |mut counts, ch| {
                    counts[0] += usize::from(is_whitespace(ch) != white_space.contains(ch));
                    counts[1] += usize::from(is_bidi_control(ch) != bidi_control.contains(ch));
                    counts[2] +=
                        usize::from(is_variation_selector(ch) != variation_selector.contains(ch));
                    counts
                });
        // V2 deliberately follows Python's four extra ASCII whitespace
        // separators and only treats FE0E/FE0F as word variation selectors.
        assert_eq!(deltas, [4, 0, 258]);
    }

    #[test]
    fn icu_general_categories_match_unicode_18_unicode_data() {
        let mut expected = vec![IcuGeneralCategory::Unassigned; 0x11_0000];
        let mut range_start = None;
        for line in include_str!("../test-data/unicode/18.0.0/UnicodeData.txt").lines() {
            let fields: Vec<_> = line.split(';').collect();
            let point = u32::from_str_radix(fields[0], 16).unwrap();
            let category = parse_general_category(fields[2]);
            if fields[1].ends_with(", First>") {
                range_start = Some((point, category));
            } else if fields[1].ends_with(", Last>") {
                let (start, range_category) = range_start.take().expect("First precedes Last");
                assert_eq!(category, range_category);
                expected[start as usize..=point as usize].fill(category);
            } else {
                expected[point as usize] = category;
            }
        }
        assert!(range_start.is_none(), "UnicodeData range is unterminated");

        let actual = CodePointMapData::<IcuGeneralCategory>::new();
        let mismatches: Vec<_> = expected
            .iter()
            .enumerate()
            .filter_map(|(point, expected)| {
                char::from_u32(point as u32)
                    .filter(|ch| actual.get(*ch) != *expected)
                    .map(|_| {
                        (
                            point,
                            *expected,
                            actual.get(char::from_u32(point as u32).unwrap()),
                        )
                    })
            })
            .collect();
        assert!(
            mismatches.is_empty(),
            "General_Category mismatches: {mismatches:?}"
        );
    }

    #[test]
    fn icu_script_property_matches_unicode_18_for_classifier_scripts() {
        let selected = [
            ("Latin", IcuScript::Latin),
            ("Cyrillic", IcuScript::Cyrillic),
            ("Greek", IcuScript::Greek),
            ("Han", IcuScript::Han),
            ("Armenian", IcuScript::Armenian),
        ];
        let mut expected = vec![None; 0x11_0000];
        for line in include_str!("../test-data/unicode/18.0.0/Scripts.txt").lines() {
            let data = line.split('#').next().unwrap_or_default().trim();
            let Some((range, script)) = data.split_once(';') else {
                continue;
            };
            let Some((_, value)) = selected.iter().find(|(name, _)| *name == script.trim()) else {
                continue;
            };
            let (start, end) = match range.trim().split_once("..") {
                Some((start, end)) => (
                    u32::from_str_radix(start, 16).unwrap(),
                    u32::from_str_radix(end, 16).unwrap(),
                ),
                None => {
                    let point = u32::from_str_radix(range.trim(), 16).unwrap();
                    (point, point)
                }
            };
            expected[start as usize..=end as usize].fill(Some(*value));
        }
        let actual = CodePointMapData::<IcuScript>::new();
        let mismatches: Vec<_> = expected
            .iter()
            .enumerate()
            .filter_map(|(point, expected)| {
                let ch = char::from_u32(point as u32)?;
                expected
                    .filter(|expected| actual.get(ch) != *expected)
                    .map(|expected| (point, expected, actual.get(ch)))
            })
            .collect();
        assert!(mismatches.is_empty(), "Script mismatches: {mismatches:?}");
    }

    #[test]
    fn uts39_skeleton_matches_unicode_18_confusables_data() {
        let mut mappings = 0;
        for line in include_str!("../test-data/unicode/18.0.0/confusables.txt").lines() {
            let data = line.split('#').next().unwrap_or_default().trim();
            if data.is_empty() {
                continue;
            }
            let fields: Vec<_> = data.split(';').map(str::trim).collect();
            let source = decode_codepoints(fields[0]);
            let expected: String = decode_codepoints(fields[1]).nfd().collect();
            let actual: String = unicode_security::skeleton(&source).collect();
            if actual.is_empty() && !expected.is_empty() {
                // UTS #39 removes Default_Ignorable_Code_Point characters
                // before applying prototypes; the source file also lists
                // deliberate mappings for some such code points.
                mappings += 1;
                continue;
            }
            assert_eq!(actual, expected, "UTS #39 confusable mapping {data}");
            mappings += 1;
        }
        assert_eq!(
            mappings, 6_712,
            "review official confusables corpus changes"
        );
    }

    #[test]
    fn v3_is_explicit_and_preserves_v2_model_view() {
        assert_eq!(PREPROCESSING_VERSION, "unicode-spam-v2");
        assert_eq!(
            PREPROCESSING_VERSION_V3,
            "unicode-spam-v3-icu4x-2.4-uts39-18"
        );
        let v2 = prepare_text("Straße");
        let v3 = prepare_text_v3("Straße");
        assert_eq!(v2.canonical, "straße");
        assert_eq!(v3.canonical, "strasse");

        let spoof = prepare_text_v3("раypal");
        assert_eq!(spoof.canonical, "paypal");
        assert!(spoof.flags.mixed_script_words > 0);
        assert!(spoof.flags.homoglyph_chars > 0);
    }

    #[test]
    fn uts39_skeleton_is_applied_once_not_iterated_to_a_fixed_point() {
        // UTS #39 maps the V3-folded thorn to itself in this pass, while a
        // second skeleton pass maps that output to ASCII "p". The model view
        // deliberately applies the specified skeleton operation once.
        let once = prepare_text_v3("Þ").canonical;
        assert_eq!(once, "þ");
        assert_eq!(prepare_text_v3(&once).canonical, "p");
    }

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config {
            cases: 512,
            rng_seed: proptest::test_runner::RngSeed::Fixed(0x0055_4344_3138),
            ..proptest::test_runner::Config::default()
        })]
        #[test]
        fn v3_unicode_projection_is_total_and_deterministic(chars in proptest::collection::vec(any::<char>(), 0..96)) {
            let text: String = chars.into_iter().collect();
            let prepared = prepare_text_v3(&text);
            prop_assert_eq!(prepare_text_v3(&text), prepared);
        }
    }

    fn parse_general_category(value: &str) -> IcuGeneralCategory {
        use IcuGeneralCategory as G;
        match value {
            "Lu" => G::UppercaseLetter,
            "Ll" => G::LowercaseLetter,
            "Lt" => G::TitlecaseLetter,
            "Lm" => G::ModifierLetter,
            "Lo" => G::OtherLetter,
            "Mn" => G::NonspacingMark,
            "Mc" => G::SpacingMark,
            "Me" => G::EnclosingMark,
            "Nd" => G::DecimalNumber,
            "Nl" => G::LetterNumber,
            "No" => G::OtherNumber,
            "Pc" => G::ConnectorPunctuation,
            "Pd" => G::DashPunctuation,
            "Ps" => G::OpenPunctuation,
            "Pe" => G::ClosePunctuation,
            "Pi" => G::InitialPunctuation,
            "Pf" => G::FinalPunctuation,
            "Po" => G::OtherPunctuation,
            "Sm" => G::MathSymbol,
            "Sc" => G::CurrencySymbol,
            "Sk" => G::ModifierSymbol,
            "So" => G::OtherSymbol,
            "Zs" => G::SpaceSeparator,
            "Zl" => G::LineSeparator,
            "Zp" => G::ParagraphSeparator,
            "Cc" => G::Control,
            "Cf" => G::Format,
            "Cs" => G::Surrogate,
            "Co" => G::PrivateUse,
            "Cn" => G::Unassigned,
            _ => panic!("unknown General_Category {value}"),
        }
    }
}
