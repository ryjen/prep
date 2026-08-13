# Development

Prep 2 uses the pinned Rust toolchain described by `rust-toolchain.toml`.

## Local quality checks

Run the same deterministic checks as CI:

```sh
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets --all-features
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
```

Dependency license, advisory, duplicate, wildcard, and source policy is enforced with pinned `cargo-deny`:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo deny --locked check
```

The root `Cargo.lock` is committed. Normal build/test/policy workflows must not silently resolve a different dependency graph.

The bounded fuzz smoke target requires the pinned nightly toolchain and `cargo-fuzz` version used by CI:

```sh
rustup toolchain install nightly-2026-08-13
cargo +nightly-2026-08-13 install cargo-fuzz --version 0.13.2 --locked
cargo +nightly-2026-08-13 fuzz run protocol_frame -- -max_total_time=10
```

## Bootstrap protocol smoke path

Issue #3 includes one deliberately internal command used to prove the process boundary before real build plugins are implemented:

```sh
cargo build --locked --workspace
cargo run --locked -p prep-cli -- internal probe-plugin ./target/debug/prep-synthetic-plugin
```

The path exercises:

```text
prep CLI → prep-core → external process → prep.plugin/1 NDJSON → validated result
```

The command is not a stable user-facing interface. It exists as a characterization/conformance seam and may move into dedicated test tooling as the plugin host from issue #6 takes shape.

## Bootstrap scope

The bootstrap intentionally does not add manifest, dependency graph, store, Git/archive, or real build-plugin implementation. Those belong to issues #4 onward after their invariants are represented by dedicated types and tests.
