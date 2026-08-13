# ADR 0004: Use a generated TOML lockfile and include toolchain identity in build results

Status: **proposed**

## Context

Two design questions affect cache correctness and reviewability:

1. how `prep.lock` is serialized;
2. which inputs determine whether an existing native build result can be reused.

For native code, `name + version` or even source digest alone is insufficient. Compiler version, target, dependency results, build options, and other ABI/code-generation inputs can materially change the output.

## Decision

### Lockfile

Use a **generated TOML `prep.lock`** with:

- an explicit schema version;
- deterministic/stable ordering;
- no semantic reliance on comments;
- immutable source identities;
- resolved dependency edges;
- resolver/provider identity where relevant.

The lockfile is checked into source control and intended to be reviewable. Normal builds do not rewrite it. Resolution/update commands perform explicit lockfile changes.

### Build result identity

A reusable build result identity must include at least:

- immutable source identity;
- ordered dependency result identities;
- target platform/triple;
- normalized toolchain identity;
- build-system/plugin identity and protocol version where it can affect output;
- normalized build configuration/options;
- relevant environment inputs explicitly modeled by Prep.

The initial toolchain fingerprint should include compiler identity/version and target information. It need not claim perfect hermetic reproducibility, but it must prevent obviously unsafe reuse across materially different toolchains.

## Rationale

TOML keeps lock changes human-reviewable alongside `prep.toml` while deterministic generation avoids treating formatting as state.

Including toolchain identity is required for safe native caching. A result produced by Clang 20 for one target is not assumed interchangeable with a result produced by GCC or another target simply because the source commit matches.

## Consequences

- cache misses may be more frequent until toolchain fingerprint normalization matures;
- correctness is preferred to aggressive reuse;
- environment variables not modeled as build inputs should not silently influence a supposedly reusable cached result;
- the result-identity implementation requires tests showing relevant compiler/target/config changes alter the result key.

## Deferred

- bit-for-bit reproducibility guarantees;
- remote cache compatibility negotiation;
- full compiler/sysroot content hashing;
- canonical cross-machine toolchain identities beyond what is necessary for safe v1 local reuse.
