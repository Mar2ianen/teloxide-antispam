# Unicode V3 preprocessing fuzz target

`unicode_v3_preprocess` feeds bounded arbitrary bytes through the opt-in V3
model projection and checks that it remains total and produces bounded output.
The deterministic Unicode conformance and property tests remain the oracle;
fuzzing complements them by exploring combinations and malformed UTF-8.

Compile without nightly instrumentation:

```sh
cargo check --manifest-path fuzz/Cargo.toml --locked
```

Run with cargo-fuzz and nightly Rust:

```sh
cargo fuzz run unicode_v3_preprocess -- -max_total_time=60
```
