# ADR 0006: Keep transactional publication on one filesystem and fingerprint output-affecting tools

Status: **proposed**

## Context

Two implementation details materially affect correctness even though they can look like storage or cache optimizations:

1. atomic publication is only reliable when the staging object and final store object can be committed using an atomic rename/link operation supported by the same filesystem;
2. native build outputs can change when the executable build tools change, even when the compiler, source, plugin wrapper, and arguments do not.

For example, a CMake plugin with identical bytes can invoke CMake 3.x or 4.x, and Ninja/Make/Autotools versions can alter generated or installed output. A cache key that fingerprints only the plugin and compiler can therefore reuse an incompatible result.

## Decision

### Transactional publication

Prep creates result staging directories **inside a transaction/staging area on the same filesystem as the destination store**.

Publication follows:

```text
allocate same-filesystem staging
→ execute build/install
→ validate tree and metadata
→ flush/close required state
→ atomically rename/commit to immutable result path
→ durably record publication metadata
```

If the configured store cannot provide the required atomic publication primitive, Prep fails closed rather than silently degrading to a cross-filesystem recursive copy that could expose partial results.

Temporary source/download work may live elsewhere when safe; **result publication staging may not**.

A destination result that already exists is never overwritten. The core verifies that an existing entry is a complete result matching the expected result identity or reports a store collision/corruption error.

### Output-affecting executable identity

`BuildInput` includes normalized identities for external executables that can materially affect the result, not only the compiler and Prep plugin bytes.

Examples include, as applicable:

- compiler/linker and target/sysroot identity;
- CMake;
- Ninja or Make;
- Autoconf/Automake/configure toolchain components when invoked;
- other build-system executables selected by a plugin.

For v1 local cache safety, an executable identity may be a normalized combination of resolved executable path/provider plus a stable version/probe result. Prep does not need to hash every system executable byte in milestone 1, but it must prefer a cache miss when it cannot establish sufficient identity.

The plugin handshake/manifest declares what tool probes it requires; the core records the resulting normalized tool identities as build inputs rather than trusting an opaque plugin version to stand in for host tool state.

## Rationale

Same-filesystem staging turns "atomic publication" into an implementable invariant rather than an aspiration. It also gives crash recovery a crisp boundary: uncommitted staging may be collected, while a committed store entry is complete.

Explicit build-tool identity follows the same conservative cache rule as compiler identity: correctness is preferred to an optimistic hit when material inputs differ or are unknown.

## Consequences

- `prep-store` owns allocation of result staging locations; plugins never choose final/store paths.
- store configuration must be validated for publication semantics.
- result identity may change when a host build tool is upgraded even when source code does not.
- plugin probe/conformance tests must cover required tool discovery and deterministic identity reporting.
- future hermetic tool bundles can replace host-tool probing without changing the conceptual `BuildInput` model.

## Deferred

- cross-filesystem transactional replication;
- remote/distributed store commit protocols;
- universal content hashing of complete host toolchains;
- bit-for-bit reproducibility guarantees.
