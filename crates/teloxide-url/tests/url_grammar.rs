use std::collections::BTreeSet;

use teloxide_url::{Found, Kind, MAX_ITEMS, Source, find_urls, parse_telegram_link, parse_url};

const SEED: u64 = 0x005e_ed20_2611_1603;
const VALID_CASES: usize = 2_048;
const V3_CASES: usize = 1_000;
const ORACLE_MAX_URL_BYTES: usize = 4_096;

#[derive(Clone, Copy)]
struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn choose(&mut self, upper: usize) -> usize {
        (self.next() as usize) % upper
    }
}

#[test]
fn canonicalization_matches_independent_component_oracles() {
    let cases = [
        (
            "HTTP://ExAmPlE.OrG:80/MiXeD?A=1&a=2#Frag",
            "http://example.org/MiXeD?A=1&a=2#Frag",
        ),
        (
            "https://EXAMPLE.org:443/Case?Key=Upper&key=lower",
            "https://example.org/Case?Key=Upper&key=lower",
        ),
        (
            "https://example.org:8443/A?x=1&x=2#Part",
            "https://example.org:8443/A?x=1&x=2#Part",
        ),
        ("https://example.org/path?", "https://example.org/path?"),
        (
            "https://example.org/escaped%2fsegment?q=%41&q=%2f",
            "https://example.org/escaped%2fsegment?q=%41&q=%2f",
        ),
        (
            "https://пример.example.org/Путь?Ключ=Значение",
            "https://xn--e1afmkfd.example.org/%D0%9F%D1%83%D1%82%D1%8C?%D0%9A%D0%BB%D1%8E%D1%87=%D0%97%D0%BD%D0%B0%D1%87%D0%B5%D0%BD%D0%B8%D0%B5",
        ),
        (
            "http://[2001:DB8::1]:80/MiXeD?A=1&a=2",
            "http://[2001:db8::1]/MiXeD?A=1&a=2",
        ),
        (
            "https://user:pass@example.org:443/Path",
            "https://user:pass@example.org/Path",
        ),
    ];

    for (raw, expected) in cases {
        let parsed = parse_url(raw).unwrap_or_else(|| panic!("V2 fixture rejected: {raw}"));
        assert_eq!(parsed.as_str(), expected, "raw={raw}");
        assert_eq!(
            parse_url(parsed.as_str()).as_ref(),
            Some(&parsed),
            "raw={raw}"
        );
    }

    let credentials = parse_url("https://t.me@evil.invalid/path").unwrap();
    assert_eq!(credentials.username(), "t.me");
    assert_eq!(credentials.host_str(), Some("evil.invalid"));
    assert_eq!(parse_telegram_link("https://t.me@evil.invalid/path"), None);
    assert_eq!(parse_telegram_link("https://t.me.evil.invalid/path"), None);
    assert_eq!(parse_telegram_link("https://not-t.me.invalid/path"), None);
    assert_eq!(
        parse_telegram_link("https://synthetic.xn--e1afmkfd.example.org/path"),
        None
    );
    assert!(matches!(
        parse_telegram_link("https://t.me/synthetic_user"),
        Some(teloxide_url::TelegramLink::Profile { username }) if username == "synthetic_user"
    ));

    for invalid in [
        "javascript://example.org/path",
        "data://example.org/path",
        "https://example.org:abc/path",
        "https://example.org:65536/path",
        "http://[2001:db8::1/path",
        "https://example.org/a b",
        "tg://",
    ] {
        assert!(
            parse_url(invalid).is_none(),
            "invalid V3 input accepted: {invalid}"
        );
    }

    let exact_limit = format!(
        "https://example.org/{}",
        "a".repeat(ORACLE_MAX_URL_BYTES - 20)
    );
    assert_eq!(exact_limit.len(), ORACLE_MAX_URL_BYTES);
    assert!(parse_url(&exact_limit).is_some());
    assert!(parse_url(&format!("{exact_limit}a")).is_none());
}

#[test]
fn seeded_valid_grammar_cases_are_idempotent_and_keep_case_sensitive_components() {
    let mut rng = XorShift64(SEED);
    let hosts = [
        ("example.org", "example.org"),
        ("MAIL.Example.org", "mail.example.org"),
        ("[2001:DB8::1]", "[2001:db8::1]"),
        ("XN--E1AFMKFD.XN--P1AI", "xn--e1afmkfd.xn--p1ai"),
        ("T.Me", "t.me"),
    ];
    let mut accepted = 0usize;
    for index in 0..VALID_CASES {
        let (raw_host, expected_host) = hosts[rng.choose(hosts.len())];
        let https = rng.next() & 1 == 0;
        let scheme = if https { "https" } else { "http" };
        let default_port = if https { 443 } else { 80 };
        let port = if rng.next() & 3 == 0 {
            default_port
        } else {
            8081
        };
        let path_case = if rng.next() & 1 == 0 {
            "MiXeD"
        } else {
            "mixed"
        };
        let query_case = if rng.next() & 1 == 0 { "Key" } else { "key" };
        let raw = format!(
            "{scheme}://{raw_host}:{port}/{path_case}/item{index}?{query_case}=A&{query_case}=a#Frag{index}"
        );
        let parsed = parse_url(&raw).unwrap_or_else(|| {
            panic!("seed={SEED:#x} index={index} generated valid URL was rejected")
        });
        accepted += 1;
        assert_eq!(parsed.scheme(), scheme, "seed={SEED:#x} index={index}");
        assert_eq!(
            parsed.host_str(),
            Some(expected_host),
            "seed={SEED:#x} index={index}"
        );
        assert_eq!(parsed.path(), format!("/{path_case}/item{index}"));
        assert_eq!(
            parsed.query(),
            Some(format!("{query_case}=A&{query_case}=a").as_str()),
            "seed={SEED:#x} index={index}"
        );
        assert_eq!(parsed.fragment(), Some(format!("Frag{index}").as_str()));
        assert_eq!(parsed.port(), (port != default_port).then_some(port));
        assert_eq!(parse_url(parsed.as_str()).as_ref(), Some(&parsed));
    }
    assert_eq!(accepted, VALID_CASES);
    println!("V2 seed={SEED:#x} accepted={accepted}/{VALID_CASES} rejected=0");
}

#[test]
fn byte_mutated_v3_inputs_are_rejected_with_bounded_results() {
    let mut rejected = 0usize;
    for index in 0..V3_CASES {
        let mutation = index % 4;
        let mut bytes = match mutation {
            0 => format!("https://example.org/path/{index}").into_bytes(),
            1 => format!("https://example.org:8080/path/{index}").into_bytes(),
            2 => format!("http://[2001:db8::1]/path/{index}").into_bytes(),
            _ => format!("https://example.org/path/{index}").into_bytes(),
        };
        match mutation {
            0 => bytes[0] = b'j',
            1 => {
                let port = bytes
                    .windows(5)
                    .position(|window| window == b":8080")
                    .unwrap()
                    + 1;
                bytes[port..port + 4].copy_from_slice(b"x080");
            }
            2 => {
                let close = bytes.iter().position(|b| *b == b']').unwrap();
                bytes.remove(close);
            }
            _ => {
                let dot = bytes.iter().position(|b| *b == b'.').unwrap();
                bytes[dot] = b':';
            }
        }
        let mutated = String::from_utf8(bytes).expect("mutations preserve UTF-8");
        assert!(
            parse_url(&mutated).is_none(),
            "seed={SEED:#x} index={index} mutation={mutation} should reject"
        );
        rejected += 1;
        let found = find_urls(&format!("prefix {mutated} suffix"));
        assert!(found.len() <= MAX_ITEMS);
        assert!(found.iter().all(|item| item.value.len() <= 4096));
    }
    println!("V3 seed={SEED:#x} rejected={rejected}/{V3_CASES} accepted=0");
    assert_eq!(rejected, V3_CASES);
}

#[test]
fn one_semantic_target_is_source_independent_and_distinct_case_targets_stay_distinct() {
    let target = "https://example.org/Case?A=1&a=2";
    let sources = [
        Source::VisibleText,
        Source::TelegramEntity,
        Source::RichMessage,
        Source::Qr,
    ];
    let mut identities = BTreeSet::new();
    let mut telegram_kinds = Vec::new();
    for source in sources {
        let mut found = Found::new(target, Kind::Url);
        found.source = source;
        let canonical = parse_url(&found.value).unwrap().to_string();
        assert_eq!(canonical, target);
        identities.insert(canonical.clone());
        telegram_kinds.push(parse_telegram_link(&canonical));
    }
    assert_eq!(identities.len(), 1);
    assert!(telegram_kinds.iter().all(|kind| *kind == telegram_kinds[0]));

    let telegram_target = "tg://resolve?domain=synthetic_bot&start=fixture";
    let mut telegram_identities = BTreeSet::new();
    let mut telegram_kinds = Vec::new();
    for source in sources {
        let mut found = Found::new(telegram_target, Kind::Url);
        found.source = source;
        let canonical = parse_url(&found.value).unwrap().to_string();
        telegram_identities.insert(canonical.clone());
        telegram_kinds.push(parse_telegram_link(&canonical));
    }
    assert_eq!(telegram_identities.len(), 1);
    assert!(matches!(
        telegram_kinds.first(),
        Some(Some(teloxide_url::TelegramLink::BotStart { .. }))
    ));
    assert!(telegram_kinds.iter().all(|kind| *kind == telegram_kinds[0]));

    identities.insert("https://example.org/case?A=1&a=2".to_owned());
    identities.insert("https://example.org/Case?a=1&A=2".to_owned());
    assert_eq!(identities.len(), 3);
}

#[test]
fn committed_public_corpus_uses_only_synthetic_hosts_and_explicit_validity() {
    let mut validity_counts = [0usize; 2];
    for line in include_str!("../../../testing/corpus/public/url_grammar_synthetic.jsonl").lines() {
        let case: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(case["schema_version"], 1);
        assert!(case["id"].as_str().unwrap().starts_with("grammar-"));
        assert_eq!(
            case["evidence_source"],
            if case["validity"] == "V2_synthetic" {
                "hand_authored_fixture"
            } else {
                "hand_authored_byte_mutation"
            }
        );
        assert_eq!(case["bot_api_version"], "not_applicable_url_syntax");
        assert_eq!(
            case["teloxide_revision"],
            "f05b90be40e9838e009281182abfcc3db2319c41"
        );

        let input = case["input"].as_str().unwrap();
        match case["validity"].as_str().unwrap() {
            "V2_synthetic" => {
                validity_counts[0] += 1;
                let canonical = parse_url(input).unwrap().to_string();
                assert_eq!(
                    Some(canonical.as_str()),
                    case["expected_canonical"].as_str()
                );
                assert_eq!(case["expected_class"], "web_url");
                let host = parse_url(input).unwrap().host_str().unwrap().to_owned();
                assert!(
                    host == "example.org"
                        || host.ends_with(".example.org")
                        || host.ends_with(".invalid"),
                    "public corpus host must be synthetic: {host}"
                );
                assert!(parse_telegram_link(input).is_none());
            }
            "V3_invalid" => {
                validity_counts[1] += 1;
                assert!(case["expected_canonical"].is_null());
                assert_eq!(case["expected_class"], "reject");
                assert!(
                    parse_url(input).is_none(),
                    "invalid fixture accepted: {input}"
                );
            }
            validity => panic!("unknown corpus validity class: {validity}"),
        }
    }
    assert_eq!(validity_counts, [5, 2]);
}
