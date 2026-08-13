# Prep

Prep is being redesigned as a small, security-conscious package and build orchestration tool for native projects.

The original Prep implementation proved the central idea: a compact core can own package state and orchestration while language-neutral plugins provide source resolution and build capabilities. The current design phase keeps that idea, but replaces implicit trust, mutable source resolution, ad-hoc process protocols, and filesystem overlays with explicit identities, policy, isolation boundaries, and transactional state.

## Status

**Prep 2 is in design.** The historical `cli` and `web` submodules remain in this repository as reference material during the design phase; they are not the target architecture.

The proposed implementation uses:

- Rust 2024 for the core and CLI;
- a language-neutral, versioned JSON plugin protocol;
- immutable source identities and a checked-in lockfile;
- isolated per-package install prefixes rather than a shared symlink overlay;
- fail-closed validation and typed errors;
- explicit capability declarations for plugins and host mutation;
- unit, integration, end-to-end, adversarial, property, and fuzz testing.

## Design documents

- [`docs/design/prep-2.md`](docs/design/prep-2.md) — system architecture and primary design
- [`docs/design/plugin-protocol-v1.md`](docs/design/plugin-protocol-v1.md) — language-neutral plugin contract
- [`docs/design/security-invariants.md`](docs/design/security-invariants.md) — threat model and non-negotiable invariants
- [`docs/adr/0001-rust-core.md`](docs/adr/0001-rust-core.md) — Rust core decision
- [`docs/adr/0002-canonical-monorepo.md`](docs/adr/0002-canonical-monorepo.md) — canonical repository decision
- [`docs/roadmap.md`](docs/roadmap.md) — staged implementation plan

## Design principles

1. **Immutable inputs before execution.** A human-friendly source reference is resolved to a stable identity before build execution.
2. **Fail closed.** Invalid metadata, ambiguous state, integrity failures, protocol violations, and incomplete transactions are errors.
3. **Contain filesystem effects.** Package identifiers never become unchecked paths; extraction and installation remain inside explicit roots.
4. **Separate mechanism from policy.** Resolvers and builders provide capabilities; the core decides whether and how they may run.
5. **No host mutation by default.** System package managers and privileged operations require explicit capability and policy decisions.
6. **Small core, explicit contracts.** Prefer typed data and narrow interfaces over hidden shared state or shell conventions.
7. **Test hostile boundaries.** Parsers, paths, archives, process control, lockfiles, and plugin frames are first-class fuzz and negative-test targets.

## Historical implementations

- `ryjen/prep-cli` — original C++ core/CLI
- `ryjen/prep-plugins` — original executable Bash plugins
- `ryjen/prep-web` — historical web component

These repositories remain useful as behavioral references and migration fixtures, but Prep 2 will be developed from the new contracts rather than by translating the C++ implementation class-for-class.
