# AGENTS.md

Context for AI agents working on the mirrord project.

## Scope

This file is the top-level orchestrator for mirrord as a project. It'll tell you about:
- How major components fit together end-to-end;
- How responsibilities are split between components;
- Cross-component workflows (protocol, routing, compatibility).

## Multi-Repo Layout

mirrord is a multi-repo project. In addition to this repository, related sibling repositories are:
- Operator repository: `../operator`
- Operator installation charts: `../charts`
- Documentation: `../docs`
- Configuration-specific documentation: `../docs-configuration`

## Quick Reference

```bash
# Core checks

# Layer
cargo check -p mirrord-layer --keep-going
# Intproxy
cargo check -p mirrord-intproxy --keep-going
# Agent (Linux-only)
cargo check -p mirrord-agent --target x86_64-unknown-linux-gnu --keep-going
# CLI
cargo check -p mirrord --keep-going

# Integration tests
cargo test -p mirrord-layer

# Always format after edits
cargo fmt
```

Use `cargo check -p <crate> --keep-going` to surface all match failures quickly.

## Project Overview

mirrord is a tool that lets developers run local processes in the context of their cloud environment.

### Component Topology

```
┌─────────────────────────────────────────────────────────────────────────┐
│  LOCAL MACHINE                                                          │
│  ┌──────────────────────┐     ┌──────────────────────┐                  │
│  │   User Application   │     │      CLI (mirrord)   │                  │
│  │  ┌────────────────┐  │     │  - Resolves target   │                  │
│  │  │     Layer      │  │     │  - Starts intproxy   │                  │
│  │  │ (LD/DYLD hook) │◄─┼─────┤  - Provides config   │                  │
│  │  └───────┬────────┘  │     └──────────────────────┘                  │
│  └──────────┼───────────┘                                               │
│             │ local protocol (TCP/Unix)                                 │
│  ┌──────────▼───────────┐                                               │
│  │      Intproxy        │  Multi-layer routing + one agent session      │
│  └──────────┬───────────┘                                               │
└─────────────┼───────────────────────────────────────────────────────────┘
              │ agent protocol over port-forward/operator tunnel
┌─────────────┼───────────────────────────────────────────────────────────┐
│  KUBERNETES │ CLUSTER                                                   │
│  ┌──────────▼───────────┐                                               │
│  │        Agent         │  Executes remote fs/network/dns/env ops       │
│  │   (target context)   │  and manages steal/mirror plumbing            │
│  └──────────────────────┘                                               │
└─────────────────────────────────────────────────────────────────────────┘
```

### Responsibility Split

- `mirrord-layer`: syscall/API interception inside user process; local fd/socket state and client-level protocol behavior.
- `mirrord-intproxy`: local orchestration hub; request/response matching; reconnect/failover; message fanout.
- `mirrord-agent`: remote execution backend in cluster context; network namespace-sensitive operations; remote file-system manipulation.
- `mirrord-protocol`: shared wire messages (`ClientMessage`, `DaemonMessage`) across layer/intproxy/agent.
- `mirrord-config`: validated feature flags and targeting configuration fed into CLI/layer/agent startup.

### Data/Control Flow

1. CLI resolves target and connectivity mode (Operator or direct Kubernetes).
2. CLI starts intproxy and injects the layer (`LD_PRELOAD`/`DYLD_INSERT_LIBRARIES`).
3. Layer initializes hooks and opens a session with intproxy.
4. intproxy connects remotely using one of two paths:
   - Direct Kubernetes: intproxy reaches the agent via port-forward/direct tunnel.
   - Operator: intproxy joins an operator-managed session and routes through operator-provided connectivity.
5. Hooked operations become protocol requests (`ClientMessage`) via intproxy.
6. Agent executes in target context and sends responses/events (`DaemonMessage`).
7. Layer translates those results back into expected libc/system call behavior.

## Configuration Orchestration

`LayerConfig` (`mirrord/config/src/lib.rs`) is the cross-component contract for:
- target selection;
- env/fs/network feature flags;
- agent runtime options;
- experimental toggles that gate behavior in multiple crates.

Config precedence is CLI args > env vars > file.

## Global Development Rules

- Keep imports at file top (no function-local `use`).
- Prefer `to_owned` for `&str` -> `String`.
- Always use `foo.rs` instead of `foo/mod.rs` for module roots.
- Avoid redundant inline comments; prefer meaningful docs on complex behavior.
- Run `cargo fmt` after edits.

## Crate Map

- `mirrord-layer`: Process-injected interception layer.
- `mirrord-agent`: Remote execution backend in cluster.
- `mirrord-intproxy`: Local routing/orchestration proxy.
- `mirrord-intproxy-protocol`: Layer <-> intproxy local protocol.
- `mirrord-protocol`: Shared layer/intproxy/agent protocol.
- `mirrord-cli`: User-facing command entrypoint.
- `mirrord-config`: Configuration parsing/validation.
- `mirrord-kube`: Kubernetes API integration.
- `mirrord-operator`: CRD types used by operator flow.
