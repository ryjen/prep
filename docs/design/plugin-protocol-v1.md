# Plugin Protocol v1

Status: **proposed**

## 1. Objective

Prep plugins are external processes that provide narrowly scoped resolver, builder, tester, installer/prober, or related capabilities without linking into the Prep process.

The protocol must remain implementable in Bash, Python, Rust, Go, or another language, but it must not depend on shell serialization, terminal behavior, positional line parsing, or undocumented exit-code conventions.

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

The plugin responds:

```json
{
  "protocol":"prep.plugin/1",
  "id":"1",
  "type":"hello_result",
  "plugin":{
    "name":"git",
    "version":"0.1.0",
    "capabilities":["network","process.spawn","filesystem.write.staging"]
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

Example resolve request:

```json
{
  "protocol":"prep.plugin/1",
  "id":"42",
  "type":"resolve",
  "source":{
    "kind":"git",
    "url":"https://github.com/fmtlib/fmt",
    "ref":"11.2.0"
  },
  "context":{
    "staging_dir":"/tmp/prep/source-...",
    "offline":false
  }
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
    "canonical_url":"https://github.com/fmtlib/fmt",
    "revision":"0123456789abcdef..."
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
    "code":"resolution_failed",
    "message":"tag 11.2.0 was not found",
    "retryable":false
  }
}
```

Process exit status is not the domain result. A plugin process that crashes or exits before returning a valid terminal result produces a `plugin_process_failed` error in the core.

## 5. Operation families

Protocol v1 should support these operation families without requiring every plugin to implement every operation.

### Resolver

- `probe_source`
- `resolve`
- `materialize`

`resolve` converts a human/mutable declaration into an immutable identity. `materialize` writes the exact locked source into a core-provided staging directory.

### Builder

- `probe_build_system`
- `configure`
- `build`
- `test`
- `install_to_staging`

The exact split may be collapsed by a simple plugin, but Prep's model distinguishes phases so evidence and failures remain attributable.

### Host dependency probe/provider

- `probe_host_dependency`
- `plan_host_change`
- `apply_host_change`

`apply_host_change` requires explicit policy approval and a declared `host.package_manager` capability. Prep must never use host mutation as an invisible fallback from normal source resolution.

## 6. Capability declarations

Capabilities are machine-readable declarations used for policy and auditability.

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

A request cannot exercise a capability the plugin did not declare. A declared capability can still be denied by policy.

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
- secrets are not included in the ordinary environment unless explicitly required;
- Prep may remove dangerous or irrelevant inherited variables;
- dependency environment composition is core-owned.

## 8. Events and user interaction

Long-running operations may emit events:

```json
{"protocol":"prep.plugin/1","id":"42","type":"event","event":"progress","message":"cloning","completed":20,"total":100}
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

The core must prevent secret prompt values from entering logs, evidence records, or diagnostic streams.

## 9. Process lifecycle

The core owns the process lifecycle.

For every invocation:

1. select plugin and evaluate policy;
2. create bounded working/staging directories;
3. construct sanitized environment;
4. spawn process;
5. complete protocol handshake;
6. send one operation request;
7. read validated events/result frames;
8. enforce timeout, output limits, and cancellation;
9. terminate/reap process;
10. validate resulting filesystem state before committing anything.

Defaults should be finite. No plugin invocation waits forever.

On cancellation or timeout, Prep first requests graceful shutdown if the protocol state permits it, then escalates to process termination after a bounded grace period.

## 10. Plugin manifest

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

Plugin installation/provenance policy is deliberately separate from protocol framing and must be defined before remote third-party plugin installation is enabled.

## 11. Error vocabulary

Protocol errors should be stable enough for orchestration without becoming an enormous errno clone.

Initial domain codes:

```text
invalid_request
unsupported_operation
unsupported_source
unsupported_build_system
not_found
unavailable
resolution_failed
integrity_failed
build_failed
test_failed
install_failed
policy_denied
cancelled
timeout
internal
```

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
- A plugin writes only to core-created staging locations permitted by the operation.
- Successful plugin completion does not imply successful store commit; core validation follows.
- The protocol itself does not claim to sandbox malicious executable code.
- Host mutation and privilege require explicit capability and policy decisions.
- Network access can be disabled by an offline policy.
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
- stdout remains protocol-clean;
- secret prompts are not echoed;
- filesystem effects stay inside provided staging roots;
- plugin exit before terminal result is treated as failure.

Synthetic adversarial plugins belong in `prep-test-support` so core process handling can be tested without relying on real Git/CMake/etc.

## 14. Deferred from v1

- arbitrary binary payload framing;
- multiplexing many concurrent requests through one long-lived plugin process;
- remote plugins;
- OS-specific strong sandbox negotiation;
- direct PTY passthrough;
- plugin-to-plugin communication.

A process-per-operation model is intentionally acceptable for v1 because it gives simple lifecycle and failure isolation.
