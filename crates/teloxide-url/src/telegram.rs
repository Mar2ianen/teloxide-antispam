use crate::parse_url;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramPeer {
    Username(String),
    ChatId(i64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotStartKind {
    Start,
    StartGroup,
    StartChannel,
    StartApp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramLink {
    Profile {
        username: String,
    },
    Message {
        peer: TelegramPeer,
        message_id: i32,
        topic_id: Option<i32>,
    },
    /// The invite is intentionally not resolved to a chat ID.
    Invite {
        hash: String,
    },
    /// A numeric +... URL may mean a phone number, not an invite.
    AmbiguousPlus {
        value: String,
    },
    UserId(u64),
    BotStart {
        username: String,
        kind: BotStartKind,
        payload: String,
        app: Option<String>,
    },
    Share {
        url: Option<String>,
    },
    /// OAuth, login, proxy etc.; do not send their query to third-party scanners.
    Reserved {
        route: String,
    },
    Other {
        route: String,
    },
}

/// Strictly recognize Telegram domains and `tg:` URIs.
/// Does not perform network resolution of names, users, channels, or invites.
pub fn parse_telegram_link(raw: &str) -> Option<TelegramLink> {
    let url = parse_url(raw)?;
    if url.scheme() == "tg" {
        return Some(parse_tg(&url));
    }
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let subdomain_username = match host.as_str() {
        "t.me" | "www.t.me" | "telegram.me" | "www.telegram.me" | "telegram.dog"
        | "www.telegram.dog" => None,
        _ => {
            let name = host.strip_suffix(".t.me")?;
            if !valid_username(name) {
                return None;
            }
            Some(name)
        }
    };
    if let Some(name) = subdomain_username {
        let segments: Vec<&str> = path_segments(&url);
        let id = segments.first().and_then(|s| parse_positive_message_id(s));
        return Some(match (id, segments.len()) {
            (Some(message_id), 1) => TelegramLink::Message {
                peer: TelegramPeer::Username(name.into()),
                message_id,
                topic_id: None,
            },
            _ => TelegramLink::Profile {
                username: name.into(),
            },
        });
    }
    let path = path_segments(&url);
    let Some(&first) = path.first() else {
        return Some(TelegramLink::Other { route: "".into() });
    };
    let first_lower = first.to_ascii_lowercase();
    if first.starts_with('+') && first.len() > 1 && path.len() == 1 {
        let token = &first[1..];
        if !is_token(token) {
            return Some(TelegramLink::Other {
                route: "invalid_plus".into(),
            });
        }
        return Some(if token.bytes().all(|b| b.is_ascii_digit()) {
            TelegramLink::AmbiguousPlus {
                value: token.into(),
            }
        } else {
            TelegramLink::Invite { hash: token.into() }
        });
    }
    if first_lower == "joinchat" {
        if let Some(&hash) = path.get(1)
            && is_token(hash)
        {
            return Some(TelegramLink::Invite { hash: hash.into() });
        }
        return Some(TelegramLink::Other {
            route: "joinchat".into(),
        });
    }
    if first_lower == "c" {
        if let Some(v) = parse_private_message(&path) {
            return Some(v);
        }
        return Some(TelegramLink::Other { route: "c".into() });
    }
    if first_lower == "share" && path.get(1).is_some_and(|s| s.eq_ignore_ascii_case("url")) {
        return Some(TelegramLink::Share {
            url: get_query(&url, "url"),
        });
    }
    if is_reserved(first) {
        return Some(TelegramLink::Reserved { route: first_lower });
    }
    if !valid_username(first) {
        return Some(TelegramLink::Other {
            route: first.into(),
        });
    }
    let username = first.to_ascii_lowercase();
    if let Some((kind, payload)) = start_param(&url) {
        let app = path.get(1).filter(|s| !s.is_empty()).map(|s| s.to_string());
        return Some(TelegramLink::BotStart {
            username,
            kind,
            payload,
            app,
        });
    }
    if let Some(&second) = path.get(1) {
        if let Some(message_id) = parse_positive_message_id(second)
            && path.len() == 2
        {
            return Some(TelegramLink::Message {
                peer: TelegramPeer::Username(username),
                message_id,
                topic_id: None,
            });
        }
        if path.len() == 3
            && let (Some(topic_id), Some(message_id)) = (
                parse_positive_message_id(second),
                parse_positive_message_id(path[2]),
            )
        {
            return Some(TelegramLink::Message {
                peer: TelegramPeer::Username(username),
                message_id,
                topic_id: Some(topic_id),
            });
        }
        return Some(TelegramLink::Other {
            route: path.join("/"),
        });
    }
    Some(TelegramLink::Profile { username })
}

fn parse_tg(url: &Url) -> TelegramLink {
    let path = url
        .host_str()
        .unwrap_or_else(|| url.path().trim_matches('/'));
    let name = path.to_ascii_lowercase();
    match name.as_str() {
        "user" => get_query(url, "id")
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|id| *id > 0)
            .map_or_else(|| TelegramLink::Other { route: name }, TelegramLink::UserId),
        "resolve" => {
            let username = get_query(url, "domain");
            match username.filter(|s| valid_username(s)) {
                Some(username) => {
                    if let Some((kind, payload)) = start_param(url) {
                        return TelegramLink::BotStart {
                            username,
                            kind,
                            payload,
                            app: None,
                        };
                    }
                    if let Some(message_id) =
                        get_query(url, "post").and_then(|s| parse_positive_message_id(&s))
                    {
                        return TelegramLink::Message {
                            peer: TelegramPeer::Username(username.to_ascii_lowercase()),
                            message_id,
                            topic_id: None,
                        };
                    }
                    TelegramLink::Profile {
                        username: username.to_ascii_lowercase(),
                    }
                }
                None => TelegramLink::Other {
                    route: "resolve".into(),
                },
            }
        }
        "join" => get_query(url, "invite")
            .filter(|s| is_token(s))
            .map_or_else(
                || TelegramLink::Other { route: name },
                |hash| TelegramLink::Invite { hash },
            ),
        "msg_url" => TelegramLink::Share {
            url: get_query(url, "url"),
        },
        "login" | "proxy" | "socks" | "confirmphone" | "passport" => {
            TelegramLink::Reserved { route: name }
        }
        _ => TelegramLink::Other { route: name },
    }
}

fn parse_private_message(path: &[&str]) -> Option<TelegramLink> {
    let local = path
        .get(1)?
        .parse::<i64>()
        .ok()
        .filter(|n| *n > 0 && *n < 1_000_000_000_000)?;
    // t.me/c/123... is a channel/supergroup ID without the Bot API -100 prefix.
    let chat_id = (-1_000_000_000_000_i64).checked_sub(local)?;
    let (topic_id, message_id) = match path {
        [_, _, message] => (None, parse_positive_message_id(message)?),
        [_, _, topic, message] => (
            Some(parse_positive_message_id(topic)?),
            parse_positive_message_id(message)?,
        ),
        _ => return None,
    };
    Some(TelegramLink::Message {
        peer: TelegramPeer::ChatId(chat_id),
        message_id,
        topic_id,
    })
}

fn start_param(url: &Url) -> Option<(BotStartKind, String)> {
    for (key, value) in url.query_pairs() {
        let kind = match key.as_ref() {
            "start" => BotStartKind::Start,
            "startgroup" => BotStartKind::StartGroup,
            "startchannel" => BotStartKind::StartChannel,
            "startapp" => BotStartKind::StartApp,
            _ => continue,
        };
        return Some((kind, value.into_owned()));
    }
    None
}
fn get_query(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.into_owned())
}
fn path_segments(url: &Url) -> Vec<&str> {
    url.path()
        .split('/')
        .filter(|s| !s.is_empty())
        .take(8)
        .collect()
}
fn valid_username(s: &str) -> bool {
    (5..=32).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 256
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
fn is_reserved(name: &str) -> bool {
    [
        "oauth",
        "login",
        "addstickers",
        "addemoji",
        "proxy",
        "socks",
        "iv",
        "boost",
        "invoice",
        "giftcode",
    ]
    .contains(&name.to_ascii_lowercase().as_str())
}
fn parse_positive_message_id(s: &str) -> Option<i32> {
    s.parse::<i32>().ok().filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_link_taxonomy_preserves_numeric_identity_boundaries() {
        let mention = crate::find("Contact @alice_bot");
        assert!(
            mention
                .iter()
                .any(|item| { item.kind == crate::Kind::Mention && item.value == "@alice_bot" })
        );

        let profile = parse_telegram_link("https://t.me/Alice_Bot");
        let resolved_profile = parse_telegram_link("tg://resolve?domain=Alice_Bot");
        assert_eq!(
            profile,
            Some(TelegramLink::Profile {
                username: "alice_bot".into()
            })
        );
        assert_eq!(profile, resolved_profile);

        assert_eq!(
            parse_telegram_link("tg://user?id=424242"),
            Some(TelegramLink::UserId(424242))
        );
        assert_eq!(
            parse_telegram_link("https://t.me/c/123456/321/789"),
            Some(TelegramLink::Message {
                peer: TelegramPeer::ChatId(-1_000_000_123_456),
                message_id: 789,
                topic_id: Some(321),
            })
        );
        assert_ne!(
            parse_telegram_link("tg://user?id=424242"),
            parse_telegram_link("https://t.me/c/424242/1")
        );
    }

    #[test]
    fn invite_syntax_is_never_guessed_into_a_peer_identity() {
        for value in [
            "https://t.me/+SyntheticInviteHash",
            "https://t.me/joinchat/SyntheticInviteHash",
            "tg://join?invite=SyntheticInviteHash",
        ] {
            assert!(matches!(
                parse_telegram_link(value),
                Some(TelegramLink::Invite { .. })
            ));
        }
        assert!(matches!(
            parse_telegram_link("https://t.me/+123456789"),
            Some(TelegramLink::AmbiguousPlus { .. })
        ));
    }

    #[test]
    fn bot_start_kinds_and_mini_app_route_are_typed_without_resolving_username() {
        let cases = [
            (
                "https://t.me/launch_bot?start=synthetic",
                BotStartKind::Start,
            ),
            (
                "https://t.me/launch_bot?startgroup=synthetic",
                BotStartKind::StartGroup,
            ),
            (
                "https://t.me/launch_bot?startchannel=synthetic",
                BotStartKind::StartChannel,
            ),
            (
                "https://t.me/launch_bot/demo?startapp=synthetic",
                BotStartKind::StartApp,
            ),
        ];

        for (value, expected_kind) in cases {
            assert!(matches!(
                parse_telegram_link(value),
                Some(TelegramLink::BotStart { username, kind, .. })
                    if username == "launch_bot" && kind == expected_kind
            ));
        }
        assert!(matches!(
            parse_telegram_link("https://t.me/launch_bot/demo?startapp=synthetic"),
            Some(TelegramLink::BotStart {
                app: Some(app),
                ..
            }) if app == "demo"
        ));
    }

    #[test]
    fn bot_like_username_suffixes_do_not_change_parser_identity() {
        for username in [
            "ordinarybot",
            "ordinaryagent",
            "ordinaryai",
            "ordinaryapp",
            "ordinarygame",
            "ordinarygay",
        ] {
            assert_eq!(
                parse_telegram_link(&format!("https://t.me/{username}")),
                Some(TelegramLink::Profile {
                    username: username.into()
                })
            );
        }
    }

    #[test]
    fn malformed_numeric_and_message_peer_values_remain_unresolved_routes() {
        for value in [
            "tg://user?id=0",
            "tg://user?id=-1",
            "tg://user?id=18446744073709551616",
            "https://t.me/c/0/7",
            "https://t.me/c/123456/0",
            "https://t.me/c/123456/1/0",
            "tg://resolve?domain=invalid-name",
        ] {
            assert!(matches!(
                parse_telegram_link(value),
                Some(TelegramLink::Other { .. })
            ));
        }
    }

    #[test]
    fn private_chat_ids_are_only_calculated_from_c_routes() {
        assert_eq!(
            parse_telegram_link("t.me/c/123456/789"),
            Some(TelegramLink::Message {
                peer: TelegramPeer::ChatId(-1_000_000_123_456),
                message_id: 789,
                topic_id: None
            })
        );
        assert!(matches!(
            parse_telegram_link("https://t.me/c/123456/321/789"),
            Some(TelegramLink::Message {
                topic_id: Some(321),
                ..
            })
        ));
    }
    #[test]
    fn spoofed_host_cannot_become_telegram() {
        assert!(parse_telegram_link("https://t.me.evil.example/alice").is_none());
        assert!(parse_telegram_link("https://t.me@evil.example/alice").is_none());
    }
    #[test]
    fn no_fake_id_for_username() {
        assert_eq!(
            parse_telegram_link("t.me/alice"),
            Some(TelegramLink::Profile {
                username: "alice".into()
            })
        );
        assert!(matches!(
            parse_telegram_link("tg://user?id=42"),
            Some(TelegramLink::UserId(42))
        ));
    }
    #[test]
    fn hidden_secrets_are_not_unwrapped_into_external_urls() {
        assert!(matches!(
            parse_telegram_link("tg://login?token=SYNTHETIC"),
            Some(TelegramLink::Reserved { .. })
        ));
        assert!(matches!(
            parse_telegram_link("t.me/+79991234567"),
            Some(TelegramLink::AmbiguousPlus { .. })
        ));
        assert!(matches!(
            parse_telegram_link("t.me/+12345"),
            Some(TelegramLink::AmbiguousPlus { .. })
        ));
    }
    #[test]
    fn bot_deep_link_and_share() {
        assert!(matches!(
            parse_telegram_link("t.me/some_bot/app?startapp=abc"),
            Some(TelegramLink::BotStart {
                kind: BotStartKind::StartApp,
                ..
            })
        ));
        assert!(matches!(
            parse_telegram_link("t.me/share/url?url=https%3A%2F%2Fexample.org"),
            Some(TelegramLink::Share { .. })
        ));
    }
}
