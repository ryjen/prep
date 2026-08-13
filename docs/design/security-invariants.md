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

A plugin is executable code. A build is also code execution. Prep 2 does **not** claim that protocol validation or capability declarations make malicious plugins or malicious build scripts safe.

The v1 core must:

- minimize ambient authority it intentionally grants;
- make dangerous capabilities visible and policy-controlled;
- validate all data crossing into Prep-owned state;
- preserve the integrity of Prep-owned caches/stores even when a plugin fails;
- provide a path to stronger platform sandboxing.

Without an OS sandbox, a malicious plugin or build process can attempt any action available to the invoking user. Documentation and tests must distinguish admission/policy controls from true process containment.

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

## 3. Invariant: Prep-owned filesystem roots are explicit and contained

Every **core-owned** filesystem write operates relative to an explicit root.

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

Plugins receive explicit source/build/staging roots and conforming plugins are required to use them. The core never publishes plugin output outside validated staging roots. Strong prevention of arbitrary plugin writes elsewhere on the host requires platform sandboxing and is not falsely claimed by v1.

## 4. Invariant: archive extraction cannot escape staging

Archive extraction is a trusted-core security boundary in v1.

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

Digest verification occurs before extraction.

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

Git/archive begin as built-in Rust providers under ADR 0003 so the v1 immutable-source trust boundary does not depend on assertions from arbitrary executable plugins.

## 6. Invariant: Prep never authorizes host mutation by default

Resolving or building a dependency must not cause Prep to intentionally invoke `apt`, Homebrew, `sudo`, or another host package manager as an invisible fallback.

Host mutation requires:

1. an operation explicitly requesting it;
2. a plugin declaring `host.package_manager` and, if needed, `privilege`;
3. policy allowing it;
4. a visible plan or user approval unless noninteractive policy explicitly permits it.

The preferred default for host packages is **probe**, not mutate.

This is an authorization invariant. Without OS containment, malicious executable code may still attempt host mutation independently of Prep. Prep must not describe capability declarations as a sandbox.

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

Environment sanitization reduces accidental/ambient influence; it is not process containment.

## 9. Invariant: store publication is transactional

A build plugin writes intended outputs to staging, never directly to a published immutable result through Prep APIs.

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

## 11. Invariant: plugin execution is bounded from Prep's perspective

Every plugin invocation has finite limits:

- timeout;
- maximum protocol frame size;
- maximum diagnostic/output volume;
- bounded prompt behavior;
- cancellation path;
- immediate-child and process-tree cleanup strategy;
- child-process reaping.

A plugin crash, hang, malformed response, or premature exit is a failure, never success.

Platform-specific tests must verify that timeout/cancellation do not leave ordinary plugin child processes running indefinitely. A malicious process deliberately escaping its process group/job boundary is a sandbox-hardening problem and must not be hidden by stronger claims than the implementation supports.

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
- which plugin content identity affected a build;
- whether Prep authorized host mutation/privilege/network-dependent behavior;
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

## 15. Invariant: offline mode does not authorize network-dependent work

When offline mode is enabled, Prep refuses to schedule an operation that declares/requires network access unless the operation can be satisfied from verified local state without network use.

Built-in Git/archive providers must obey this directly.

For external plugins, this is an admission-policy guarantee. **Strong proof that a malicious plugin cannot access the network requires platform network isolation.** Hardened runners should add that enforcement where available, but v1 must not describe policy alone as a network sandbox.

## 16. Invariant: plugin distribution does not silently become remote code execution

Under ADR 0005, protocol v1 permits official/bundled or explicitly installed local plugins. Prep records content identity for the plugin code/manifest used.

There is no automatic remote plugin installation or update path in v1.

Adding one requires a separate security design covering provenance/signatures, trust roots, namespace ownership, update/rollback behavior, and capability review.

## 17. Security verification gates

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
- plugin crash/hang/cancellation/process-tree cases;
- interrupted store transaction recovery tests;
- tests that distinguish admission-policy behavior from any platform sandbox enforcement.

## 18. Security design review trigger

Changes require explicit security review when they introduce or expand:

- privileged execution;
- host package mutation;
- plugin installation/update from remote sources;
- new archive/filesystem extraction behavior;
- external source-provider plugins;
- new network source types;
- shared writable global state;
- secret handling;
- OS sandbox bypass/escape hatches;
- remote binary caches or artifact signatures.
