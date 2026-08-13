# ADR 0002: Make `ryjen/prep` the canonical Prep 2 monorepo

Status: **proposed**

## Context

The historical project is split across `ryjen/prep`, `ryjen/prep-cli`, `ryjen/prep-plugins`, and `ryjen/prep-web`. The original `prep` repository mostly acts as a container with git submodules.

The CLI and plugin split represented a sound conceptual boundary, but the CLI already pinned an exact plugin repository commit as a submodule. During a redesign of the protocol, core invariants, store model, tests, and reference plugins, separate release repositories add coordination cost without providing meaningful independence.

## Decision

Use **`ryjen/prep` as the canonical Prep 2 source repository**.

During the redesign, keep core crates, protocol definitions, reference plugins, conformance tests, adversarial fixtures, and architecture documentation in one repository.

The historical repositories remain available as reference implementations and migration fixtures.

## Rationale

A monorepo provides atomic changes across:

- core protocol types;
- protocol documentation;
- reference plugin implementations;
- conformance tests;
- source/store contracts;
- end-to-end fixtures;
- CI and security gates.

That is particularly valuable before `prep.plugin/1` stabilizes.

The architectural boundary remains process-level and protocol-level; repository boundaries are not required to enforce it.

## Repository shape

```text
prep/
├── crates/
├── plugins/
├── tests/
├── docs/
└── ...
```

The existing `cli` and `web` submodules remain temporarily while the design/migration plan is being accepted. They should leave the active build graph once Prep 2 scaffolding exists and historical references are documented.

`ryjen/prep-plugins` is not imported wholesale. Individual useful plugin behaviors are reimplemented against protocol v1 with black-box fixtures where appropriate.

## Future plugin distribution

This ADR does not require all plugins to live in the monorepo forever.

Once protocol v1 and plugin provenance/install policy are stable:

- official/reference plugins may remain in-tree;
- third-party plugins may live anywhere;
- individual plugins may be released independently;
- the conformance suite remains the compatibility authority.

## Consequences

- Cross-component design changes use one PR and one CI pipeline.
- The repository becomes larger, but the implementation is easier to evolve coherently.
- Existing submodule-based build assumptions are deprecated.
- `prep-web` is not part of the Prep 2 critical path; any future UI should consume stable machine-readable CLI/core interfaces rather than shape core architecture.

## Revisit when

Revisit repository extraction only after a component has an independently stable contract, release cadence, ownership boundary, or distribution need. Do not split merely to mirror runtime component boundaries.
