# teloxide-url

Small MIT library for extracting ordinary URLs, Telegram links, usernames,
mentions, hashtags, cashtags and bot commands. The default build has no
`teloxide`, database, HTTP client, or moderation dependencies.

```rust
use teloxide_url::{find_urls, parse_telegram_link};

let links = find_urls("Посмотри docs.example.org/guide");
assert_eq!(links[0].value, "docs.example.org/guide");

let telegram = parse_telegram_link("https://t.me/c/123456/789");
```

Enable the `teloxide` feature to extract native UTF-16 Telegram entities,
hidden `text_link` targets and supported nested `RichMessage` URLs:

```toml
teloxide-url = { version = "0.1", default-features = false, features = ["teloxide"] }
```

`parse_telegram_link` parses syntax only. It never resolves usernames, invite
hashes or membership through the network. A `t.me/+digits` route is represented
as ambiguous because phone-number and invite meanings cannot be distinguished
from the URL alone. Usernames do not imply numeric user IDs.

The parser also recognizes the exact client compatibility form
`tg://emoji?id=<decimal u64>` as `TelegramLink::CustomEmojiFallback`. This is
only a typed URL candidate. With the `teloxide` adapter it is marked on
addressed URL entities and rich URL nodes, but the application must still
validate the complete visible label as one emoji grapheme and preserve ordinary
URL evidence. This format is client compatibility behavior, not a universal
Bot API guarantee; no network lookup or arbitrary URL exemption is performed.

Inputs, URL lengths, traversal depth and number of results are bounded. The
library does not fetch links, assess reputation, or prescribe moderation
actions. Telegram invite and bot deep-link parameters may contain sensitive
tokens; callers should avoid logging the raw value.
