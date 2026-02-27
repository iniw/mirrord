# AGENTS.md

Context for AI agents working in `mirrord/agent` (`mirrord-agent`).

## Scope

This file covers:
- `mirrord/agent/src` (main agent binary and runtime/task orchestration)
- `mirrord/agent/env` (`mirrord-agent-env`, typed env contract shared with operator)
- `mirrord/agent/iptables` (`mirrord-agent-iptables`, iptables backends/chains/rule lifecycle)

## Quick Reference

```bash
# Linux-only crate
cargo check -p mirrord-agent --target x86_64-unknown-linux-gnu --keep-going

# Supporting sub-crates used by mirrord-agent
cargo check -p mirrord-agent-env --keep-going
cargo check -p mirrord-agent-iptables --target x86_64-unknown-linux-gnu --keep-going

# Focused tests that run on host (if platform supports them)
cargo test -p mirrord-agent
```

## Key Paths

- Entrypoint and lifecycle: `src/main.rs`, `src/entrypoint.rs`, `src/entrypoint/setup.rs`
- Client transport + optional TLS: `src/client_connection.rs`
- Incoming traffic redirect/mirror/steal: `src/incoming/**`, `src/mirror.rs`, `src/steal/**`
- Outgoing TCP/UDP: `src/outgoing.rs`, `src/outgoing/udp.rs`, `src/outgoing/socket_stream.rs`
- DNS + reverse DNS: `src/dns.rs`, `src/reverse_dns.rs`
- Remote filesystem operations: `src/file.rs`, `src/util/path_resolver.rs`
- Target discovery/container runtime: `src/runtime.rs`, `src/runtime/crio.rs`, `src/container_handle.rs`
- Background task runtime + namespace entry: `src/task.rs`, `src/task/status.rs`, `src/namespace.rs`
- Metrics endpoint and counters: `src/metrics.rs`
- Agent env schema: `env/src/envs.rs`, `env/src/steal_tls.rs`
- Iptables composition: `iptables/src/lib.rs`, `iptables/src/mesh/**`, `iptables/src/prerouting.rs`

## Architecture

### Process Model (targeted vs targetless)

- The binary is Linux-only (`src/main.rs`).
- `entrypoint::main` chooses one of two modes:
1. `start_agent(args)` for `targetless` mode or already-marked child process.
2. `start_iptable_guard(args)` for targeted mode parent process.
- In targeted mode, parent/child split is mandatory:
1. Parent spawns child with `MIRRORD_AGENT_CHILD_PROCESS=true`.
2. Child runs normal agent logic.
3. Parent waits for child exit or SIGTERM, then cleans iptables chains.
- Dirty iptables detection happens before serving clients. If rules already exist:
1. Either fail fast (`AgentError::IPTablesDirty`) and notify first client with `DaemonMessage::Close`.
2. Or clean leftovers when `clean_iptables_on_start` is enabled.

### Runtime Topology and Namespace Rules

- Main agent loop runs on current-thread tokio runtime (`#[tokio::main(flavor = "current_thread")]`).
- Long-lived network-facing background tasks run on a dedicated `BgTaskRuntime` thread.
- `BgTaskRuntime` may enter target net namespace before creating tokio runtime (`setns`).
- For targeted non-ephemeral mode, network runtime is created in target net namespace.
- For targetless and ephemeral mode, network runtime stays in current namespace.
- This split exists to keep agent shutdown deterministic if a background task hangs.

### Connection and Message Pipeline

- `ClientConnection` wraps TCP stream in `actix_codec::Framed<DaemonCodec>`.
- Optional TLS on this link is controlled by operator-provided cert (`AGENT_OPERATOR_CERT_ENV`), enforced by `AgentTlsConnector`.
- One `ClientConnectionHandler` per connected layer client.
- Each handler owns:
1. `FileManager`
2. Optional `TcpMirrorApi`
3. Optional `TcpStealerApi`
4. `TcpOutgoingApi` and `UdpOutgoingApi`
5. `DnsApi` and `ReverseDnsApi`
- `ClientConnectionHandler::start` is a `tokio::select!` fan-in over:
1. incoming `ClientMessage`
2. mirror/steal events
3. outgoing TCP/UDP task responses
4. DNS/reverse DNS responses
5. cancellation token

### Incoming Traffic Stack (mirror + steal)

- Redirecting is centralized in one `RedirectorTask<R: PortRedirector>` (`incoming/task.rs`).
- `MirrorHandle` and `StealHandle` are thin client APIs over a shared redirector task.
- Default redirector is `IpTablesRedirector` composed as IPv4/IPv6 `ComposedRedirector`.
- New redirected TCP connections are classified via `MaybeHttp::detect`:
1. Optional TLS termination/re-origination per port via `StealTlsHandlerStore`.
2. HTTP version detection (`http::detect_http_version`) with timeout.
- Per-port state tracks:
1. one stealer subscriber (`steal_tx`)
2. multiple mirror subscribers (`mirror_txs`)
3. active IO tasks (`JoinSet`)
- Unsubscribed-but-still-active race is handled by passthrough until active IO drains, then redirect is removed.

### Steal Semantics

- `TcpStealerTask` is shared across clients.
- `PortSubscriptions` enforces clash rules:
1. only one unfiltered owner per port
2. multiple filtered owners per port
- `TcpStealerTask` dispatches stolen traffic according to:
1. subscription type (filtered/unfiltered)
2. protocol version capabilities of each client
3. HTTP filter matching (including optional body buffering)
- `TcpStealerApi` adapts responses to client protocol version:
1. legacy HTTP request messages
2. framed/chunked variants
3. mode-agnostic transport metadata for TLS-aware paths

### Mirror Semantics

- `TcpMirrorApi` owns port subscriptions and in-flight mirrored streams.
- Supports filtered HTTP mirror (`LayerTcp::PortSubscribeFilteredHttp`).
- For filters requiring body inspection, request body is buffered before final decision.
- Connection/request IDs are generated per client API instance.

### Outgoing TCP/UDP

- Outgoing is per-client and isolated:
1. `TcpOutgoingTask` handles connect/read/write/close for TCP + Unix domain sockets.
2. `UdpOutgoingTask` does same for UDP.
- Both run on `BgTaskRuntime`, so target namespace routing applies when configured.
- TCP connect path uses `SocketStream::connect`, with:
1. normal IP connect
2. Unix socket path resolution via `/proc/<pid>/root/...` for target filesystem sockets
- Throughput is back-pressured with semaphore-based throttling (`Throttled` + permits).

### DNS and Reverse DNS

- `DnsWorker` is global per agent instance; each client has a `DnsApi`.
- Worker reads target `/etc/resolv.conf` and `/etc/hosts` (`/proc/<pid>/root/etc/...` when targeted).
- Every lookup builds resolver from current files (no persistent resolver cache).
- API preserves request/response ordering per client using `FuturesOrdered`.
- Reverse DNS uses per-request `spawn_blocking(dns_lookup::lookup_addr)` in network runtime.

### File Operations

- `FileManager` is synchronous (in handler thread) and handles all `FileRequest` variants.
- File path access is target-aware via `InTargetPathResolver`:
1. prefixes paths with `/proc/<pid>/root`
2. prevents escaping root through parent traversal/symlink resolution
- Manager owns remote FD tables for files/dirs/getdents streams and updates `OPEN_FD_COUNT`.
- In targetless mode, no resolver is used and operations hit agent container filesystem directly.

### Target and Environment Discovery

- Container runtime adapters in `runtime.rs`:
1. Docker
2. containerd (multiple socket path probing)
3. cri-o
4. ephemeral mode fallback
- `ContainerHandle` caches target PID + raw env.
- Agent env map is merged from:
1. runtime inspection env
2. `/proc/<pid>/environ` (or `/proc/self/environ`)
- `GetEnvVarsRequest` filtering is wildcard-based with built-in denylist.

### Iptables Layer (`mirrord-agent-iptables`)

- `SafeIpTables` abstracts redirect backend and guarantees cleanup path.
- Redirect strategy is dynamically chosen:
1. standard chain
2. mesh-specific chain composition (Istio/Linkerd/Kuma/Ambient/CNI)
3. prerouting fallback when standard chain is unavailable
- Optional wrappers:
1. connection flush-on-redirect updates
2. mesh exclusion chain for agent port
- Chain names are static (`MIRRORD_INPUT`, `MIRRORD_OUTPUT`, `MIRRORD_STANDARD`, `MIRRORD_EXCLUDE_FROM_MESH`) and therefore conflict-sensitive.

## Operational Constraints

- Do not change the startup readiness line format lightly (`println!("agent ready - version ...")`); kube-side startup waits depend on it.
- Avoid blocking operations on agent runtimes; both main and background runtimes are single-threaded.
- Avoid noisy warn/error logs for client-level failures; see `mirrord/agent/CONTRIBUTING.md` tracing guidance.
- Keep protocol compatibility in mind; many paths branch on `ClientProtocolVersion`.
- Targetless mode intentionally disables incoming mirror/steal; do not assume these APIs always exist.

## Change Workflows

### Adding/Changing a `ClientMessage` in agent

1. Update handler in `ClientConnectionHandler::handle_client_message` (`src/entrypoint.rs`).
2. Update corresponding API/task (`file`, `dns`, `mirror`, `steal`, `outgoing`, etc.).
3. Ensure `DaemonMessage` response path is wired through `respond`.
4. Run `cargo check -p mirrord-agent --target x86_64-unknown-linux-gnu --keep-going`.

### Incoming Traffic Changes

1. Update `incoming/task.rs` state transitions and cleanup invariants.
2. Update `steal/*` or `mirror.rs` protocol adapters.
3. Verify protocol-version guards for new behavior.
4. Validate iptables lifecycle and cleanup.

### Iptables Changes

1. Update `mirrord/agent/iptables` composition.
2. Ensure dirty-rule detection in `entrypoint.rs` still matches actual chain names.
3. Keep parent/child guard cleanup behavior intact.
