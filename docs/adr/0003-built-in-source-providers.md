# ADR 0003: Implement Git and archive source providers in the trusted core first

Status: **proposed**

## Context

Prep's original architecture treated Git and archive resolution as ordinary executable plugins. In Prep 2, source resolution sits directly on the immutable-input trust boundary:

- Git must resolve mutable human references to exact commits;
- archive bytes must be digest-verified;
- archive extraction must enforce filesystem containment and resource limits;
- source-cache/offline behavior must preserve the same identity semantics.

Making the first Git/archive implementation a third-party-style executable would force the core either to trust plugin assertions about source identity or independently reimplement much of the verification logic anyway.

## Decision

Implement **Git and archive as built-in Rust source providers for Prep 2 v1**, behind a narrow internal `SourceProvider`-style interface.

External source-provider operations are **not part of the required `prep.plugin/1` v1 surface**. The architecture preserves the possibility of a later protocol extension for new source types once the core can independently validate provider output and plugin provenance is mature.

## Core ownership

The core/source subsystem owns:

- canonical source identity;
- immutable Git commit verification;
- archive digest verification;
- bounded archive extraction and path containment;
- source-cache keys and offline semantics;
- submodule policy/identity;
- transition from resolved → verified → materialized source.

Any future external source provider must return data/materialized state that the core can validate before a source becomes trusted or eligible for build execution.

## Rationale

This keeps the smallest security-critical source types inside the memory-safe, fuzzed core while retaining a future extensibility path where it is valuable.

It also reduces the v1 bootstrap dependency cycle: Prep does not need a working plugin installation/provenance system merely to securely fetch its first dependency.

Deferring unused source-plugin operations also follows economy of mechanism: protocol v1 should contain only operations exercised by the initial implementation and conformance suite.

## Consequences

- `plugins/git` and `plugins/archive` are not required as external v1 runtime components.
- Historical Git/archive plugin behavior remains useful as characterization input only.
- Build-system plugins remain external process plugins and continue proving the language-neutral architecture.
- The initial protocol conformance suite does not need resolver operations.
- A future external source-provider API must not bypass core identity, integrity, or containment validation.

## Revisit when

Revisit after protocol v1, local plugin provenance, and source verification contracts are stable enough that an external provider can be treated as an untrusted mechanism whose output is independently validated by the core, and a real new source type justifies expanding the protocol.
