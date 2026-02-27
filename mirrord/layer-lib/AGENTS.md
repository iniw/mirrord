# AGENTS.md

Context for AI agents working in `mirrord/layer-lib` (`mirrord-layer-lib`).

## Scope

This file covers:
- `mirrord/layer-lib/src` (shared runtime used by Unix `mirrord-layer` and Windows layer code)
- `mirrord/layer-lib/tests` (crate-level tests; currently Windows-focused)

## Quick Reference

```bash
# Shared layer runtime crate
cargo check -p mirrord-layer-lib --keep-going
cargo test -p mirrord-layer-lib
```

## Key Paths

- Crate surface exports: `src/lib.rs`
- Setup and derived runtime state: `src/setup.rs`
- Layer <-> intproxy sync connection and response matching: `src/proxy_connection.rs`
- Detour/bypass control flow primitives: `src/detour.rs`
- Shared error types and errno/WSA mapping: `src/error.rs`, `src/error/windows.rs`
- File policy and remapping: `src/file/filter.rs`, `src/file/mapper.rs`, `src/file/{unix,windows}/**`
- Socket shared model and ops: `src/socket.rs`, `src/socket/sockets.rs`, `src/socket/ops.rs`
- DNS and hostname helpers: `src/socket/dns.rs`, `src/socket/dns_selector.rs`, `src/socket/hostname.rs`
- Logging and diagnostics helpers: `src/logging.rs`, `src/debugger_ports.rs`, `src/mutex.rs`, `src/trace_only.rs`
- Windows process creation/injection helpers: `src/process/windows/**`

## Architecture

### Setup and Derived State

- `init_layer_setup` computes runtime config adjustments before hooks run:
1. SIP-only mode forces local fs behavior.
2. Targetless mode forces `FsModeConfig::LocalWithOverrides` (unless already local).
3. Trace-only mode disables agent-dependent features.
- `LayerSetup` is a global `OnceLock` that holds derived data reused by hook logic:
1. file filter/remapper
2. outgoing and DNS selectors
3. incoming mode helper
4. proxy address and other resolved config flags

### Proxy Request Transport

- `ProxyConnection` in `src/proxy_connection.rs` owns framed sync TCP codecs for local
  layer <-> intproxy traffic.
- Message IDs come from `AtomicU64`.
- `ResponseManager` handles out-of-order responses by parking unmatched replies in a map keyed by
  `message_id`.
- Global helper functions (`make_proxy_request_with_response` / `make_proxy_request_no_response`)
  route through `PROXY_CONNECTION`.

### Detour/Error Semantics

- `Detour<S>` (Unix path) is the core hook return model: `Success`, `Bypass`, `Error`.
- `DetourGuard` prevents recursive interception when layer code internally touches hooked APIs.
- `Bypass` variants are behavior contracts (not only failures): they decide when to call original
  libc/Winsock flow.
- `HookError` and `LayerError` map to platform-native errno/WSA values in `src/error.rs`.

### File and Socket Shared Helpers

- `FileFilter` combines explicit user regex sets with platform defaults (local, read-only,
  not-found).
- `FileRemapper` applies the first matching mapping regex replacement.
- `SOCKETS` stores managed descriptors as `HashMap<SocketDescriptor, Arc<UserSocket>>`.
- `SocketState` models lifecycle (`Initialized`, `Bound`, `Listening`, `Connected`).
- `connect_common` centralizes outgoing connect interception/fallback flow.
- `UserSocket::close` sends cleanup messages for active listening/outgoing tracked sockets.

### DNS and Hostname Helpers

- `DnsSelector` implements local-vs-remote DNS routing based on resolved config filters.
- `remote_getaddrinfo` performs remote DNS over proxy and updates reverse DNS cache.
- `socket/hostname.rs` provides remote hostname/config-file reads via RAII-wrapped remote file
  access.

### Windows Process Helpers

- `process/windows/execution` centralizes CreateProcess + injection support and child env
  propagation.
- `LayerInitEvent` (`process/windows/sync.rs`) coordinates parent-child init completion.
- `SHARED_SOCKETS_ENV_VAR` and resolved config forwarding preserve behavior across child process
  creation.

## Operational Invariants

- `SETUP` and `PROXY_CONNECTION` are single-assignment globals; preserve initialization ordering.
- Keep error mapping exhaustive when adding new `Bypass`, protocol, DNS, or connect failure
  variants.
- Preserve message correlation invariants in `ProxyConnection` (`message_id` generation and
  outstanding response queue behavior).
- Preserve socket handoff compatibility via `MIRRORD_SHARED_SOCKETS` encoding/decoding.
- Keep config-derived behavior inside `LayerSetup` so Unix and Windows layer codepaths stay
  aligned.

## Change Workflows

### Adding a New Shared Proxy Request Helper

1. Add/extend protocol types in shared protocol crates.
2. Add helper logic in `layer-lib` using proxy request helper functions.
3. Map new failure modes in `HookError`/`LayerError` if needed.
4. Run `cargo check -p mirrord-layer-lib --keep-going`.

### Extending Bypass/Detour Behavior

1. Add `Bypass` variant and update handling call-sites.
2. Update platform error mapping paths.
3. Verify both Unix and Windows users of that helper keep expected bypass semantics.
4. Run crate checks and relevant layer tests.

### Changing Setup/Config Derivation

1. Update `init_layer_setup` / `LayerSetup::new`.
2. Keep file/network selectors and hook gating aligned with `mirrord-config`.
3. Validate targetless and trace-only behavior.
