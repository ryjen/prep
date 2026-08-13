# ADR 0005: No remote third-party plugin installation in protocol v1

Status: **proposed**

## Context

A plugin is executable code. The historical project bundled/defaulted plugins through repository/submodule/package mechanisms, but Prep 2's language-neutral protocol makes future independent plugin distribution desirable.

Remote plugin installation introduces a separate supply-chain problem from dependency source resolution: executable provenance, update policy, trust roots, rollback, signatures, and capability review.

That problem is not required to validate the core/plugin architecture.

## Decision

For Prep 2 v1:

- official/reference build plugins may be built and distributed with the Prep source/release;
- additional plugins may be configured/installed **explicitly from local filesystem content**;
- Prep records a content identity for the plugin package/executable + manifest used by an operation;
- plugin manifests and declared capabilities are validated before execution;
- there is **no `prep plugin install <url>` or automatic remote plugin update path**;
- no registry is trusted by default.

Remote third-party plugin installation is deferred until a dedicated provenance/install design is accepted.

## Minimum local plugin identity

The store/evidence model should be able to identify at least:

- plugin name;
- declared version;
- protocol version;
- manifest digest;
- executable/package content digest sufficient to distinguish code changes;
- declared capabilities.

A plugin changing bytes without changing its declared version therefore still changes its execution identity.

## Rationale

This lets Prep prove the external-process/plugin model without creating an unnecessary remote-code-installation surface during bootstrap.

It also makes plugin code changes attributable in build result metadata and avoids pretending semantic versions alone are a security identity.

## Consequences

- early third-party plugins require explicit local installation/configuration;
- reference plugins can still be written in any language;
- a future registry/install mechanism must define signature/provenance verification, update/rollback behavior, namespace ownership, and capability-policy UX before enabling remote code execution.

## Revisit when

Revisit only after protocol v1 and local plugin identity are stable and there is a real distribution requirement for independently hosted plugins.
