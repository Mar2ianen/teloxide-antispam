use std::ops::Range;

use linkify::{LinkFinder, LinkKind};
use url::Url;

/// Hard input/output bounds, deliberately independent of the application.
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const MAX_URL_BYTES: usize = 4096;
pub const MAX_ITEMS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Url,
    Mention,
    /// Native Telegram text_mention identifies an actual user (not a guessed ID).
    UserId(u64),
    Hashtag,
    Cashtag,
    BotCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    VisibleText,
    TelegramEntity,
    RichMessage,
    Qr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// URL target or literal mention/tag/command. Never auto-follow or log unredacted.
    pub value: String,
    pub kind: Kind,
    pub source: Source,
    /// Byte range within the original visible text; absent for hidden/Rich targets.
    pub span: Option<Range<usize>>,
    /// Display text for links whose actual URL differs from what the user sees.
    pub label: Option<String>,
    /// Strict `tg://emoji?id=` candidate on an addressed Telegram URL entity
    /// or rich URL node. It does not validate the label or grant an exemption.
    pub custom_emoji_fallback_id: Option<u64>,
}

impl Found {
    pub fn new(value: impl Into<String>, kind: Kind) -> Self {
        Self {
            value: value.into(),
            kind,
            source: Source::VisibleText,
            span: None,
            label: None,
            custom_emoji_fallback_id: None,
        }
    }
}

/// Parses HTTP(S) and Telegram URI schemes; bare hostnames get `https://`.
/// An unknown or invalid scheme is *not* rewritten into a valid web URL.
/// Parsing != safety checking.
pub fn parse_url(raw: &str) -> Option<Url> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > MAX_URL_BYTES || raw.chars().any(char::is_whitespace) {
        return None;
    }
    let absolute =
        raw.get(..3).is_some_and(|s| s.eq_ignore_ascii_case("tg:")) || raw.contains("://");
    let parsed = if absolute {
        Url::parse(raw).ok()?
    } else {
        Url::parse(&format!("https://{raw}")).ok()?
    };
    match parsed.scheme() {
        "http" | "https" if parsed.host_str().is_some() => Some(parsed),
        "tg" if !parsed.path().is_empty() || parsed.host_str().is_some() => Some(parsed),
        _ => None,
    }
}

pub fn find_urls(text: &str) -> Vec<Found> {
    let text = bounded(text);
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url]);
    finder.url_must_have_scheme(false);

    let mut result = Vec::new();
    for item in finder.links(text) {
        if result.len() >= MAX_ITEMS {
            break;
        }
        if has_url_prefix_boundary(text, item.start()) {
            continue;
        }
        let raw = item.as_str();
        if parse_url(raw).is_none() {
            continue;
        }
        result.push(Found {
            value: raw.to_owned(),
            kind: Kind::Url,
            source: Source::VisibleText,
            span: Some(item.start()..item.end()),
            label: None,
            custom_emoji_fallback_id: None,
        });
    }
    // `linkify` intentionally skips some valid bracketed IP literals such as
    // `http://[2001:db8::1]/`. Scan explicit web schemes as bounded tokens too;
    // `Url` remains the authority on whether the candidate is syntactically valid.
    for (index, _) in text.char_indices().filter(|(i, _)| {
        text.get(*i..*i + 7)
            .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
            || text
                .get(*i..*i + 8)
                .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    }) {
        if result.len() >= MAX_ITEMS {
            break;
        }
        if has_url_prefix_boundary(text, index) {
            continue;
        }
        let rest = &text[index..];
        let n = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
            .unwrap_or(rest.len());
        let raw = rest[..n].trim_end_matches([',', '.', ')', ']', '!', ';']);
        if parse_url(raw).is_some() {
            let end = index + raw.len();
            if !result.iter().any(|found| {
                found
                    .span
                    .as_ref()
                    .is_some_and(|s| s.start == index && s.end >= end)
            }) {
                result.push(Found {
                    value: raw.into(),
                    kind: Kind::Url,
                    source: Source::VisibleText,
                    span: Some(index..end),
                    label: None,
                    custom_emoji_fallback_id: None,
                });
            }
        }
    }
    // `linkify` has no obligation to understand the custom tg: scheme.
    // Find it explicitly without interpreting an arbitrary URI as an HTTP URL.
    for (index, _) in text.char_indices().filter(|(i, _)| {
        text.get(*i..*i + 3)
            .is_some_and(|s| s.eq_ignore_ascii_case("tg:"))
    }) {
        if result.len() >= MAX_ITEMS {
            break;
        }
        if has_url_prefix_boundary(text, index) {
            continue;
        }
        let rest = &text[index..];
        let n = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\''))
            .unwrap_or(rest.len());
        let raw = rest[..n].trim_end_matches([',', '.', ')', ']', '!', ';']);
        if parse_url(raw).is_some() {
            let end = index + raw.len();
            if !result.iter().any(|found| {
                found
                    .span
                    .as_ref()
                    .is_some_and(|s| s.start <= index && s.end >= end)
            }) {
                result.push(Found {
                    value: raw.into(),
                    kind: Kind::Url,
                    source: Source::VisibleText,
                    span: Some(index..end),
                    label: None,
                    custom_emoji_fallback_id: None,
                });
            }
        }
    }
    result.sort_by_key(|v| v.span.as_ref().map_or(usize::MAX, |s| s.start));
    result.truncate(MAX_ITEMS);
    result
}

/// Finds URLs, mentions, hashtags, cashtags and bot commands in ordinary text.
/// Native Telegram entities remain authoritative; this is a fallback for unmarked text / OCR / QR.
pub fn find(text: &str) -> Vec<Found> {
    let text = bounded(text);
    let mut found = find_urls(text);
    for (start, ch) in text.char_indices() {
        if found.len() >= MAX_ITEMS {
            break;
        }
        let kind = match ch {
            '@' => Kind::Mention,
            '#' => Kind::Hashtag,
            '$' => Kind::Cashtag,
            '/' => Kind::BotCommand,
            _ => continue,
        };
        // Do not scan tokens inside actual URLs and avoid matching an email address.
        if found.iter().any(|f| {
            f.span
                .as_ref()
                .is_some_and(|s| s.start <= start && start < s.end)
        }) {
            continue;
        }
        if start > 0 {
            let prev = text[..start].chars().next_back().unwrap_or(' ');
            if prev.is_alphanumeric() || prev == '_' || (ch == '/' && prev == ':') {
                continue;
            }
        }
        let tail = &text[start + ch.len_utf8()..];
        let token_len = tail
            .char_indices()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .last()
            .map_or(0, |(i, c)| i + c.len_utf8());
        let end = start + ch.len_utf8() + token_len;
        let token = &text[start..end];
        if token_len == 0 || token.len() > 128 {
            continue;
        }
        // Telegram user handles are ASCII with 5..=32 characters.
        if matches!(kind, Kind::Mention)
            && (!(5..=32).contains(&token_len)
                || !tail[..token_len]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        {
            continue;
        }
        // Telegram commands only have ASCII letters, digits, underscore.
        if matches!(kind, Kind::BotCommand)
            && !tail[..token_len]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            continue;
        }
        let mut value = token.to_owned();
        if matches!(kind, Kind::Hashtag) && text[end..].starts_with('@') {
            let rest = &text[end + 1..];
            let suffix = rest
                .bytes()
                .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
                .count();
            if (5..=32).contains(&suffix) {
                value.push('@');
                value.push_str(&rest[..suffix]);
            }
        }
        let end = start + value.len();
        if found.iter().any(|f| {
            f.span
                .as_ref()
                .is_some_and(|s| s.start < end && start < s.end)
        }) {
            continue;
        }
        found.push(Found {
            value,
            kind,
            source: Source::VisibleText,
            span: Some(start..end),
            label: None,
            custom_emoji_fallback_id: None,
        });
    }
    found.sort_by_key(|v| v.span.as_ref().map_or(usize::MAX, |s| s.start));
    found.truncate(MAX_ITEMS);
    found
}

/// Convert Telegram's UTF-16 offsets to Rust byte offsets, safely.
pub fn utf16_range(text: &str, offset: usize, length: usize) -> Option<Range<usize>> {
    let end = offset.checked_add(length)?;
    fn at(text: &str, target: usize) -> Option<usize> {
        let mut units = 0usize;
        for (byte, c) in text.char_indices() {
            if units == target {
                return Some(byte);
            }
            units = units.checked_add(c.len_utf16())?;
            if units > target {
                return None;
            }
        }
        (units == target).then_some(text.len())
    }
    Some(at(text, offset)?..at(text, end)?)
}

fn bounded(text: &str) -> &str {
    let mut end = text.len().min(MAX_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn has_url_prefix_boundary(text: &str, index: usize) -> bool {
    text[..index]
        .chars()
        .next_back()
        .is_some_and(|previous| previous.is_alphanumeric() || matches!(previous, '_' | '@'))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_entity_offsets() {
        assert_eq!(utf16_range("😺привет", 2, 6), Some(4..16));
        assert_eq!(utf16_range("😺привет", 1, 2), None);
    }
    #[test]
    fn target_host_not_fake_prefix() {
        assert_eq!(
            parse_url("https://t.me@evil.example/").unwrap().host_str(),
            Some("evil.example")
        );
    }
    #[test]
    fn finds_custom_tg_uris_without_web_scheme() {
        assert_eq!(find_urls("Тут TG:resolve?domain=alice").len(), 1);
        assert!(parse_url("TG:resolve?domain=alice").is_some());
    }
    #[test]
    fn extracting_text_handles_punctuation() {
        let v = find("Привет @alice! https://example.org/a, #help@mychat /start@some_bot");
        assert!(v.iter().any(|f| f.value == "@alice"));
        assert!(v.iter().any(|f| f.value == "https://example.org/a"));
        assert!(v.iter().any(|f| f.value == "#help@mychat"));
        assert!(v.iter().any(|f| f.value == "/start"));
    }

    #[test]
    fn extracts_bracketed_ipv6_urls_and_rejects_unicode_usernames() {
        let urls = find_urls("IPv6 http://[2001:db8::1]/path.");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].value, "http://[2001:db8::1]/path");
        assert!(
            find("Пишите @абвгде")
                .iter()
                .all(|v| v.kind != Kind::Mention)
        );
        assert!(find_urls("кhttp://example.org").is_empty());
    }

    #[test]
    fn malformed_offsets_and_input_limits_never_panic() {
        let text = "😺https://example.org";
        for offset in 0..=text.encode_utf16().count() + 2 {
            for length in 0..=text.encode_utf16().count() + 2 {
                if let Some(range) = utf16_range(text, offset, length) {
                    assert!(text.is_char_boundary(range.start));
                    assert!(text.is_char_boundary(range.end));
                }
            }
        }
        let oversized = format!("{}https://example.org", "a".repeat(MAX_TEXT_BYTES));
        assert!(find_urls(&oversized).is_empty());
    }
}
