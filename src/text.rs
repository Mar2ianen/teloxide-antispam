use std::collections::BTreeSet;

pub fn first_text_chars(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }

    if limit == 0 {
        return "…".to_string();
    }

    format!("{}…", trimmed.chars().take(limit).collect::<String>())
}

pub fn normalize_ai_markers(text: &str) -> String {
    text.replace(['—', '–'], "-")
        .replace(['«', '»'], "\"")
        .replace("Вот вариант:", "")
        .replace("Вариант:", "")
        .trim()
        .to_string()
}

/// Map Latin confusables to Cyrillic only inside tokens that already
/// contain Cyrillic. Pure Latin tokens (brands, tickers, commands) stay
/// unchanged so `OpenAI` is not corrupted by a Cyrillic word elsewhere.
pub fn normalize_cyrillic_homoglyphs(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, output: &mut String| {
        if token
            .chars()
            .any(|ch| matches!(ch, '\u{0400}'..='\u{04ff}'))
        {
            for ch in std::mem::take(token).chars() {
                output.push(match ch {
                    'A' => 'А',
                    'B' => 'В',
                    'C' => 'С',
                    'E' => 'Е',
                    'H' => 'Н',
                    'K' => 'К',
                    'M' => 'М',
                    'O' => 'О',
                    'P' => 'Р',
                    'T' => 'Т',
                    'X' => 'Х',
                    'Y' => 'У',
                    'a' => 'а',
                    'c' => 'с',
                    'e' => 'е',
                    'o' => 'о',
                    'p' => 'р',
                    'x' => 'х',
                    'y' => 'у',
                    _ => ch,
                });
            }
        } else {
            output.push_str(&std::mem::take(token));
        }
    };
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            token.push(ch);
        } else {
            flush(&mut token, &mut output);
            output.push(ch);
        }
    }
    flush(&mut token, &mut output);
    output
}

pub fn has_mixed_script_homoglyphs(text: &str) -> bool {
    normalize_cyrillic_homoglyphs(text) != text
}

fn is_url_token(trimmed: &str) -> bool {
    let lower = trimmed.to_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

pub fn strip_links(text: &str) -> String {
    text.split_whitespace()
        .filter_map(|word| {
            let trimmed = word.trim_matches(|ch: char| {
                ch.is_ascii_punctuation()
                    || matches!(ch, '«' | '»' | '“' | '”' | '„' | '‹' | '›' | '【' | '】')
            });
            if is_url_token(trimmed) {
                return None;
            }
            // Glued text such as `look(https://example.com)`: drop the URL
            // part, keep the readable prefix when it exists.
            let lower = trimmed.to_lowercase();
            let marker = lower.find("http://").or_else(|| lower.find("https://"));
            if let Some(index) = marker {
                let prefix = trimmed[..index].trim_matches(|ch: char| {
                    ch.is_ascii_punctuation()
                        || matches!(ch, '«' | '»' | '“' | '”' | '„' | '‹' | '›' | '【' | '】')
                });
                return (!prefix.is_empty()).then(|| prefix.to_string());
            }
            Some(word.to_string())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Normalize personal-channel evidence for grounded comparisons:
/// lowercase, letters/numbers/whitespace only, collapsed whitespace.
pub fn normalize_channel_evidence(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// First-message campaign tokens: words of at least four characters,
/// with DM/funnel synonyms folded into canonical markers.
pub fn token_set(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() >= 4)
        .map(campaign_token)
        .collect()
}

fn campaign_token(word: &str) -> String {
    match word {
        "отправить"
        | "отправлю"
        | "переслать"
        | "перешлю"
        | "скинуть"
        | "скину"
        | "поделиться"
        | "поделюсь"
        | "закинуть"
        | "закину" => "send_offer".to_string(),
        "личку" | "личные" | "сообщения" | "стучитесь" => {
            "direct_messages".to_string()
        }
        "аудиокнигу" | "аудиокнига" | "аудиоверсия" | "текстовая" => {
            "promoted_material".to_string()
        }
        _ => word.to_owned(),
    }
}

/// Template Jaccard similarity. An empty union yields 0.0 so empty
/// messages do not match each other.
pub fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    let union = left.union(right).count();
    if union == 0 {
        0.0
    } else {
        left.intersection(right).count() as f64 / union as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_text_chars_marks_truncation() {
        assert_eq!(first_text_chars("abcdef", 3), "abc…");
        assert_eq!(first_text_chars("abc", 3), "abc");
    }

    #[test]
    fn strip_links_handles_wrapping_punctuation() {
        assert_eq!(strip_links("смотри (https://example.com), ок"), "смотри ок");
        assert_eq!(strip_links("смотри https://example.com. ок"), "смотри ок");
    }

    #[test]
    fn normalizes_latin_homoglyph_in_cyrillic_name() {
        assert_eq!(normalize_cyrillic_homoglyphs("Tанюша"), "Танюша");
        assert!(has_mixed_script_homoglyphs("Tанюша"));
        assert_eq!(normalize_cyrillic_homoglyphs("Alice"), "Alice");
    }

    #[test]
    fn pure_latin_brand_survives_cyrillic_context() {
        assert_eq!(
            normalize_cyrillic_homoglyphs("Привет OpenAI"),
            "Привет OpenAI"
        );
        assert!(!has_mixed_script_homoglyphs("Привет OpenAI"));
    }

    #[test]
    fn campaign_tokens_fold_dm_funnel_synonyms() {
        let tokens = token_set("Напишите в личку, скину аудиокнигу бесплатно");
        assert!(tokens.contains("send_offer"));
        assert!(tokens.contains("direct_messages"));
        assert!(tokens.contains("promoted_material"));
    }

    #[test]
    fn jaccard_ignores_empty_sets() {
        let empty = BTreeSet::new();
        assert_eq!(jaccard(&empty, &empty), 0.0);
    }
}
