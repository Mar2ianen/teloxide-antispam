# teloxide-antispam

Independent Rust library. Do not introduce dependencies on a specific bot,
SQL schema, owner, Telegram chat, or LLM provider. Configuration, snapshot
loading, persistence, and Telegram enforcement belong to the consumer.

- Documentation, comments, errors, and commit messages must be in English.
  Non-English strings are allowed for text-processing rules and test inputs.
- Feature matrix: no-default, unicode, classifier, scoring, cas, default, all.
- Before committing: cargo fmt --check, cargo test --all-targets --all-features,
  cargo clippy --all-targets --all-features -- -D warnings, and the feature matrix.
- Never commit private corpora, moderation exports, credentials, or DSNs.
- A model score does not authorize bans. Unreviewed exports and account bans
  are not gold labels for individual messages.
- Preprocessing, model artifacts, and calibration must have explicit versions.
- New tests use synthetic inputs and must not reference neighboring repositories.
