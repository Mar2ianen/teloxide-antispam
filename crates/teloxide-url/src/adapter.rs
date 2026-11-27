//! Adapter for the pinned Mar2ianen teloxide fork's typed entities/RichMessage.
//! No `Bot` or network calls. All spans are byte ranges, after UTF-16 conversion.
use teloxide::types::{
    MediaKind, Message, MessageEntity, MessageEntityKind, MessageKind, RichBlock, RichBlockCaption,
    RichBlockKind, RichMessageButton, RichText, RichTextObject,
};

use crate::extract::MAX_URL_BYTES;
use crate::{Found, Kind, MAX_ITEMS, Source, find, parse_telegram_link, parse_url, utf16_range};

fn url_kind(value: &str) -> Kind {
    match parse_telegram_link(value) {
        Some(crate::TelegramLink::CustomEmojiFallback { id }) => Kind::CustomEmojiFallback { id },
        _ => Kind::Url,
    }
}

const MAX_RICH_NODES: usize = 1024;
const MAX_RICH_DEPTH: usize = 24;

/// Extract references from a Telegram message; use one shared parser for OCR and QR strings.
/// Native entities override fallback visible-text matches on the same span.
pub fn extract_from_message(message: &Message) -> Vec<Found> {
    let mut output = Vec::new();
    if let Some(text) = message.text() {
        output.extend(extract_text_entities(
            text,
            message.entities().unwrap_or_default(),
        ));
    } else if let Some(caption) = message.caption() {
        output.extend(extract_text_entities(
            caption,
            message.caption_entities().unwrap_or_default(),
        ));
    }
    if let MessageKind::Common(common) = &message.kind
        && let MediaKind::RichMessage(media) = &common.media_kind
    {
        let mut walk = RichWalker::default();
        walk.blocks(&media.rich_message.blocks, 0);
        output.extend(walk.results);
    }
    output.truncate(MAX_ITEMS);
    output
}

pub fn extract_text_entities(text: &str, entities: &[MessageEntity]) -> Vec<Found> {
    let mut out = find(text);
    let mut native = Vec::new();
    for e in entities.iter().take(MAX_RICH_NODES) {
        if native.len() >= MAX_ITEMS {
            break;
        }
        let Some(span) = utf16_range(text, e.offset, e.length) else {
            continue;
        };
        let Some(visible) = text.get(span.clone()) else {
            continue;
        };
        let item = match &e.kind {
            MessageEntityKind::Url if parse_url(visible).is_some() => Found {
                value: visible.to_owned(),
                kind: url_kind(visible),
                source: Source::TelegramEntity,
                span: Some(span.clone()),
                label: None,
            },
            MessageEntityKind::TextLink { url } => Found {
                value: url.as_str().to_owned(),
                kind: url_kind(url.as_str()),
                source: Source::TelegramEntity,
                span: Some(span.clone()),
                label: Some(visible.into()),
            },
            MessageEntityKind::Mention => Found {
                value: visible.into(),
                kind: Kind::Mention,
                source: Source::TelegramEntity,
                span: Some(span.clone()),
                label: None,
            },
            MessageEntityKind::TextMention { user } => Found {
                value: visible.into(),
                kind: Kind::UserId(user.id.0),
                source: Source::TelegramEntity,
                span: Some(span.clone()),
                label: None,
            },
            MessageEntityKind::Hashtag => Found {
                value: visible.into(),
                kind: Kind::Hashtag,
                source: Source::TelegramEntity,
                span: Some(span.clone()),
                label: None,
            },
            MessageEntityKind::Cashtag => Found {
                value: visible.into(),
                kind: Kind::Cashtag,
                source: Source::TelegramEntity,
                span: Some(span.clone()),
                label: None,
            },
            MessageEntityKind::BotCommand => Found {
                value: visible.into(),
                kind: Kind::BotCommand,
                source: Source::TelegramEntity,
                span: Some(span.clone()),
                label: None,
            },
            _ => continue,
        };
        if item.value.len() <= MAX_URL_BYTES {
            native.push(item);
        }
    }
    // Prefer Telegram's typed link target over the same visible text matched by linkify.
    for n in &native {
        if let Some(span) = &n.span {
            out.retain(|v| {
                !v.span
                    .as_ref()
                    .is_some_and(|s| s.start < span.end && span.start < s.end)
            });
        }
    }
    out.extend(native);
    out.sort_by_key(|item| item.span.as_ref().map_or(usize::MAX, |s| s.start));
    out.truncate(MAX_ITEMS);
    out
}

#[derive(Default)]
struct RichWalker {
    results: Vec<Found>,
    visited: usize,
}
impl RichWalker {
    fn active(&mut self, depth: usize) -> bool {
        if depth >= MAX_RICH_DEPTH
            || self.visited >= MAX_RICH_NODES
            || self.results.len() >= MAX_ITEMS
        {
            return false;
        }
        self.visited += 1;
        true
    }
    fn record(&mut self, value: &str, kind: Kind, label: Option<String>) {
        if self.results.len() < MAX_ITEMS && !value.is_empty() && value.len() <= MAX_URL_BYTES {
            self.results.push(Found {
                value: value.into(),
                kind,
                source: Source::RichMessage,
                span: None,
                label,
            });
        }
    }
    fn button(&mut self, button: &RichMessageButton, depth: usize) {
        if !self.active(depth) {
            return;
        }
        if let Some(url) = button.url.as_deref()
            && parse_url(url).is_some()
        {
            self.record(url, Kind::Url, Some(label(&button.text)));
        }
        self.inline(&button.text, depth + 1);
    }
    fn inline(&mut self, text: &RichText, depth: usize) {
        if !self.active(depth) {
            return;
        }
        match text {
            RichText::Text(s) => {
                for item in find(s) {
                    if self.results.len() == MAX_ITEMS {
                        break;
                    }
                    self.record(&item.value, item.kind, item.label);
                }
            }
            RichText::List(items) => {
                for item in items.iter().take(MAX_RICH_NODES) {
                    self.inline(item, depth + 1);
                }
            }
            RichText::Object(obj) => match obj {
                RichTextObject::Url(v) => {
                    if parse_url(&v.url).is_some() {
                        self.record(&v.url, url_kind(&v.url), Some(label(&v.text)));
                    }
                    self.inline(&v.text, depth + 1);
                }
                RichTextObject::TextMention(v) => {
                    self.record(&label(&v.text), Kind::UserId(v.user.id.0), None);
                    self.inline(&v.text, depth + 1);
                }
                RichTextObject::Mention(v) => {
                    self.record(&v.username, Kind::Mention, None);
                    self.inline(&v.text, depth + 1);
                }
                RichTextObject::Hashtag(v) => {
                    self.record(&v.hashtag, Kind::Hashtag, None);
                    self.inline(&v.text, depth + 1);
                }
                RichTextObject::Cashtag(v) => {
                    self.record(&v.cashtag, Kind::Cashtag, None);
                    self.inline(&v.text, depth + 1);
                }
                RichTextObject::BotCommand(v) => {
                    self.record(&v.bot_command, Kind::BotCommand, None);
                    self.inline(&v.text, depth + 1);
                }
                RichTextObject::Button(v) => self.button(&v.button, depth + 1),
                RichTextObject::Bold(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Italic(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Underline(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Strikethrough(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Spoiler(v) => self.inline(&v.text, depth + 1),
                RichTextObject::DateTime(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Subscript(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Superscript(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Marked(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Code(v) => self.inline(&v.text, depth + 1),
                RichTextObject::EmailAddress(v) => self.inline(&v.text, depth + 1),
                RichTextObject::PhoneNumber(v) => self.inline(&v.text, depth + 1),
                RichTextObject::BankCardNumber(v) => self.inline(&v.text, depth + 1),
                RichTextObject::TonAddress(v) => self.inline(&v.text, depth + 1),
                RichTextObject::AnchorLink(v) => self.inline(&v.text, depth + 1),
                RichTextObject::Reference(v) => self.inline(&v.text, depth + 1),
                RichTextObject::ReferenceLink(v) => self.inline(&v.text, depth + 1),
                RichTextObject::CustomEmoji(_)
                | RichTextObject::MathematicalExpression(_)
                | RichTextObject::Anchor(_)
                | RichTextObject::Unknown(_) => {}
            },
        }
    }
    fn caption(&mut self, v: &Option<RichBlockCaption>, depth: usize) {
        if let Some(c) = v {
            self.inline(&c.text, depth + 1);
            if let Some(credit) = &c.credit {
                self.inline(credit, depth + 1);
            }
        }
    }
    fn blocks(&mut self, blocks: &[RichBlock], depth: usize) {
        if !self.active(depth) {
            return;
        }
        for block in blocks {
            if self.results.len() >= MAX_ITEMS || self.visited >= MAX_RICH_NODES {
                break;
            }
            let RichBlock::Known(block) = block else {
                continue;
            };
            match block.as_ref() {
                RichBlockKind::Paragraph(v) => self.inline(&v.text, depth + 1),
                RichBlockKind::Heading(v) => self.inline(&v.text, depth + 1),
                RichBlockKind::Pre(v) => self.inline(&v.text, depth + 1),
                RichBlockKind::Footer(v) => self.inline(&v.text, depth + 1),
                RichBlockKind::List(v) => {
                    for item in v.items.iter().take(MAX_RICH_NODES) {
                        self.blocks(&item.blocks, depth + 1);
                    }
                }
                RichBlockKind::Blockquote(v) => {
                    self.blocks(&v.blocks, depth + 1);
                    if let Some(t) = &v.credit {
                        self.inline(t, depth + 1);
                    }
                }
                RichBlockKind::ExpandableBlockquote(v) => {
                    self.blocks(&v.blocks, depth + 1);
                    if let Some(t) = &v.credit {
                        self.inline(t, depth + 1);
                    }
                }
                RichBlockKind::Pullquote(v) => {
                    self.inline(&v.text, depth + 1);
                    if let Some(t) = &v.credit {
                        self.inline(t, depth + 1);
                    }
                }
                RichBlockKind::Collage(v) => {
                    self.blocks(&v.blocks, depth + 1);
                    self.caption(&v.caption, depth + 1);
                }
                RichBlockKind::Slideshow(v) => {
                    self.blocks(&v.blocks, depth + 1);
                    self.caption(&v.caption, depth + 1);
                }
                RichBlockKind::Table(v) => {
                    for row in v.cells.iter().take(MAX_RICH_NODES) {
                        for cell in row.iter().take(MAX_RICH_NODES) {
                            if let Some(t) = &cell.text {
                                self.inline(t, depth + 1);
                            }
                        }
                    }
                    if let Some(t) = &v.caption {
                        self.inline(t, depth + 1);
                    }
                }
                RichBlockKind::Buttons(v) => {
                    for b in v.buttons.iter().take(MAX_RICH_NODES) {
                        self.button(b, depth + 1);
                    }
                }
                RichBlockKind::Details(v) => {
                    self.inline(&v.summary, depth + 1);
                    self.blocks(&v.blocks, depth + 1);
                }
                RichBlockKind::Document(v) => self.caption(&v.caption, depth + 1),
                RichBlockKind::Map(v) => self.caption(&v.caption, depth + 1),
                RichBlockKind::Animation(v) => self.caption(&v.caption, depth + 1),
                RichBlockKind::Audio(v) => self.caption(&v.caption, depth + 1),
                RichBlockKind::Photo(v) => self.caption(&v.caption, depth + 1),
                RichBlockKind::Video(v) => self.caption(&v.caption, depth + 1),
                RichBlockKind::VoiceNote(v) => self.caption(&v.caption, depth + 1),
                RichBlockKind::Divider(_)
                | RichBlockKind::Anchor(_)
                | RichBlockKind::Thinking(_)
                | RichBlockKind::MathematicalExpression(_) => {}
            }
        }
    }
}

/// Bounded display string for link labels; not an authority for target resolution.
fn label(text: &RichText) -> String {
    fn append(out: &mut String, text: &RichText, depth: usize) {
        if out.len() >= 256 || depth > 12 {
            return;
        }
        match text {
            RichText::Text(t) => {
                for c in t.chars() {
                    if out.len() + c.len_utf8() > 256 {
                        break;
                    }
                    out.push(c);
                }
            }
            RichText::List(v) => {
                for item in v.iter().take(256) {
                    if out.len() >= 256 {
                        break;
                    }
                    append(out, item, depth + 1);
                }
            }
            RichText::Object(v) => match v {
                RichTextObject::Bold(x) => append(out, &x.text, depth + 1),
                RichTextObject::Italic(x) => append(out, &x.text, depth + 1),
                RichTextObject::Url(x) => append(out, &x.text, depth + 1),
                RichTextObject::TextMention(x) => append(out, &x.text, depth + 1),
                RichTextObject::Mention(x) => append(out, &x.text, depth + 1),
                _ => {}
            },
        }
    }
    let mut out = String::new();
    append(&mut out, text, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use teloxide::types::RichMessage;
    #[test]
    fn rich_hidden_target_and_text_mention() {
        let rich: RichMessage = serde_json::from_value(serde_json::json!({"blocks":[
            {"type":"paragraph","text":["Смотри ",{"type":"url","url":"https://example.org/hidden","text":"нажми"}]},
            {"type":"details","summary":"Скрыто","blocks":[{"type":"buttons","buttons":[
                {"text":"Сайт","url":"https://example.org/button"}]}]},
            {"type":"thinking","text":"https://private.example"}
        ]})).unwrap();
        let mut w = RichWalker::default();
        w.blocks(&rich.blocks, 0);
        assert!(w.results.iter().any(
            |f| f.value == "https://example.org/hidden" && f.label.as_deref() == Some("нажми")
        ));
        assert!(
            w.results
                .iter()
                .any(|f| f.value == "https://example.org/button")
        );
        assert!(
            !w.results
                .iter()
                .any(|f| f.value.contains("private.example"))
        );
    }
    #[test]
    fn rich_expandable_quote_recurses_and_preserves_hidden_links() {
        let rich: RichMessage = serde_json::from_value(serde_json::json!({
            "blocks": [{
                "type": "expandable_blockquote",
                "blocks": [
                    {"type":"paragraph","text":{"type":"url",
                        "url":"https://example.org/inside","text":"открыть"}},
                    {"type":"details","summary":"детали","blocks":[
                        {"type":"paragraph","text":"@nested_channel"}
                    ]}
                ],
                "credit": "источник"
            }]
        }))
        .unwrap();
        let mut walker = RichWalker::default();
        walker.blocks(&rich.blocks, 0);
        assert!(walker.results.iter().any(|v| {
            v.value == "https://example.org/inside" && v.label.as_deref() == Some("открыть")
        }));
        assert!(walker.results.iter().any(|v| v.value == "@nested_channel"));
    }
    #[test]
    fn hidden_link_overrides_visible_url() {
        let entity: MessageEntity = serde_json::from_value(serde_json::json!({
            "type":"text_link","offset":0,"length":7,"url":"https://evil.example"
        }))
        .unwrap();
        let found = extract_text_entities("example", &[entity]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].value, "https://evil.example/");
        assert_eq!(found[0].source, Source::TelegramEntity);
    }

    #[test]
    fn custom_emoji_url_is_only_classified_from_addressed_entities() {
        let entity: MessageEntity = serde_json::from_value(serde_json::json!({
            "type":"text_link","offset":0,"length":2,"url":"tg://emoji?id=00042"
        }))
        .unwrap();
        let found = extract_text_entities("😀", &[entity]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].kind, Kind::CustomEmojiFallback { id: 42 });
        assert_eq!(found[0].source, Source::TelegramEntity);
        assert_eq!(found[0].span, Some(0..4));
        assert_eq!(found[0].label.as_deref(), Some("😀"));

        let plain = find("tg://emoji?id=42");
        assert_eq!(plain.len(), 1);
        assert_eq!(plain[0].kind, Kind::Url);
        assert_eq!(plain[0].source, Source::VisibleText);
    }

    #[test]
    fn rich_text_custom_emoji_url_remains_a_candidate_with_its_label() {
        let rich: RichMessage = serde_json::from_value(serde_json::json!({"blocks":[{
            "type":"paragraph","text":{"type":"url",
              "url":"tg://emoji?id=42", "text":"😀"}
        }]}))
        .unwrap();
        let mut walker = RichWalker::default();
        walker.blocks(&rich.blocks, 0);
        assert!(walker.results.iter().any(|found| {
            found.kind == Kind::CustomEmojiFallback { id: 42 }
                && found.source == Source::RichMessage
                && found.label.as_deref() == Some("😀")
        }));
    }
    #[test]
    fn rejects_split_surrogate_offset() {
        let entity: MessageEntity = serde_json::from_value(serde_json::json!({
            "type":"url","offset":1,"length":6
        }))
        .unwrap();
        assert!(extract_text_entities("😺hello!", &[entity]).is_empty());
    }
    #[test]
    fn actual_native_user_id_is_preserved() {
        let e: MessageEntity = serde_json::from_value(serde_json::json!({
            "type":"text_mention","offset":0,"length":5,
            "user":{"id":42,"is_bot":false,"first_name":"Alice"}
        }))
        .unwrap();
        let values = extract_text_entities("Alice", &[e]);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].kind, Kind::UserId(42));
    }

    #[test]
    fn utf16_offset_after_emoji_recovers_hidden_url() {
        let e: MessageEntity = serde_json::from_value(serde_json::json!({
            "type":"text_link", "offset":2, "length":6,
            "url":"https://example.org/secret"
        }))
        .unwrap();
        let values = extract_text_entities("😺смотри", &[e]);
        assert!(
            values
                .iter()
                .any(|v| v.value == "https://example.org/secret" && v.span == Some(4..16))
        );
    }

    #[test]
    fn rich_text_mention_keeps_numeric_id() {
        let rich: RichMessage = serde_json::from_value(serde_json::json!({"blocks":[{
            "type":"paragraph","text":{"type":"text_mention",
              "text":"Alice", "user":{"id":42,"is_bot":false,"first_name":"Alice"}}
        }]}))
        .unwrap();
        let mut w = RichWalker::default();
        w.blocks(&rich.blocks, 0);
        assert!(w.results.iter().any(|v| v.kind == Kind::UserId(42)));
    }
}
