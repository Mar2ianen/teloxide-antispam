//! Small, deterministic URL and Telegram reference extraction.
//!
//! No network calls, persistence, reputation, Telegram API calls, or moderation policy.
//! The optional `teloxide` feature adapts native Telegram entities and rich messages.

mod extract;
mod telegram;

#[cfg(feature = "teloxide")]
pub mod adapter;

pub use extract::{Found, Kind, MAX_ITEMS, Source, find, find_urls, parse_url, utf16_range};
pub use telegram::{
    BotStartKind, TelegramLink, TelegramPeer, parse_custom_emoji_uri, parse_telegram_link,
};
