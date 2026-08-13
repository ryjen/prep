# Plugin Protocol v1

Status: **proposed**

## 1. Objective

Prep plugins are external processes that provide narrowly scoped build, test, host-provider, or future extension capabilities without linking into the Prep process.

The protocol must remain implementable in Bash, Python, Rust, Go, or another language, but it must not depend on shell serialization, terminal behavior, positional line parsing, or undocumented exit-code conventions.

Git and archive source resolution are **not external plugin requirements in v1**. They begin as built-in Rust source providers because immutable source identity, digest verification, and archive containment are part of the trusted bootstrap boundary. External source-provider operations are deferred until those validation contracts and plugin provenance are mature.

## 2. Transport

Protocol v1 uses **newline-delimited JSON (NDJSON)** over stdin/stdout.

- stdin: Prep → plugin protocol frames;
- stdout: plugin → Prep protocol frames only;
- stderr: human-readable diagnostics captured by Prep;
- UTF-8 only;
- one compact JSON object per line;
- configurable maximum frame size, defaulting to a conservative value such as 1 MiB;
- invalid JSON, oversized frames, unknown required fields, or invalid state transitions are protocol errors.

Plugins must not write arbitrary user output to stdout. That keeps the control channel unambiguous and fuzzable.

PTY execution is not part of v1. User interaction is brokered by the core through structured prompt events.

## 3. Version handshake

Prep starts a plugin and sends:

```json
{"protocol":"prep.plugin/1","id":"1","type":"hello","prep_version":"2.0.0-dev"}
```

A CMake plugin may respond:

```json
{
  "protocol":"prep.plugin/1",
  "id":"1",
  "type":"hello_result",
  "plugin":{
    "name":"cmake",
    "version":"0.1.0",
    "operations":["probe_build_system","configure"],
    "capabilities":["process.spawn","filesystem.read.source","filesystem.write.staging"]
  }
}
```

A protocol major-version mismatch terminates execution before any operational request is sent.

## 4. Request model

Every request contains:

- `protocol` — exact protocol identifier;
- `id` — caller-generated request identifier;
- `type` — request kind;
- `context` — bounded execution context where applicable;
- operation-specific fields.

Example configure request:

```json
{
  "protocol":"prep.plugin/1",
  "id":"42",
  "type":"configure",
  "package":{
    "name":"hello",
    "version":"1.0.0"
  },
  "context":{
    "source_dir":"/tmp/prep/source-...",
    "build_dir":"/tmp/prep/build-...",
    "staging_dir":"/tmp/prep/staging-...",
    "dependency_prefixes":["/home/user/.local/share/prep/store/..."],
    "environment":{
      "CMAKE_PREFIX_PATH":"..."
    }
  },
  "arguments":["-DCMAKE_BUILD_TYPE=Release"]
}
```

Successful response:

```json
{
  "protocol":"prep.plugin/1",
  "id":"42",
  "type":"result",
  "status":"ok",
  "value":{
    "configured":true
  }
}
```

Failure response:

```json
{
  "protocol":"prep.plugin/1",
  "id":"42",
  "type":"result",
  "status":"error",
  "error":{
    "code":"build_failed",
    "message":"cmake configuration failed",
    "retryable":false
  }
}
```

Process exit status is not the domain result. A plugin process that crashes or exits before returning a valid terminal result produces a `plugin_process_failed` error in the core.

## 5. Operation families

Protocol v1 supports only operation families required by the first implementation. Unused future extension points are not part of the conformance contract.

### Builder

- `probe_build_system`
- `configure`
- `build`
- `test`
- `install_to_staging`

A simple plugin does not have to implement every phase. Its manifest/handshake declares the operations it actually supports. Prep's model distinguishes phases so evidence and failures remain attributable.

### Host dependency probe/provider

- `probe_host_dependency`
- `plan_host_change`
- `apply_host_change`

`apply_host_change` requires explicit policy approval and a declared `host.package_manager` capability. Prep must never use host mutation as an invisible fallback from normal source resolution.

### Deferred source-provider extension

External `resolve`/`materialize` operations are deliberately outside the required v1 surface. Git/archive are built-in first under ADR 0003. A later protocol extension may add source providers only if the core can independently validate their identity/integrity outputs before use.

## 6. Capability declarations

Capabilities are machine-readable declarations used for **admission policy, visibility, and auditability**.

Initial vocabulary:

```text
network
process.spawn
filesystem.read.source
filesystem.write.staging
host.package_manager
privilege
prompt
prompt.secret
```

Capabilities should be declarative and coarse enough to remain stable. Platform-specific sandbox permissions may later refine them.

Prep refuses to authorize/request an operation whose required capabilities were not declared or are denied by policy.

**Capability declarations are not, by themselves, an OS sandbox.** Without platform containment, a malicious executable plugin may attempt actions beyond its declaration. Prep v1 must not claim otherwise. Where a hardened runner can enforce filesystem/network/process restrictions, those controls strengthen the same policy model; otherwise capability policy is admission control plus attribution.

## 7. Execution context

Prep constructs the execution context and does not blindly inherit the caller's environment.

The context may include:

```json
{
  "source_dir":"...",
  "build_dir":"...",
  "staging_dir":"...",
  "dependency_prefixes":["..."],
  "environment":{
    "PATH":"...",
    "CMAKE_PREFIX_PATH":"..."
  },
  "offline":false
}
```

Rules:

- paths are absolute and generated/validated by the core;
- plugins must not infer Prep repository/store paths;
- reference/conforming plugins write outputs only to the roots assigned to the operation;
- Prep never treats a plugin-returned arbitrary path as permission to publish or mutate store state;
- secrets are not included in the ordinary environment unless explicitly required;
- Prep may remove dangerous or irrelevant inherited variables;
- dependency environment composition is core-owned.

Absent a platform sandbox, the core can constrain what it *authorizes and publishes*, not guarantee that malicious plugin code cannot touch other host resources available to its OS user.

## 8. Events and user interaction

Long-running operations may emit events:

```json
{"protocol":"prep.plugin/1","id":"42","type":"event","event":"progress","message":"building","completed":20,"total":100}
```

User prompts are structured events:

```json
{
  "protocol":"prep.plugin/1",
  "id":"42",
  "type":"event",
  "event":"prompt",
  "prompt_id":"p1",
  "message":"Password required",
  "secret":true
}
```

Prep decides whether prompting is permitted, obtains the value without echo when appropriate, and responds:

```json
{"protocol":"prep.plugin/1","id":"42","type":"prompt_response","prompt_id":"p1","value":"..."}
```

The core must prevent secret prompt values from entering logs, evidence records, lockfiles, or diagnostic streams.

## 9. Process lifecycle

The core owns the process lifecycle.

For every invocation:

1. select plugin and evaluate policy;
2. verify the local plugin identity/manifest;
3. create working/build/staging directories;
4. construct sanitized environment;
5. spawn process;
6. complete protocol handshake;
7. send one operation request;
8. read validated events/result frames;
9. enforce timeout, output limits, and cancellation;
10. terminate/reap process;
11. validate Prep-owned resulting state before committing anything.

Defaults are finite. No plugin invocation waits forever.

On cancellation or timeout, Prep first requests graceful shutdown if the protocol state permits it, then escalates to process termination after a bounded grace period.

Process-tree cleanup requires platform-aware tests; killing only the immediate plugin process is insufficient if it leaves spawned children running.

## 10. Plugin manifest and identity

A plugin distribution includes metadata separate from the runtime handshake, conceptually:

```toml
schema = "prep.plugin/1"
name = "cmake"
version = "0.1.0"
executable = "prep-plugin-cmake"
operations = ["probe_build_system", "configure"]
capabilities = ["process.spawn", "filesystem.read.source", "filesystem.write.staging"]
```

The manifest is validated before execution. `executable` is resolved within the installed plugin package and cannot escape it through relative path traversal.

Under ADR 0005, protocol v1 supports official/bundled or explicitly installed local plugins only. Prep records content identity so changed executable/plugin bytes remain distinguishable even when the declared semantic version is unchanged.

Remote third-party installation and automatic update are outside v1.

## 11. Error vocabulary

Protocol errors should be stable enough for orchestration without becoming an enormous errno clone.

Initial domain codes:

```text
invalid_request
unsupported_operation
unsupported_build_system
not_found
unavailable
build_failed
test_failed
install_failed
policy_denied
cancelled
timeout
internal
```

Host-provider operations may additionally use a stable host-operation error code where needed rather than exposing provider-specific exit codes directly.

The core wraps process/protocol failures separately:

```text
plugin_spawn_failed
plugin_handshake_failed
plugin_protocol_violation
plugin_process_failed
plugin_output_limit_exceeded
```

Messages are diagnostic; orchestration uses codes and typed core errors.

## 12. Security rules

- Plugin stdout is untrusted structured input and is schema/size validated.
- Paths returned by plugins are never accepted as authoritative store paths.
- Conforming plugins are required to use core-provided operation roots; Prep only publishes validated staging content.
- Successful plugin completion does not imply successful store commit; core validation follows.
- The protocol itself does not claim to sandbox malicious executable code.
- Host mutation and privilege require explicit capability and policy decisions before Prep authorizes the operation.
- Offline policy prevents Prep from authorizing declared network-dependent operations; strong no-network enforcement requires a platform sandbox and must not be falsely claimed otherwise.
- Environment values and command arguments are data, not shell source.

## 13. Testing contract

A conformance suite should be usable against any plugin implementation.

It should verify:

- handshake/version behavior;
- every declared operation has valid request/result semantics;
- unknown operations fail cleanly;
- malformed frames do not crash the plugin/core;
- frame/output limits are enforced;
- timeout and cancellation work;
- spawned-child cleanup is exercised;
- stdout remains protocol-clean;
- secret prompts are not echoed;
- response paths outside assigned roots are rejected;
- reference plugins confine intended outputs to assigned roots;
- plugin exit before terminal result is treated as failure.

Where CI provides a sandbox, additional tests should prove declared filesystem/network restrictions at the OS layer. Those tests are platform hardening, not a prerequisite for claiming protocol conformance.

Synthetic adversarial plugins belong in `prep-test-support` so core process handling can be tested without relying on real CMake/Make/etc.

## 14. Deferred from v1

- external source-provider operations;
- arbitrary binary payload framing;
- multiplexing many concurrent requests through one long-lived plugin process;
- remote plugin installation/registry/update;
- OS-specific strong sandbox negotiation;
- direct PTY passthrough;
- plugin-to-plugin communication.

A process-per-operation model is intentionally acceptable for v1 because it gives simple lifecycle and failure isolation.
