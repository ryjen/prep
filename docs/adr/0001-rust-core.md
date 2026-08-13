# ADR 0001: Use Rust for the Prep 2 core

Status: **proposed**

## Context

Prep is a native package/build orchestration tool that accepts external metadata, resolves network sources, extracts archives, constructs filesystem paths, launches build processes/plugins, manages persistent state, and may eventually broker privileged host operations.

The historical implementation used C++17. Its architecture remains useful, but the review found recurring classes of failure around unchecked integer status codes, manual filesystem/process handling, parsing boundaries, and shell/process coupling.

A redesign should choose a language that makes these invariants easier to express and continuously verify rather than merely port the old classes.

## Decision

Use **Rust 2024 edition** for the Prep 2 core and CLI.

Plugins remain language-neutral external processes and do not need to be implemented in Rust.

## Rationale

Rust is preferred because it provides:

- memory safety without a managed runtime;
- explicit `Result`/typed error handling;
- strong newtypes/enums for identifiers, paths, source identities, protocol states, and capabilities;
- RAII for files, temporary directories, child processes, and transaction state;
- mature serialization support for TOML/JSON schemas;
- strong fuzzing support through libFuzzer/cargo-fuzz;
- Clippy/rustfmt and straightforward warnings-as-errors quality gates;
- good cryptographic/hash and Git/archive ecosystem options;
- single-native-binary distribution consistent with Prep's original operational model.

## Alternatives considered

### Modern C++

Would preserve more implementation code and history, but the redesign would still need substantial custom discipline around lifetime, error, parser, and process safety. Because the intended architecture changes significantly, preserving source-level continuity has limited value.

### Go

A credible alternative with excellent implementation velocity, process/network ergonomics, and distribution. It is rejected mainly because Prep's core benefits from stronger domain modeling around state transitions, immutable identities, capability policy, and filesystem invariants. Go would still be a good plugin implementation language.

### Python

Excellent for plugins and rapid experiments, but not preferred for the long-lived native core because of runtime distribution and weaker enforcement of low-level process/filesystem invariants.

### Zig

Technically attractive for systems tooling but currently offers less ecosystem/tooling maturity for this project's dependency, protocol, fuzzing, and security-analysis needs.

## Consequences

- Prep 2 is a redesign, not a direct source port.
- Historical behavior must be preserved through characterization tests/fixtures where it remains desirable.
- Unsafe Rust requires explicit justification and review; the initial core should aim to use none.
- Rust-specific types must not leak into the external plugin protocol.
- The repository should use a workspace with only meaningful crate boundaries rather than one crate per conceptual component.

## Revisit when

Revisit only if an early spike demonstrates a concrete blocker in Rust around required platform process control, plugin interoperability, archive handling, or distribution. Familiarity or rewrite cost alone is not sufficient reason to reverse the decision.
