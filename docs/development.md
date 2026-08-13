# Development

Prep 2 currently requires the stable Rust toolchain described by `rust-toolchain.toml`.

## Local quality checks

Run the same deterministic checks as CI:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
python3 scripts/check-licenses.py
```

Dependency advisory checking requires `cargo-audit`:

```sh
cargo install cargo-audit --locked
cargo audit
```

The bounded fuzz smoke target requires nightly Rust and `cargo-fuzz`:

```sh
cargo +nightly install cargo-fuzz --locked
cargo +nightly fuzz run protocol_frame -- -max_total_time=10
```

## Bootstrap protocol smoke path

Issue #3 includes one deliberately internal command used to prove the process boundary before real build plugins are implemented:

```sh
cargo build --workspace
cargo run -p prep-cli -- internal probe-plugin ./target/debug/prep-synthetic-plugin
```

The path exercises:

```text
prep CLI → prep-core → external process → prep.plugin/1 NDJSON → validated result
```

The command is not a stable user-facing interface. It exists as a characterization/conformance seam and may move into dedicated test tooling as the plugin host from issue #6 takes shape.

## Scope discipline

The bootstrap intentionally does not add manifest, dependency graph, store, Git/archive, or real build-plugin implementation. Those belong to issues #4 onward after their invariants are represented by dedicated types and tests.
