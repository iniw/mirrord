# AGENTS.md

Context for AI agents working in `mirrord/operator` (`mirrord-operator`).

## Scope

This file covers:
- `mirrord/operator/src/crd.rs` and `mirrord/operator/src/crd/**` (CRD type definitions and schema contract)
- `mirrord/operator/src/client.rs` and `mirrord/operator/src/client/**` (CLI-side operator API client)
- How this crate is consumed by `mirrord/cli` and by the operator repository at `../operator`
- CRD compatibility rules across operator upgrades

## Quick Reference

```bash
# CRD model only
cargo check -p mirrord-operator --features crd --keep-going

# Full client + CRD model used by CLI
cargo check -p mirrord-operator --features client --keep-going
cargo test -p mirrord-operator --features client

# CRD-focused tests
cargo test -p mirrord-operator --features crd

# Regenerate CRD YAMLs under mirrord/operator/crds/*.yaml
cargo test -p mirrord-operator --features crd write_all_crd_yamls -- --ignored
```

## Key Paths

- CRD module root: `src/crd.rs`
- Core session/copy target CRDs: `src/crd/session.rs`, `src/crd/copy_target.rs`
- Preview CRD: `src/crd/preview.rs`
- Policy/Profile CRDs: `src/crd/policy.rs`, `src/crd/profile.rs`
- Queue splitting CRDs: `src/crd.rs`, `src/crd/kafka.rs`
- DB branching CRDs: `src/crd/db_branching/{core,mysql,pg,mongodb}.rs`
- Multi-cluster CRD: `src/crd/multi_cluster.rs`
- Client entrypoint: `src/client.rs`
- Connect URL/query encoding contract: `src/client/connect_params.rs`
- Branch CRD creation/wait logic: `src/client/database_branches.rs`

Cross-repo consumers:
- CLI orchestration: `../mirrord/mirrord/cli/src/connection.rs`
- CLI preview CRD creation/watch: `../mirrord/mirrord/cli/src/preview.rs`
- CLI operator status/session routes: `../mirrord/mirrord/cli/src/operator/{status,session}.rs`
- Operator HTTP routes and reconciliation: `../operator/operator/controller/src/**`
- Operator startup wiring for controllers: `../operator/operator/service/src/main.rs`

## Crate Role

`mirrord-operator` is the shared contract crate between CLI and operator:
- `feature = "crd"`: source-of-truth Rust types for CRD specs/statuses, including `schemars` schema generation.
- `feature = "client"`: operator API client used by CLI to discover operator status, create/connect sessions, and create some CRDs.

## CLI Interaction Map

The CLI frequently creates or consumes resources defined in this crate:

1. Main `mirrord exec` / operator flow
- Entry: `mirrord/cli/src/connection.rs::create_and_connect`.
- Uses `OperatorApi::try_new` to fetch `MirrordOperatorCrd`.
- Uses `OperatorApi::connect_in_new_session` / `connect_in_multi_cluster_session`.
- May create `CopyTargetCrd` directly (`Api<CopyTargetCrd>::create`).
- May create DB branch CRDs (`MysqlBranchDatabase`, `PgBranchDatabase`, `MongodbBranchDatabase`) before connect.

2. `mirrord preview ...`
- CLI creates, watches, lists, and deletes `PreviewSession` CRs directly via Kubernetes `Api<PreviewSession>`.
- Operator preview controller reconciles these CRs in `../operator/operator/preview-env/src/controller.rs`.

3. `mirrord operator status`
- Reads `MirrordOperatorCrd` and status payload types from this crate.
- Compatibility formatting (`legacy` vs `modern`) is handled operator-side in `../operator/operator/controller/src/status.rs`.

4. `mirrord operator session ...`
- Uses `Api<SessionCrd>` delete routes (`/sessions`, `/sessions/{id}`, `/sessions/inactive`).

5. Profile resolution
- CLI fetches `MirrordClusterProfile` and `MirrordProfile` from cluster (`cli/src/profile.rs`).

## CRD Ownership in `../operator`

When changing a CRD here, check its reconciler there:

- `MirrordClusterSession` -> `../operator/operator/session/src/**`
- `PreviewSession` -> `../operator/operator/preview-env/src/**`
- `CopyTargetCrd` -> `../operator/operator/context` + `../operator/operator/controller/src/copy_target/**`
- `MirrordWorkloadQueueRegistry` / `MirrordSqsSession` -> `../operator/operator/sqs-splitting/src/**`
- `MirrordKafka*` CRDs -> `../operator/operator/kafka-splitting/src/**`
- `*BranchDatabase` CRDs -> `../operator/operator/db-branching/src/**`
- `MirrordClusterWorkloadPatch*` -> `../operator/operator/workload-patch/src/**` and pod mutator integration
- `MirrordOperatorCrd`, `TargetCrd`, `SessionCrd` route contract -> `../operator/operator/controller/src/{status,target,restful,openapi}.rs`

## Backward Compatibility (Critical)

CRD compatibility is the highest-risk area in this crate.

### Why this is critical

- CRD schemas are generated from these Rust types via `schemars`.
- Those schemas are installed into Kubernetes and validated by the API server.
- Operator + CRDs are delivered through Helm charts (chart repo is external: `metalbear-co/charts`; operator repo tracks it via `../operator/helm-chart-ref.txt`).
- During upgrades, existing CR objects created under older schemas remain in the cluster.
- New operator versions must still deserialize and reconcile those old CR objects.

If deserialization fails, reconciliation loops fail permanently for that object, which can block operator progress.

### Hard rules

- Prefer additive changes.
- New fields on existing CRDs should usually be optional: `Option<T>` + `#[serde(default, skip_serializing_if = "Option::is_none")]` when appropriate.
- Do not remove/rename/change type of existing serialized fields without a compatibility plan.
- Keep `group/version/kind/plural` stable for existing resources unless you implement a full multi-version migration path.
- Keep `TargetCrd::urlfied_name` and connect URL contracts stable (see `client.rs` URL compatibility tests).
- Use forward-compatible enums for evolving feature sets (`#[serde(other)]` and explicit unknown variants where possible).
- When wire shape must change, use compatibility wrappers like:
  - `CopyTargetEntryCompat`
  - `LockedPortCompat`
- Keep old compatibility fields until both old clients and old operators are no longer supported (see deprecated fields in `MirrordOperatorSpec`).

### Existing compatibility patterns to preserve

- `NewOperatorFeature::Unknown` and other `serde(other)` variants.
- `KubeTarget` handling of unknown target types for forward compatibility.
- Profile `unknown_fields` flattening for strict CLI validation.
- `CopyTargetEntryCompat` and `LockedPortCompat` dual-format deserialize with modern schema exposure.
- Query param JSON encoding contract shared by:
  - client: `src/client/connect_params.rs`
  - operator server parser: `../operator/operator/controller/src/restful/params.rs`

## Helm and Release Coordination

The operator Helm chart source lives in the sibling repository at `../charts/`.

`../operator/helm-chart-ref.txt` is used only by operator e2e tests to select the chart commit.
Operator/chart runtime sync is driven by the chart itself (it defines which operator version/image is installed).

When CRD/API changes are made here:
- Update corresponding operator controllers/routes in `../operator`.
- Update chart templates/CRDs and chart version in `../charts/`.
- Update `../operator/helm-chart-ref.txt` only when e2e should test a newer chart ref.
- If new resources/verbs are needed, update RBAC in both:
  - chart templates
  - `mirrord operator setup` generation path (`mirrord/operator/src/setup.rs` in the main repo, when relevant)

## Operational Invariants

- Keep enum handling exhaustive across CLI and operator consumers.
- Do not silently change API route shapes generated from `CustomResource` metadata.
- Keep connect query semantics aligned between client serializer and operator deserializer.
- Preserve compatibility tests that assert stable URLs and serialized forms.

## Change Workflows

### Adding a new CRD

1. Add type in `src/crd.rs` or `src/crd/**` with `CustomResource` + `JsonSchema`.
2. Add a test printing it's output to `src/crd.rs`.
3. Wire CLI/client usage if needed.
4. Add operator reconciler/route in `../operator`.
5. Update chart + RBAC in `../charts/`.
6. Run checks and format.

### Modifying an existing CRD

1. First design for backward compatibility.
2. Prefer optional/additive fields.
3. If shape changed, add compat deserialization path.
4. Add regression tests for old payloads where practical.
5. Validate affected CLI and operator routes/controllers.
6. Coordinate chart rollout.

### Changing connect URL/query contracts

1. Update client URL/query builder (`src/client.rs`, `src/client/connect_params.rs`).
2. Update operator parser (`../operator/operator/controller/src/restful/params.rs`).
3. Update compatibility tests in both repos.
4. Treat these as public API changes; avoid breaking existing clients.
