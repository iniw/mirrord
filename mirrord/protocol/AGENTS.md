# AGENTS.md

Context for AI agents working in `mirrord/protocol` (`mirrord-protocol`).

## Scope

This file covers:
- `mirrord/protocol/src` (wire-level message types, compatibility gates, and codec used by layer/intproxy/agent/operator)
- `mirrord/protocol/Cargo.toml` (independent protocol crate versioning)
- `mirrord/protocol/README.md` (crate overview)

## Quick Reference

```bash
# Main crate
cargo check -p mirrord-protocol --keep-going
cargo test -p mirrord-protocol
```

## Key Paths

- Crate-level compatibility and version contract: `src/lib.rs`
- Top-level protocol messages + bincode codec: `src/codec.rs`
- File operation protocol types and FS metadata contracts: `src/file.rs`
- Incoming mirror/steal TCP protocol and HTTP filter types: `src/tcp.rs`
- Outgoing protocol types and socket-address serialization: `src/outgoing.rs`, `src/outgoing/{tcp,udp}.rs`
- DNS and reverse-DNS protocol messages: `src/dns.rs`
- Shared remote error transport types: `src/error.rs`
- Byte payload wrapper used by multiple message families: `src/payload.rs`
- Request correlation ID type with stable encoding: `src/uid.rs`
- Batched HTTP body frame helper used by protocol consumers: `src/batched_body.rs`
- VPN message types: `src/vpn.rs`

## Architecture

### Protocol Ownership

- `mirrord-protocol` is the canonical wire contract for communication across independently shipped components.
- Rust type layout in this crate is the source of truth for serialized representation.
- `ClientMessage` and `DaemonMessage` in `src/codec.rs` are the top-level envelopes; feature-specific modules define payload enums/structs.

### Serialization Model

- Wire format is `bincode` as implemented by the concrete Rust types in this crate.
- `ProtocolCodec<I, O>` (plus `ClientCodec`/`DaemonCodec`) handles stream decode/encode for protocol traffic.
- Types with custom serialization behavior (`Payload`, `Uid`) must preserve wire compatibility when edited.
- Debug formatting intentionally redacts some sensitive values (`RemoteEnvVars`).

### Compatibility and Versioning

- Backward compatibility is mandatory for this crate.
- Compatibility-safe changes include:
1. Renaming types/fields/variants.
2. Changing a field type only if the bincode representation is unchanged.
3. Adding a new enum variant only at the end, and only when gated by negotiated protocol version.
4. Code changes unrelated to wire representation.
- Compatibility-breaking examples include:
1. Adding a field.
2. Changing `T` <-> `Option<T>`.
3. Reordering enum variants.
- Capability gates are centralized as `LazyLock<VersionReq>` constants near related types (`file.rs`, `tcp.rs`, `dns.rs`, `error.rs`, `outgoing.rs`, `codec.rs`, `lib.rs`).
- Version negotiation is done by consumers using `SwitchProtocolVersion`; this crate provides `VERSION` and the gated feature constants.

### CI/Release Constraints

- This crate is versioned independently from most workspace crates.
- CI enforces that any change under `mirrord/protocol/**` must include a `mirrord/protocol/Cargo.toml` change.
- Version bump policy from crate docs:
1. Protocol extension: bump minor.
2. Non-extension/internal change: bump patch.
- Major-version cleanup points are tracked with `#[protocol_break(<major>)]`.

## Operational Invariants

- Never reorder existing enum variants that are serialized on the wire.
- For additive protocol changes, append new variants and gate sending behavior by negotiated version.
- Keep deprecated variants that still participate in compatibility paths until a planned major break.
- Keep cross-platform representation types stable (`SocketAddress`, `UnixAddr`, `MetadataInternal`, `FsMetadataInternal*`, `RemoteIOError`).
- Keep `Encode`/`Decode` derivations or custom impls symmetric and deterministic.
- When introducing new request/response variants, maintain explicit routing expectations in downstream crates (`mirrord-intproxy`, `mirrord-agent`, `mirrord-layer`, and operator-side integrations).

## Change Workflows

### Adding a new protocol capability

1. Add new message/types in this crate, keeping enum ordering compatibility rules.
2. Add a minimum supported `VersionReq` constant near the new type.
3. Add conversion/fallback behavior where older peers need alternate messages (for example V1/V2 request compatibility).
4. Update all consumers to handle the new variant explicitly.
5. Bump `mirrord/protocol/Cargo.toml` version (usually minor for new capability).
6. Run checks:
   - `cargo check -p mirrord-protocol --keep-going`
   - `cargo check -p mirrord-intproxy --keep-going`
   - `cargo check -p mirrord-layer --keep-going`
   - `cargo check -p mirrord-agent --target x86_64-unknown-linux-gnu --keep-going`

### Changing existing protocol types without wire changes

1. Confirm encoded representation stays identical (or add tests proving compatibility).
2. Keep/extend version gates if behavior depends on negotiated versions.
3. Bump patch version in `mirrord/protocol/Cargo.toml`.
4. Run `cargo test -p mirrord-protocol` and at least one downstream consumer check.

### Preparing for a major compatibility break

1. Mark transitional code paths with `#[protocol_break(<next-major>)]`.
2. Keep old variants/types in place until the major bump is intentionally performed.
3. At major bump time, remove transitional paths and resolve compile errors raised by `protocol_break`.
