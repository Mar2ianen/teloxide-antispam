use teloxide_url::{
    BotStartKind, Kind, TelegramLink, TelegramPeer, find, find_urls, parse_telegram_link,
};

#[test]
fn url_parser_cases_from_handoff() {
    let cases = include_str!("fixtures/cases.jsonl");
    for line in cases.lines() {
        let data: serde_json::Value = serde_json::from_str(line).unwrap();
        let text = data["input"].as_str().unwrap();
        let id = data["id"].as_str().unwrap();
        let refs = find(text);
        let urls = find_urls(text);
        match id {
            "telegram_mention" => assert!(refs.iter().any(|r| r.kind == Kind::Mention), "{id}"),
            "telegram_tag" => assert!(refs.iter().any(|r| r.kind == Kind::Hashtag), "{id}"),
            "telegram_command" => assert!(refs.iter().any(|r| r.kind == Kind::BotCommand), "{id}"),
            "telegram_cashtag" => assert!(refs.iter().any(|r| r.kind == Kind::Cashtag), "{id}"),
            "telegram_fwd" => assert_eq!(urls.len(), 2, "{id}"),
            "security_host_confusion" | "security_userinfo" => {
                assert_eq!(urls.len(), 1, "{id}");
                assert!(parse_telegram_link(&urls[0].value).is_none(), "{id}");
            }
            "telegram_message_private" => assert!(
                matches!(
                    parse_telegram_link(text),
                    Some(TelegramLink::Message {
                        peer: TelegramPeer::ChatId(_),
                        topic_id: None,
                        ..
                    })
                ),
                "{id}"
            ),
            "telegram_message_private_topic" => assert!(
                matches!(
                    parse_telegram_link(text),
                    Some(TelegramLink::Message {
                        peer: TelegramPeer::ChatId(_),
                        topic_id: Some(_),
                        ..
                    })
                ),
                "{id}"
            ),
            "telegram_plus_ambiguous" => assert!(
                matches!(
                    parse_telegram_link(text),
                    Some(TelegramLink::AmbiguousPlus { .. })
                ),
                "{id}"
            ),
            "telegram_bot_startapp" | "telegram_bot_directapp" => assert!(
                matches!(
                    parse_telegram_link(text),
                    Some(TelegramLink::BotStart {
                        kind: BotStartKind::StartApp,
                        ..
                    })
                ),
                "{id}"
            ),
            "telegram_join" | "telegram_plus_invite" => assert!(
                matches!(parse_telegram_link(text), Some(TelegramLink::Invite { .. })),
                "{id}"
            ),
            "tg_user_id" => assert!(
                matches!(parse_telegram_link(text), Some(TelegramLink::UserId(_))),
                "{id}"
            ),
            "tg_login_secret" | "telegram_reserved" => assert!(
                matches!(
                    parse_telegram_link(text),
                    Some(TelegramLink::Reserved { .. })
                ),
                "{id}"
            ),
            "telegram_share_url" => assert!(
                matches!(parse_telegram_link(text), Some(TelegramLink::Share { .. })),
                "{id}"
            ),
            "telegram_message_public" | "telegram_subdomain" => assert!(
                matches!(
                    parse_telegram_link(text),
                    Some(TelegramLink::Message { .. })
                ),
                "{id}"
            ),
            "telegram_user_link" | "telegram_alias" | "tg_resolve" => assert!(
                matches!(
                    parse_telegram_link(text),
                    Some(TelegramLink::Profile { .. })
                ),
                "{id}"
            ),
            _ => assert!(!urls.is_empty(), "url missing for {id}"),
        }
    }
}
