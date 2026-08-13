# Prep 2 security invariants

Status: **proposed**

This document defines security properties the implementation must preserve. They are design constraints, not optional hardening tasks.

## 1. Threat model

Prep processes data and code from multiple trust domains:

- project manifests and lockfiles;
- dependency metadata;
- remote Git repositories;
- downloaded archives;
- plugin manifests and plugin stdout;
- build-system files and build scripts;
- host environment and toolchain state;
- shared/local Prep store metadata.

Assume repository metadata, remote source content, archive entries, and plugin output can be malformed or malicious.

A plugin is executable code. A build is also code execution. Prep 2 does **not** claim that protocol validation alone makes malicious plugins or malicious build scripts safe. The core's job is to minimize ambient authority, make dangerous capabilities visible/policy-controlled, constrain Prep-owned state, and provide a path to stronger platform sandboxing.

## 2. Invariant: identifiers are not paths

Package names, plugin names, versions, source labels, and operation names are parsed as validated semantic identifiers.

No identifier may reach filesystem path construction before validation.

At minimum reject:

- empty strings;
- `/` and `\\` path separators where identifiers are expected;
- `.` or `..` traversal segments;
- NUL/control characters;
- absolute-path forms;
- drive/UNC/path-prefix ambiguity on supported platforms.

Use typed wrappers such as `PackageName` rather than passing raw `String` through core APIs where practical.

## 3. Invariant: filesystem roots are explicit and contained

Every operation that writes files receives an explicit root owned by the core.

For any derived path:

```text
normalize(candidate) must remain within normalize(root)
```

Containment checks must account for:

- `..` traversal;
- absolute paths;
- symlink traversal;
- hardlink targets;
- race-sensitive replacement where relevant;
- platform path prefixes.

A path that cannot be proven contained fails closed.

## 4. Invariant: archive extraction cannot escape staging

Archive extraction is a security boundary.

Reject or safely handle entries containing:

- absolute paths;
- parent traversal;
- symlinks/hardlinks whose resolved target escapes staging;
- device nodes and other special files not explicitly permitted;
- unsupported path encodings;
- path collisions that change entry type unexpectedly.

Extraction limits should include configurable bounds on:

- total expanded bytes;
- entry count;
- individual entry size;
- path length;
- nesting depth where library support permits it.

Digest verification occurs before extraction when the source type provides a digest.

## 5. Invariant: normal execution uses immutable source identity

A normal locked build never executes a moving branch/tag/URL without verification.

Git:

- human references resolve to an exact commit;
- checkout HEAD must match the locked commit;
- submodule identities, if enabled, are captured/verified rather than recursively accepted from mutable state.

Archive:

- lockfile records a cryptographic digest;
- downloaded bytes must match before extraction.

Local development paths are explicitly marked non-immutable and cannot silently populate globally reusable cache entries as if they were verified remote inputs.

## 6. Invariant: host mutation is denied by default

Resolving or building a dependency must not silently invoke `apt`, Homebrew, `sudo`, or another host package manager.

Host mutation requires:

1. an operation explicitly requesting it;
2. a plugin declaring `host.package_manager` and, if needed, `privilege`;
3. policy allowing it;
4. a visible plan or user approval unless noninteractive policy explicitly permits it.

The preferred default for host packages is **probe**, not mutate.

## 7. Invariant: plugin control traffic is data, never shell source

The core does not serialize values into shell declarations or `eval`-compatible strings.

Protocol frames are parsed as structured data with size and schema bounds.

When a plugin needs to invoke a subprocess, arguments remain an argv vector. Shell execution is not used merely to interpolate package metadata or build options.

If a specific build system deliberately requires a shell command, that is an explicit plugin behavior and its quoting/argument model is independently tested.

## 8. Invariant: environment inheritance is allowlisted/sanitized

Plugins and builds do not receive the entire caller environment by default.

The process runner constructs environment state intentionally, including dependency composition variables such as `PATH`, `CMAKE_PREFIX_PATH`, and `PKG_CONFIG_PATH`.

Special attention is required for variables that alter dynamic loading or tool execution, including platform equivalents of:

- `LD_PRELOAD`;
- `LD_LIBRARY_PATH`;
- `DYLD_*`;
- compiler wrapper variables;
- language/package-manager injection variables.

The exact allow/deny model belongs to process policy and may differ between a trusted development build and a hardened execution mode.

## 9. Invariant: store publication is transactional

A build plugin writes to staging, never directly to a published immutable result.

Publication follows:

```text
create staging → execute → validate → fsync/close as required → atomic commit → record metadata
```

If any step before commit fails, no valid result becomes visible.

A failed or interrupted build must not be mistaken for a cached success on the next run.

Published result directories are immutable through Prep APIs.

## 10. Invariant: package outputs never destructively overlay each other

Each package result receives an isolated prefix.

Activation composes prefixes; it does not copy/symlink package output into a single mutable shared tree where installation/removal order changes ownership.

Name collisions are represented and diagnosed rather than resolved by overwriting another package's files.

## 11. Invariant: plugin execution is bounded

Every plugin invocation has finite limits:

- timeout;
- maximum protocol frame size;
- maximum diagnostic/output volume;
- bounded prompt behavior;
- cancellation path;
- child-process reaping.

A plugin crash, hang, malformed response, or premature exit is a failure, never success.

## 12. Invariant: parsing failures do not terminate the process unexpectedly

Manifest, lockfile, protocol, and store metadata parsing returns typed errors.

Malformed external input must not:

- panic across a trust boundary;
- call `exit()` from a utility layer;
- leave partially mutated persistent state;
- be reinterpreted as defaults that broaden authority.

## 13. Invariant: policy decisions are attributable

For security-relevant actions, Prep should be able to explain:

- which plugin/capability was selected;
- which policy allowed or denied it;
- which immutable source identity was used;
- whether host mutation/privilege/network access occurred;
- which result identity was published.

The initial implementation can record this in structured logs/store metadata; a more general provenance/evidence format may follow later.

## 14. Invariant: secrets are not evidence

Secret prompt responses, tokens, passwords, and credentials must not be written to:

- stdout/stderr logs;
- lockfiles;
- store metadata;
- provenance records;
- crash diagnostics where avoidable.

Plugins requiring credentials should consume brokered secret input or a narrowly provided credential mechanism.

## 15. Invariant: offline means no network

When offline mode is enabled, a plugin operation declaring `network` is denied unless it can satisfy the operation from verified local state without performing network access.

The core should test this policy using synthetic plugins and, where possible, platform-level network isolation in hardened test jobs.

## 16. Security verification gates

Before a Prep 2 alpha is considered usable for real dependency builds, CI must include at minimum:

- warnings denied in Rust workspace code;
- Clippy;
- dependency advisory/license policy;
- ShellCheck for shell reference plugins;
- unit/integration/E2E tests;
- ASan/UBSan for any unsafe/native components introduced;
- fuzz smoke jobs for parsers/path/archive logic;
- archive traversal fixtures;
- malicious identifier/path fixtures;
- malformed/oversized protocol frames;
- plugin crash/hang/cancellation cases;
- interrupted store transaction recovery tests.

## 17. Security design review trigger

Changes require explicit security review when they introduce or expand:

- privileged execution;
- host package mutation;
- plugin installation/update from remote sources;
- new archive/filesystem extraction behavior;
- new network source types;
- shared writable global state;
- secret handling;
- OS sandbox bypass/escape hatches;
- remote binary caches or artifact signatures.
