# AGENTS.md

Context for AI agents working in `mirrord/intproxy` (`mirrord-intproxy` and `mirrord-intproxy-protocol`).

## Scope

This file covers:
- `mirrord/intproxy/src` (internal proxy runtime, task system, routing, reconnect/failover)
- `mirrord/intproxy/protocol` (layer <-> intproxy protocol and codecs)

## Quick Reference

```bash
# Main crate
cargo check -p mirrord-intproxy --keep-going
cargo test -p mirrord-intproxy

# Local protocol crate used between layer and intproxy
cargo check -p mirrord-intproxy-protocol --keep-going
```

## Key Paths

- Main event loop and routing: `src/lib.rs`
- Main task IDs/messages: `src/main_tasks.rs`
- Background task infrastructure: `src/background_tasks.rs`
- Layer connection lifecycle: `src/layer_initializer.rs`, `src/layer_conn.rs`
- Agent connection and reconnect policy: `src/agent_conn.rs`
- Ping health-check flow: `src/ping_pong.rs`
- Failover mode implementation: `src/failover_strategy.rs`
- Per-feature proxies: `src/proxies/{files,incoming,outgoing,simple}.rs`
- Shared request/resource helpers: `src/request_queue.rs`, `src/remote_resources.rs`
- Local protocol definitions: `protocol/src/lib.rs`
- Local protocol codec (sync/async): `protocol/src/codec.rs`, `protocol/src/codec/codec_async.rs`

## Architecture

### Runtime Model

- One `IntProxy` serves multiple layer processes (`LayerConnection` tasks) and one agent connection (`AgentConnection` task).
- Feature logic is split into dedicated background tasks:
1. `FilesProxy`
2. `IncomingProxy`
3. `OutgoingProxy`
4. `SimpleProxy` (DNS + env)
5. `PingPong`
- `LayerInitializer` accepts new layer sockets and performs `NewSession` handshake.

### Intproxy and Operator

- The CLI resolves connectivity before starting intproxy:
1. It tries operator flow first (`create_and_connect` in `mirrord/cli/src/connection.rs`).
2. If operator session creation succeeds, intproxy receives `AgentConnectInfo::Operator`.
3. If operator is unavailable/disabled/unlicensed, flow falls back to OSS and intproxy receives `AgentConnectInfo::DirectKubernetes`.
- In operator mode, intproxy does not create agents itself. It joins an existing operator-managed session via `OperatorApi::connect_in_existing_session` (`src/agent_conn.rs`).
- In OSS mode, intproxy talks directly to the agent through Kubernetes port-forward.
- Reconnect behavior differs by connect mode:
1. Operator sessions may be reconnectable (`allow_reconnect`), so `AgentConnection` can restart and emit `ConnectionRefresh`.
2. Direct Kubernetes and external proxy modes are non-reconnectable in `AgentConnection` (`ReconnectFlow::Break`).
- Current implementation detail: `mirrord exec` still spawns intproxy in both operator and OSS flows; operator changes how intproxy reaches and manages the remote session, not whether intproxy exists.

### Startup Gating and Protocol Negotiation

- On startup, intproxy immediately sends `ClientMessage::SwitchProtocolVersion`.
- `LayerInitializer` messages are suspended until `DaemonMessage::SwitchProtocolVersionResponse` is received.
- This prevents handling layer requests before version-dependent behavior is known.
- After negotiation, protocol version is broadcast to `FilesProxy`, `IncomingProxy`, `OutgoingProxy`, and `SimpleProxy`.

### Message Pipeline

1. Layer sends `LocalMessage<LayerToProxyMessage>` over the local TCP channel.
2. `LayerConnection` converts that into `ProxyMessage::FromLayer`.
3. `IntProxy` routes to feature proxy (`FilesProxy`, `IncomingProxy`, `OutgoingProxy`, `SimpleProxy`).
4. Feature proxy may send `ClientMessage` to the agent via `MessageBus::send_agent`.
5. `AgentConnection` receives `DaemonMessage` and emits `ProxyMessage::FromAgent`.
6. `IntProxy` routes response/events back to the correct feature proxy or directly to layer.
7. Responses to layer are sent as `ProxyMessage::ToLayer`.

### Request/Response Matching Strategy

- Layer messages carry `MessageId`; agent messages usually do not.
- Feature proxies use FIFO `RequestQueue` instances keyed by request type/protocol to recover `(MessageId, LayerId)` for responses.
- This relies on agent-side ordering guarantees per request stream (for example sequential file operations in legacy paths).
- New response paths must preserve this invariant or add explicit correlation IDs (as done by outgoing V2 connect `Uid`).

### Core IntProxy State

- `pending_layers: HashSet<(LayerId, MessageId)>` tracks in-flight requests that should eventually receive a layer response.
- `reconnect_task_queue: Option<VecDeque<ProxyMessage>>` buffers new work while reconnect is in progress.
- `connected_layers: HashMap<LayerId, ProcessInfo>` is used for periodic liveness logging.
- `agent_tx` is replaceable and gets refreshed after reconnect.

## Reconnect, Health, and Failover

### PingPong and Reconnect Triggering

- `PingPong` sends periodic `ClientMessage::Ping`.
- If no pong and no other agent traffic is observed in the interval, it emits `ConnectionRefresh::Request`.
- `AgentConnection` can also request reconnect on channel failure or explicit `RequestReconnect`.

### ConnectionRefresh Flow

- `ConnectionRefresh::Start`:
1. Broadcast to all feature proxies and ping-pong task.
2. Initialize `reconnect_task_queue`.
3. Feature proxies flush pending requests with deterministic "agent lost" responses and clear transient state.
- `ConnectionRefresh::End(new_tx)`:
1. Broadcast new tx handle to tasks.
2. Replace `IntProxy.agent_tx`.
3. Re-negotiate protocol version with the new connection.
4. Replay queued messages in order.

### Failover Mode

- Any critical `ProxyRuntimeError` switches `IntProxy` to `FailoverStrategy`.
- In failover:
1. Pending requests are answered with `ProxyToLayerMessage::ProxyFailed(...)`.
2. New layer connections are still accepted.
3. Most messages are ignored except those requiring immediate failure responses.
- This preserves debuggability and avoids hanging layer requests after terminal proxy failure.

## Feature Proxies

### `SimpleProxy` (`src/proxies/simple.rs`)

- Handles DNS (`GetAddrInfo`) and env (`GetEnvVars`) only.
- Keeps separate request queues for DNS and env.
- Uses protocol gating for `GetAddrInfoRequestV2`; falls back to v1 when needed.
- Optional hard-fail behavior on DNS permission denied (`dns_permission_error_fatal`).

### `FilesProxy` (`src/proxies/files.rs`)

- Handles all file operations and file-specific version compatibility.
- Tracks remote file/dir descriptors with `RemoteResources<u64>` so forked layers can share them safely.
- Maintains local buffering layers:
1. Directory buffering via `ReadDirBatch` (when protocol supports it).
2. Read-only file buffering via `ReadLimited` and local `fd_position`.
- Reconnect safety is handled by `RouterFileOps`:
1. Tracks highest user-facing fd.
2. Applies fd offsets after agent loss so stale fds cannot target unrelated new-agent descriptors.
3. Flushes synthetic error responses for outstanding operations.

### `OutgoingProxy` (`src/proxies/outgoing.rs`)

- Manages intercepted outgoing TCP/UDP and per-connection `Interceptor` tasks.
- Connect flows:
1. Legacy sequential connect responses use `RequestQueue`.
2. V2 connect uses `(Uid, NetProtocol)` map for parallel in-flight connects.
- Generates proxy-local outgoing IDs (`u128`) before agent responds, so layer can proceed immediately.
- Optional experimental non-blocking TCP connect path:
1. Uses `BusyTcpListener` hacks to emulate `EINPROGRESS`-like behavior.
2. Unblocks local connect only after agent confirms remote connect.
- Maintains connection ownership per layer with `RemoteResources<u128>` for correct fork/close semantics.

### `IncomingProxy` (`src/proxies/incoming.rs`)

- Handles mirrored/stolen incoming traffic and port subscription lifecycle.
- Delegates per-unit work to internal tasks:
1. `TcpProxyTask` for full mirrored/stolen TCP connections.
2. `HttpGatewayTask` for per-request HTTP forwarding/retries/response serialization.
- `SubscriptionsManager` resolves complex cases:
1. Multiple subscribes on same port.
2. Layer fork inheritance.
3. Delayed agent confirmation/rejection.
- `MetadataStore` maps local `(listener, peer)` pairs to remote source/local metadata for `ConnMetadata`.
- HTTP response format (`Basic`/`Framed`/`Chunked`) is negotiated from protocol version.
- On reconnect, subscriptions are restored after protocol re-negotiation (`restore_subscriptions_on_protocol_version_switch`).

## Internal Task Framework

- All long-running units implement `BackgroundTask`.
- `BackgroundTasks` owns task registration, message fan-in, completion handling, and suspend/resume support.
- Restart-capable tasks (`AgentConnection`, `PingPong`) implement `RestartableBackgroundTask`.
- `MessageBus` is the only intended communication surface for tasks:
1. `send` to parent
2. `recv` from parent
3. `send_agent` using current agent tx handle

## Layer <-> IntProxy Protocol (`mirrord-intproxy-protocol`)

- This protocol is intentionally not backward compatible across releases because layer + intproxy are shipped together.
- Message envelope: `LocalMessage<T> { message_id, inner }`.
- Layer requests are `LayerToProxyMessage`; proxy responses are `ProxyToLayerMessage`.
- Custom bincode framing uses 4-byte big-endian length prefix + payload.
- Both sync and async codecs are supported:
1. Sync path for layer-side hooks.
2. Async path for tokio-based intproxy runtime.

## Operational Invariants

- Keep `IntProxy::handle_agent_message` and `IntProxy::handle_layer_message` exhaustive on protocol enums.
- Keep `pending_layers` accounting correct: add when request expects response, remove on `ToLayer`.
- Preserve request queue ordering assumptions when changing agent-side behavior.
- Any reconnect path must flush pending responses to avoid hanging hooked syscalls in the layer.
- Any new layer fork behavior should be mirrored across `FilesProxy`, `IncomingProxy`, and `OutgoingProxy`.

## Change Workflows

### Adding a new `LayerToProxyMessage`

1. Add protocol types in `mirrord/intproxy/protocol/src/lib.rs`.
2. Route request in `IntProxy::handle_layer_message` (`src/lib.rs`).
3. Add handling in the appropriate feature proxy.
4. Add response mapping in layer if applicable.
5. Run `cargo check -p mirrord-intproxy --keep-going`.

### Adding a new `DaemonMessage` consumed by intproxy

1. Route in `IntProxy::handle_agent_message` (`src/lib.rs`).
2. Forward to proper proxy task message type.
3. Add reconnect/failover behavior if request/response lifecycle is stateful.
4. Add/update tests in `src/lib.rs` or feature proxy tests.

### Extending reconnect behavior

1. Ensure `ConnectionRefresh::Start` clears all stale local state.
2. Ensure pending layer requests get deterministic error responses.
3. Ensure `ConnectionRefresh::End` updates tx handles and restores protocol-gated behavior.
4. Validate queue replay correctness under concurrent new layer requests.
