## Overview

### Hooking Infrastructure

`HookManager` wraps Frida interceptor transactions (`begin_transaction` on creation, `end_transaction` on drop).
`replace!` (and `replace_with_fallback!`) resolves a symbol, installs the detour, and stores the original function
pointer.

`#[hook_fn]` and `#[hook_guard_fn]` from `mirrord-layer-macro` generate:
1. `Fn*` type alias
2. `FN_*` global storage for the original function
3. Optional `DetourGuard` pre-check (`hook_guard_fn`)

### Shared State

- `OPEN_FILES`: maps local fd → `Arc<RemoteFile>`
- `SOCKETS`: maps local fd → `Arc<UserSocket>`
- `OPEN_DIRS`: tracks open remote directory streams and stable dirent buffers
- `MANAGED_ADDRINFO`: tracks layer-allocated `addrinfo` chains to decide whether `freeaddrinfo` is custom or libc

`dup` syscall semantics rely on `Arc` ownership, so remote resources are closed only when the final reference is
dropped.

## Rules

- Be careful with lock ordering and fork paths. `fork_detour` grabs major layer locks before fork to avoid
inherited-locked-mutex deadlocks. Deadlocks are easy to introduce.
