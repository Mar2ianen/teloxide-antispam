# Unicode V3 preprocessing fuzz target

`unicode_v3_preprocess` feeds at most 1 MiB of arbitrary bytes through the
opt-in V3 model projection and checks observation counters against the source
or Unicode 18 NFKC expansion budget. The corpus includes U+FC60, whose single
source scalar expands to a space and two combining marks before the stacked-mark
counter is computed. Deterministic Unicode conformance and property tests remain the
oracle; fuzzing complements them by exploring combinations and malformed
UTF-8.

Compile without nightly instrumentation:

```sh
cargo check --manifest-path fuzz/Cargo.toml --locked
```

Run with cargo-fuzz and nightly Rust:

```sh
cargo fuzz run unicode_v3_preprocess -- -max_total_time=60
```

CI compiles the target in the `unicode-v3` matrix row. A bounded runtime smoke
run can use `-max_total_time=30 -max_len=1048576 -rss_limit_mb=2048 -timeout=10`.
