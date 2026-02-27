# AGENTS.md

Context for AI agents working in `mirrord/layer` (`mirrord-layer` and `mirrord-layer-macro`).

## Scope

This file covers:
- `mirrord/layer/src` (runtime entrypoint, hook registration, file/socket/exec detours)
- `mirrord/layer/macro` (proc-macros that generate detour boilerplate)
- `mirrord/layer/tests` (integration-level behavior contracts for the layer)

## Related AGENTS Files

- Shared layer runtime helpers used by this crate live in `mirrord/layer-lib`; see
  `../layer-lib/AGENTS.md`.

## Quick Reference

```bash
# Main layer crate
cargo check -p mirrord-layer --keep-going
cargo test -p mirrord-layer

# Proc-macro crate used by hooks
cargo check -p mirrord-layer-macro --keep-going
```

## Key Paths

- Entry and startup flow: `src/lib.rs`, `src/load.rs`
- Hook manager + macro helpers: `src/hooks.rs`, `src/macros.rs`, `macro/src/lib.rs`
- File hooks/ops/state: `src/file/hooks.rs`, `src/file/ops.rs`, `src/file/open_dirs.rs`, `src/file.rs`
- Socket hooks/ops/state: `src/socket/hooks.rs`, `src/socket/ops.rs`, `src/socket.rs`
- Exec/fork/vfork behavior: `src/exec_hooks.rs`, `src/exec_hooks/hooks.rs`, `src/lib.rs`
- Linux Go runtime hooks: `src/go/mod.rs`, `src/go/linux_x64.rs`, `src/go/linux_aarch64.rs`
- macOS-only extras: `src/exec_utils.rs`, `src/tls.rs`
- Layer integration tests: `tests/**/*.rs`, `tests/apps/**`

## Architecture

### Process Load and Mode Selection

- Library constructor is `mirrord_layer_entry_point` (`#[ctor]` in `src/lib.rs`).
- Startup reads resolved config and process metadata (`ExecuteArgs::from_env`).
- `LoadType` is computed in `src/load.rs`:
1. `Full`: normal hooking + proxy session.
2. `Skip`: no syscall interception, but still establishes a lightweight intproxy session (unless trace-only mode).
3. `SIPOnly` (macOS): exec/SIP patch path only.
- `MIRRORD_DONT_LOAD` hard-stops loading.

### Full Startup Sequence

`layer_start` (`src/lib.rs`) performs:
1. tracing init;
2. global setup init (`init_layer_setup(config, false)`);
3. hook registration (`enable_hooks`);
4. intproxy session creation (`ProxyConnection::new` + `NewSessionRequest`);
5. optional remote env fetch/remap/override/unset.

Hook registration is feature-gated by config:
- file hooks only when fs feature is active;
- DNS hooks only when remote DNS is enabled;
- exec hooks on macOS or experimental Linux toggle;
- Go hooks on Linux `x86_64`/`aarch64`.

### Hooking Infrastructure

- `HookManager` wraps Frida interceptor transactions (`begin_transaction` on creation, `end_transaction` on drop).
- `replace!` (and `replace_with_fallback!`) resolves symbol, installs detour, and stores original function pointer.
- `#[hook_fn]` and `#[hook_guard_fn]` from `mirrord-layer-macro` generate:
1. `Fn*` type alias;
2. `FN_*` global original function storage;
3. optional `DetourGuard` pre-check (`hook_guard_fn`).
- Detour return model comes from `mirrord_layer_lib::detour`: `Success`, `Bypass`, `Error`.

### Shared State Model

- Files: `OPEN_FILES` maps local fd -> `Arc<RemoteFile>`.
- Sockets: `SOCKETS` maps local fd -> `Arc<UserSocket>`.
- Directories: `OPEN_DIRS` tracks open remote directory streams and stable dirent buffers.
- DNS pointers: `MANAGED_ADDRINFO` tracks layer-allocated `addrinfo` chains to decide whether `freeaddrinfo` is custom or libc.

Dup semantics rely on `Arc` ownership so remote resources are closed only when final reference is dropped.

### File Subsystem

Core behavior is in `src/file/ops.rs`:
- Canonical path pipeline: `ensure_not_relative_or_not_found` -> remap -> `ensure_remote` filter checks.
- `open` sends remote `OpenFileRequest`, then creates a local fake fd (temp file or `/dev/null`) and inserts mapping into `OPEN_FILES`.
- `openat` supports relative-to-remote-fd operations.
- `read`, `pread`, `write`, `pwrite`, `lseek`, `stat*`, `mkdir*`, `unlink*`, `rename`, `readlink`, `statfs*`, `getdents64` forward to remote operations with bypass/error handling.
- Directory APIs (`opendir`/`readdir`/`closedir`) are implemented via `OPEN_DIRS`, with per-stream reusable dirent buffers.

Important invariants:
- `RemoteFile::Drop` sends remote close; do not add logging there (lock re-entry deadlock risk).
- `close_layer_fd` must remain log-minimal (can run while stdio fds are closing).

### Socket Subsystem

Core behavior is in `src/socket/ops.rs`:
- `socket` creates local fd and inserts `UserSocket` in `Initialized` state.
- `bind` validates domain/state, applies address fallback strategy (`bind_similar_address`), and records requested vs actual local bind address.
- `listen` subscribes remote incoming traffic (`PortSubscribe`) and transitions to `Listening`.
- `accept` resolves remote metadata (`ConnMetadataRequest`) and creates managed accepted sockets.
- `connect` delegates to common outgoing logic; when outgoing interception is active, connects are routed through intproxy/agent.
- `getsockname`/`getpeername` translate between layer-facing and remote-facing addresses.
- `dup*`/`fcntl(F_DUP*)` preserve socket/file map correctness.

UDP + DNS special handling:
- `sendto`/`sendmsg` has special port-53 path that semantically "connects" to interceptor flow for DNS.
- `recvfrom` patches source address based on managed socket state.
- DNS hooks cover `getaddrinfo`, `gethostbyname`, `gethostname`, and macOS DNS config shims.

### Process Lifecycle: close/fork/vfork/exec

- `close_detour` always calls original close then `close_layer_fd` cleanup.
- `fork_detour` grabs major layer locks before fork to avoid inherited locked mutex deadlocks; child creates a new proxy session using `parent_layer` linkage.
- `vfork_detour` emulates vfork semantics using `fork` + `CLOEXEC` pipe synchronization to avoid UB from writing in shared-memory child.
- `execve`/`execv` hooks inject serialized shared sockets into `MIRRORD_SHARED_SOCKETS` env (`prepare_execve_envp`).

### Linux Go Runtime Hooking

- Linux Go binaries are handled with architecture-specific assembly detours.
- Go runtime version is detected from `runtime.buildVersion.str`.
- Hook targets differ by Go version (pre-1.19, 1.19+, 1.23+, 1.25+, 1.26 symbol moves).
- Detours route selected syscalls back into layer file/socket detours while preserving Go runtime calling conventions.
- Optional `dlopen` hook path enables Go hooks in dynamically loaded modules (`experimental.dlopen_cgo`).

### macOS-Specific Paths

- SIP-only mode and patching logic flows through `exec_utils`.
- Optional TLS trust override hook (`SecTrustEvaluateWithError`) is enabled by experimental config.
- Additional syscall symbol variants (`$NOCANCEL`, `$INODE64`) are hooked where needed for compatibility.

## Operational Invariants

- Keep hook registration exhaustive for each feature gate; missing variants often break specific runtimes (Node/libuv/Go).
- Preserve state-machine validity (`Initialized` -> `Bound` -> `Listening` / `Connected`) in socket flows.
- Any new detour that allocates C-owned memory must define exact free path ownership.
- Be careful with lock ordering and fork paths; deadlocks are easy to introduce.
- Do not emit logs from paths documented as unsafe for logging (`RemoteFile::Drop`, `close_layer_fd`).

## Change Workflows

### Adding a New libc Hook

1. Add detour in `src/file/hooks.rs`, `src/socket/hooks.rs`, or `src/exec_hooks/hooks.rs`.
2. Implement safe logic in corresponding `ops.rs` when possible.
3. Register symbol in `enable_*_hooks`.
4. Ensure bypass path calls original function with correct pointer/value semantics.
5. Add/update integration tests in `mirrord/layer/tests`.

### Adding a New Remote File/Socket Operation

1. Add protocol message(s) in `mirrord-protocol`.
2. Implement layer-side request/response handling.
3. Update intproxy routing.
4. Update agent handler.
5. Run targeted checks/tests for all touched crates.

### Changing fork/exec/shared-socket Behavior

1. Validate `fork_detour` child re-session flow.
2. Validate `MIRRORD_SHARED_SOCKETS` propagation and decoding path.
3. Run tests focused on fork/exec/dup/listen behavior (`fork`, `spawn`, `issue864`, `dup_listen`, `double_listen`).

### Changing Go Hooking

1. Update correct arch file (`linux_x64.rs` or `linux_aarch64.rs`).
2. Re-check runtime symbol names by Go version.
3. Preserve register/stack ABI assumptions in naked assembly.
4. Run layer integration tests that cover Go syscall paths.
