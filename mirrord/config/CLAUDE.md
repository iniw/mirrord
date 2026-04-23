## Overview

Most public config structs derive `MirrordConfig`. The derive macro generates a `*FileConfig` type where all fields
become `Option<_>`, plus an implementation that resolves final values into the runtime struct.

- `#[config(nested)]` means the field is resolved through `FromMirrordConfig::Generator`.
- `#[config(toggleable)]` wraps nested generators with `ToggleableConfig`, so users can write `true`/`false` as shorthand.

### How values are resolved

Value resolution uses `MirrordConfigSource` combinators (`.or`, `.layer`), chaining env + file values. Precedence: CLI
args > env vars > config file.

Resolved configs are passed to downstream processes via `LayerConfig::encode`/`decode` and the `MIRRORD_RESOLVED_CONFIG`
env var.

### Validation

`LayerConfig::verify` checks for conflicting settings across features (targetless/copy-target/operator, incoming filter
exclusivity, fs buffer limits, startup retry bounds, etc.). Sub-verifiers live in feature modules (`dns`, `outgoing`,
`split_queues`).
