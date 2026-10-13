# Prep 1 → Prep 2 migration

Prep 2 is a redesign, not an in-place upgrade of the historical C++ implementation.

The canonical source repository is now `ryjen/prep`. The historical `ryjen/prep-cli`, `ryjen/prep-plugins`, and `ryjen/prep-web` repositories are behavioral references only; a Prep 2 clone does not require them as submodules or build inputs.

## Migration model

Migration is intentionally one-way:

1. characterize useful Prep 1 behavior with fixtures or black-box tests;
2. translate declarations into Prep 2's explicit manifest and lock model;
3. reimplement only behavior that still fits the Prep 2 trust and isolation boundaries.

Do not preserve unsafe Prep 1 semantics merely for compatibility.

## Manifest mapping

| Prep 1 | Prep 2 | Notes |
| --- | --- | --- |
| `name` | `[package].name` | Identifier is validated. |
| `version` | `[package].version` | Version is validated. |
| `build_system` | `[build].systems` | Names identify declared build capabilities/plugins. |
| `build_options` | `[build].options` | Prep 2 uses explicit key/value options rather than blindly forwarding an arbitrary list. |
| dependency `name` / `version` | `[[dependencies]].name/version` | Duplicate names are rejected. |
| `git.location` | dependency source `kind = "git"` | Human refs may be declared, but the lockfile records an immutable commit identity. |
| `archive.location` | dependency source `kind = "archive"` | Prep 2 v1 requires HTTPS; the lockfile records SHA-256. |
| local source | dependency source `kind = "path"` | Local paths are intentionally not globally cacheable. |
| `apt` / `homebrew` fallback blocks | explicit host-provider plan | Host mutation is not an implicit dependency-resolution fallback. |
| plugin-specific arbitrary JSON | explicit protocol/config contract | Unknown manifest fields fail closed. |
| `executable` | no direct v1 manifest equivalent yet | Output/install metadata is handled by the build/result model rather than copied mechanically. |

A minimal Prep 2 declaration looks like:

```toml
schema = "prep/1"

[package]
name = "example"
version = "1.0.0"

[build]
systems = ["cmake", "ninja"]

[[dependencies]]
name = "example-lib"
version = "1.2.3"

[dependencies.source]
kind = "git"
url = "https://github.com/example/example-lib.git"
ref = "v1.2.3"
```

The declared Git ref is not the final identity. Resolution produces a checked-in `prep.lock` containing the exact commit and provider identity.

## Intentional compatibility breaks

Prep 2 deliberately does **not** preserve several Prep 1 behaviors:

- mutable Git refs are resolved and locked before build execution;
- HTTP archives without integrity are rejected rather than silently accepted;
- archive identity is content-addressed with SHA-256;
- host package-manager mutation is explicit, attributable, and policy-controlled;
- package installation uses isolated immutable result prefixes and transactional publication rather than a shared symlink overlay;
- plugin communication uses the versioned `prep.plugin/1` JSON protocol rather than the ad-hoc line/PTY protocol;
- plugin capabilities such as network access, prompting, privilege, or host mutation are explicit admission inputs;
- malformed or unknown declarations fail closed.

## Current importer status

The repository contains a historical `package.json` fixture and tests that classify migration outcomes. This is a compatibility characterization boundary, **not yet a promise of a production automatic importer**.

In particular, the fixture intentionally demonstrates legacy declarations that cannot be translated safely without user action (for example an HTTP archive with no digest and an `apt` fallback).

Until a user-facing importer is implemented, migrate manifests explicitly and review every trust-model change rather than performing a mechanical JSON-to-TOML conversion.

## Historical repositories

- `ryjen/prep-cli` — original C++ core/CLI
- `ryjen/prep-plugins` — original executable/script plugin set
- `ryjen/prep-web` — historical web component

Useful behavior can be ported into this monorepo behind Prep 2 contracts and tests. Historical implementation source should not be copied wholesale unless there is a concrete reason and its license/notices are preserved.
