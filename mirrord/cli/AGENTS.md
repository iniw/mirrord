# AGENTS.md

Context for AI agents working in `mirrord/cli` (the `mirrord` binary crate).

## Scope

This file covers:
- `mirrord/cli/src` (command parsing, command dispatch, process execution, operator/agent connection setup, proxy subprocess bootstrap)
- CLI-specific flows that spawn or configure `mirrord-intproxy` and `mirrord-extproxy`
- IDE-facing and machine-readable command contracts (`ext`, `container-ext`, `verify-config`, `ls`, proxy address handshakes)

## Quick Reference

```bash
# Main CLI crate
cargo check -p mirrord --keep-going
cargo test -p mirrord

# Useful focused tests
cargo test -p mirrord parse_accept_invalid_certificates
cargo test -p mirrord deny_non_oss_targets_without_operator

# If editing wizard-gated code paths
cargo check -p mirrord --features wizard --keep-going
```

## Key Paths

- Command entrypoint and dispatch: `src/main.rs`
- CLI argument model and env override mapping: `src/config.rs`
- Exec/container bootstrap and proxy subprocess spawning: `src/execution.rs`
- Operator-first then OSS fallback connection flow: `src/connection.rs`
- Internal proxy command bootstrap: `src/internal_proxy.rs`
- External proxy command bootstrap: `src/external_proxy.rs`
- Container runtime + sidecar orchestration: `src/container.rs`, `src/container/**`
- IDE extension exec flow: `src/extension.rs`
- Config verification JSON contract for IDEs: `src/verify_config.rs`
- Target listing flow and operator-dependent behavior: `src/list.rs`
- Port-forward runtime: `src/port_forward.rs`
- CI command flow and pid store lifecycle: `src/ci.rs`, `src/ci/**`
- Tracing/log initialization policy: `src/logging.rs`
- CLI/user-facing diagnostics and error shaping: `src/error.rs`

## Architecture

### Runtime Shell

- `main` parses `Cli`, initializes tracing, loads `UserData`, and dispatches `Commands`.
- Runtime is single-threaded tokio (`new_current_thread`).
- Proxy commands (`intproxy`, `extproxy`) initialize tracing separately from the normal command path.

### Config Lifecycle and Precedence

- CLI flags are converted into env overrides (mainly through `ExecParams::as_env_vars` and peers).
- Resolved config is built via `LayerConfig::resolve(ConfigContext)`.
- Profile adjustments are applied with `profile::apply_profile_if_configured` before verification.
- Verification warnings are emitted from `ConfigContext::into_warnings`.
- Precedence remains: CLI args > env vars > config file.

### `exec` / `ext` Execution Path

1. Validate execution context (`ensure_not_nested` for `exec`).
2. Resolve and verify config (+ optional version check).
3. `create_and_connect` attempts operator flow first, then falls back to OSS as needed.
4. `MirrordExecution::start_internal`:
   - extracts layer library (or uses `MIRRORD_LAYER_FILE`);
   - fetches remote env when enabled;
   - spawns `mirrord intproxy` subprocess;
   - reads printed intproxy address from subprocess stdout;
   - prepares injected env (`LD_PRELOAD`/`DYLD_INSERT_LIBRARIES`, resolved config, proxy addr).
5. `exec` does `execve` (or Windows managed process path); `ext` serializes execution info and waits for proxy exit.

### Operator vs OSS Connect Policy

- `try_connect_using_operator` behavior:
1. `operator == Some(false)`: skip operator.
2. `operator == Some(true)`: operator is required; missing operator/license is an error.
3. `operator == None`: try operator; fall back to OSS on not installed/invalid license.
- OSS mode must still run `process_config_oss` checks (operator-only target/features rejected, warnings emitted for multi-pod/http filter UX).

### Proxy Process Topology

- Internal proxy (`intproxy`) and external proxy (`extproxy`) are launched as CLI subprocesses.
- Connect info is passed through `MIRRORD_AGENT_CONNECT_INFO` (JSON-serialized `AgentConnectInfo`).
- Both proxy commands perform initial `connect_and_ping` handshake before serving clients.
- `intproxy` prints its listener address to stdout for the layer parent flow; `extproxy` prints its address for sidecar setup.

### Container Flow

- `container_command` and `container_ext_command`:
1. resolve/verify config;
2. start native `extproxy`;
3. start intproxy sidecar container;
4. inject env/network/volume runtime args for the user container command.
- Optional TLS between sidecar intproxy and extproxy is created with `SecureChannelSetup`.

### Machine-Readable Command Contracts

- `verify-config` prints JSON with `VerifiedConfig`, including special targetless serialization for IDE behavior.
- `ls` prints JSON (rich or compatibility format).
- `ext` and `container-ext` print serialized execution/runtime data to progress output.
- For hidden/proxy/helper commands, stdout is part of an API surface; extra stdout noise can break callers.

## Change Workflows

### Adding a New CLI Command

1. Add args/variant in `src/config.rs` (`Commands` + argument structs).
2. Wire execution branch in `src/main.rs`.
3. Initialize progress/analytics and config resolve/verify as needed.
4. Apply platform gating (`windows_unsupported!`, `#[cfg]`, `#[cfg_attr(..., command(hide = true))`) where appropriate.
5. Add/adjust parsing and behavior tests.

### Changing `exec`/`ext` Bootstrap

1. Update `src/main.rs` orchestration and `src/execution.rs` spawn/environment logic together.
2. Validate injection env vars, resolved config propagation, and proxy address propagation.
3. Re-check macOS SIP patch and Linux static binary warning paths if touched.
4. Ensure extension flow (`src/extension.rs`) remains compatible.
