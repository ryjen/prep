# ADR 0004: Use a generated TOML lockfile and include toolchain identity in build results

Status: **proposed**

## Context

Two design questions affect cache correctness and reviewability:

1. how `prep.lock` is serialized;
2. which inputs determine whether an existing native build result can be reused.

For native code, `name + version` or even source digest alone is insufficient. Compiler version, target, dependency results, build options, build-system executables, and other ABI/code-generation inputs can materially change the output.

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
- normalized compiler/linker/toolchain identity;
- identities for output-affecting build executables actually selected, such as CMake, Ninja, Make, or Autotools components;
- build-system/plugin content identity and protocol version where it can affect output;
- normalized build configuration/options;
- relevant environment inputs explicitly modeled by Prep.

The initial toolchain fingerprint should include compiler identity/version and target information. Output-affecting host tools must also have a normalized identity sufficient for conservative local cache reuse. It need not claim perfect hermetic reproducibility, but it must prevent obviously unsafe reuse across materially different toolchains or build-tool versions.

ADR 0006 specifies the same-filesystem publication requirement and refines output-affecting executable identity.

## Rationale

TOML keeps lock changes human-reviewable alongside `prep.toml` while deterministic generation avoids treating formatting as state.

Including toolchain and build-tool identity is required for safe native caching. A result produced by Clang for one target, or by materially different CMake/Ninja versions, is not assumed interchangeable simply because source and plugin bytes match.

## Consequences

- cache misses may be more frequent until toolchain fingerprint normalization matures;
- correctness is preferred to aggressive reuse;
- environment variables not modeled as build inputs should not silently influence a supposedly reusable cached result;
- the result-identity implementation requires tests showing relevant compiler/target/build-tool/config changes alter the result key.

## Deferred

- bit-for-bit reproducibility guarantees;
- remote cache compatibility negotiation;
- full compiler/sysroot/content hashing;
- canonical cross-machine toolchain identities beyond what is necessary for safe v1 local reuse.
