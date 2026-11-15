use proptest::{
    prelude::*,
    test_runner::{Config, RngSeed, TestRunner},
};
use serde::Serialize;
use teloxide_url::parse_url;

const DEFAULT_SEED: u64 = 0x01_02_03_04_05_06_07_08;
const CASES: u32 = 256;

#[derive(Serialize)]
struct CaseReport {
    case_id: &'static str,
    invariant_id: &'static str,
    validity_class: &'static str,
    origin: &'static str,
    seed: u64,
    generated: u32,
    accepted: u32,
    rejected: u32,
}

#[test]
fn generated_http_url_components_remain_valid_and_round_trip() {
    let seed = std::env::var("TELOXIDE_URL_PROPTEST_SEED")
        .map(|value| {
            value
                .parse::<u64>()
                .expect("TELOXIDE_URL_PROPTEST_SEED must be an unsigned integer")
        })
        .unwrap_or(DEFAULT_SEED);
    let strategy = ("[a-z]{1,12}", "[A-Za-z0-9_-]{1,18}", "[A-Za-z0-9]{1,18}");
    let mut runner = TestRunner::new(Config {
        cases: CASES,
        rng_seed: RngSeed::Fixed(seed),
        failure_persistence: None,
        ..Config::default()
    });
    runner
        .run(&strategy, |(host, path, query)| {
            let raw = format!("https://{host}.example.org/{path}?ref={query}");
            let parsed = parse_url(&raw)
                .ok_or_else(|| TestCaseError::fail("generated valid URL was rejected"))?;
            let independent = url::Url::parse(&raw)
                .map_err(|_| TestCaseError::fail("reference URL parser rejected input"))?;
            let expected_host = format!("{host}.example.org");
            let expected_path = format!("/{path}");
            let expected_query = format!("ref={query}");
            prop_assert_eq!(parsed.host_str(), Some(expected_host.as_str()));
            prop_assert_eq!(parsed.path(), expected_path.as_str());
            prop_assert_eq!(parsed.query(), Some(expected_query.as_str()));
            prop_assert_eq!(parsed.as_str(), independent.as_str());
            prop_assert_eq!(parse_url(parsed.as_str()).unwrap(), parsed);
            Ok(())
        })
        .unwrap_or_else(|error| panic!("seeded URL property failed: {error}"));

    let report = CaseReport {
        case_id: "generated-http-url-components",
        invariant_id: "URL-01",
        validity_class: "v2",
        origin: "property_generated",
        seed,
        generated: CASES,
        accepted: CASES,
        rejected: 0,
    };
    assert_eq!(report.generated, report.accepted + report.rejected);
    assert!(!report.case_id.is_empty());
    assert_eq!(report.invariant_id, "URL-01");
    assert_eq!(report.validity_class, "v2");
    assert_eq!(report.origin, "property_generated");
    assert!(report.seed > 0);
    let serialized = serde_json::to_value(report).unwrap();
    assert_eq!(serialized["generated"], CASES);
}
