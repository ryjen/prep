# Prep 2 roadmap

Status: **proposed**

This roadmap orders implementation by architectural risk. The objective is to prove the trust boundaries and contracts before porting the historical feature surface.

## Phase 0 — Design acceptance

Deliverables:

- Prep 2 architecture accepted;
- Rust core ADR accepted;
- canonical monorepo ADR accepted;
- plugin protocol v1 request/result/capability model accepted;
- source identity and lockfile model accepted;
- security invariants accepted;
- implementation issues linked and ordered.

Exit criterion: implementation can begin without relying on historical implicit trust semantics.

## Phase 1 — Workspace and quality floor

Create the minimal Rust workspace and CI without implementing package resolution.

Deliverables:

- `prep-cli`, `prep-core`, `prep-manifest`, `prep-protocol`, `prep-store`, and test support only where justified;
- formatting and warnings-as-errors;
- Clippy;
- unit/integration test jobs;
- dependency advisory/license policy;
- CodeQL/static analysis where useful;
- fuzz harness wiring with at least one smoke target;
- Linux primary job and macOS compile/test coverage;
- documented local developer commands.

Exit criterion: a trivial typed request can pass CLI → core → synthetic plugin → validated result under CI.

## Phase 2 — Manifest, lockfile, graph

Deliverables:

- `prep.toml` schema and validation;
- semantic package/plugin identifier types;
- `prep.lock` schema/versioning;
- deterministic serialization;
- dependency graph construction and cycle detection;
- explicit lock/update workflow;
- historical `package.json` import experiment/compatibility fixture.

Exit criterion: a dependency graph can be declared, resolved to mock immutable identities, locked, reloaded, and compared deterministically.

## Phase 3 — Store and filesystem invariants

Deliverables:

- XDG-aware user cache/store paths;
- project-local state directory;
- core-owned temporary/staging directories;
- path-containment API;
- transaction model for publishing immutable results;
- per-package isolated prefixes;
- activation/environment composition;
- interrupted transaction recovery;
- GC metadata/leases sufficient to avoid deleting live results.

Exit criterion: synthetic build outputs can be staged, validated, committed atomically, activated together, and removed/GC'd without cross-package overwrite.

## Phase 4 — Protocol v1 and process host

Deliverables:

- NDJSON framing and schema validation;
- hello/version handshake;
- request/result/error/event types;
- capability declarations and policy evaluation;
- sanitized environment construction;
- timeouts, cancellation, output/frame limits;
- structured prompt/secret broker;
- synthetic adversarial plugins;
- protocol conformance harness.

Exit criterion: crashes, hangs, malformed output, protocol mismatch, excessive output, and cancellation all fail closed without corrupting persistent state.

## Phase 5 — Source resolution

Implement the minimum real source set.

### Git

- resolve mutable ref → exact commit;
- materialize exact locked commit;
- canonical remote handling;
- explicit submodule policy;
- offline/cache behavior.

### Archive

- HTTPS transport;
- required digest verification;
- bounded download/extraction;
- traversal/symlink/hardlink protections;
- archive-bomb limits.

Exit criterion: remote source materialization is immutable, verified, adversarially tested, and repeatable from the lockfile.

## Phase 6 — Build capabilities

Port useful behavior, not historical implementation structure.

Order:

1. CMake;
2. Ninja/Make;
3. Autotools;
4. additional systems only after real use cases require them.

Each plugin receives source/build/staging roots from the core and returns structured results. Build options are represented as data/argv, not interpolated shell source.

Exit criterion: a fixture project with a transitive native dependency resolves, builds, tests, installs to isolated prefixes, activates, and executes successfully.

## Phase 7 — Host dependency model

Deliverables:

- host dependency probe abstraction;
- explicit plan representation for host changes;
- opt-in apt/Homebrew providers;
- privilege/prompt policy;
- noninteractive behavior;
- strong tests proving normal dependency resolution cannot silently mutate the host.

Exit criterion: host package mutation is always attributable and policy-controlled.

## Phase 8 — Legacy transition

Deliverables:

- document which Prep 1 behaviors are supported, changed, or intentionally removed;
- remove historical `cli`/`web` submodules from the active repository layout;
- point archived/historical repositories to `ryjen/prep`;
- add migration/import guidance;
- decide whether `prep-web` remains archived/private historical material or has an independent future.

Exit criterion: `ryjen/prep` is self-contained and no Prep 2 build/test path depends on historical repositories.

## Phase 9 — Alpha readiness

Required before an alpha release:

- all security invariants implemented or explicitly marked deferred with no false security claim;
- end-to-end locked builds for representative CMake + Make/Ninja projects;
- Linux/macOS validation;
- fuzz regression corpus checked in where useful;
- threat model reviewed against implementation;
- SBOM/release provenance strategy;
- installation/update story;
- user documentation for manifest, lock, plugins, offline mode, and host policy.

## Deferred candidates

Do not pull these into the critical path without a concrete requirement:

- remote binary cache;
- signed third-party plugin registry;
- strong OS build sandboxing;
- Windows support;
- web UI;
- long-lived/multiplexed plugin daemons;
- distributed builds;
- generalized package registry.
