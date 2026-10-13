# ADR 0002: Make `ryjen/prep` the canonical Prep 2 monorepo

Status: **accepted**

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

The historical `cli` and `web` submodules were retained only during design/bootstrap. They were removed from the active repository layout once the Rust workspace, protocol, store, migration fixture, and CI scaffolding were self-contained.

`ryjen/prep-plugins` is not imported wholesale. Individual useful plugin behaviors are reimplemented against protocol v1 with black-box fixtures where appropriate.

## Implementation status

As of 2026-10-12, `ryjen/prep` no longer requires the historical `cli` or `web` gitlinks. The canonical clone contains the active Rust workspace, protocol and security contracts, test support, fuzz targets, and migration fixtures. Historical repositories remain external references only.

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
- `prep-web` is currently a private, empty repository and is explicitly excluded from Prep 2. Do not import, publicize, or create a web frontend to satisfy monorepo structure. If a UI is justified after CLI/API stabilization, start as an in-tree `apps/web` component backed by stable machine-readable contracts; split only for an independent release/ownership boundary.

## Revisit when

Revisit repository extraction only after a component has an independently stable contract, release cadence, ownership boundary, or distribution need. Do not split merely to mirror runtime component boundaries.
