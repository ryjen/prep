# ADR 0003: Implement Git and archive source providers in the trusted core first

Status: **proposed**

## Context

Prep's original architecture treated Git and archive resolution as ordinary executable plugins. In Prep 2, source resolution sits directly on the immutable-input trust boundary:

- Git must resolve mutable human references to exact commits;
- archive bytes must be digest-verified;
- archive extraction must enforce filesystem containment and resource limits;
- source-cache/offline behavior must preserve the same identity semantics.

The external plugin protocol can represent resolver operations, but making the first Git/archive implementation a third-party-style executable would force the core either to trust plugin assertions about source identity or independently reimplement much of the verification logic anyway.

## Decision

Implement **Git and archive as built-in Rust source providers for Prep 2 v1**, behind a narrow internal `SourceProvider`-style interface.

Keep resolver operations in `prep.plugin/1` as an extensibility point for future source types, but do not make external resolver plugins necessary to bootstrap or enforce the v1 trust model.

## Core ownership

The core/source subsystem owns:

- canonical source identity;
- immutable Git commit verification;
- archive digest verification;
- bounded archive extraction and path containment;
- source-cache keys and offline semantics;
- submodule policy/identity;
- transition from resolved → verified → materialized source.

External source plugins, when enabled later, must return data that the core can validate before a source becomes trusted/published.

## Rationale

This keeps the smallest security-critical source types inside the memory-safe, fuzzed core while retaining plugin extensibility where it is valuable.

It also reduces the v1 bootstrap dependency cycle: Prep does not need a working plugin installation/provenance system merely to securely fetch its first dependency.

## Consequences

- `plugins/git` and `plugins/archive` are not required as external v1 runtime components.
- Historical Git/archive plugin behavior remains useful as characterization input only.
- Build-system plugins remain external process plugins and continue proving the language-neutral architecture.
- A future external source plugin API must not be able to bypass core integrity/containment validation.

## Revisit when

Revisit after protocol v1, plugin provenance, and source verification contracts are stable enough that an external provider can be treated as an untrusted mechanism whose output is independently validated by the core.
