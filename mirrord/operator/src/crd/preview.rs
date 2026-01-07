use kube::CustomResource;
use mirrord_config::target::Target;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::crd::Session;

/// This resource represents a preview environment pod created with a user-provided image
/// and a mirrord-agent sidecar for traffic control.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[kube(
    group = "operator.metalbear.co",
    version = "v1",
    kind = "PreviewTarget",
    root = "PreviewTargetCrd",
    status = "PreviewTargetStatus",
    namespaced
)]
pub struct PreviewTargetSpec {
    /// User's container image to run in the preview pod.
    pub image: String,

    /// Environment key used to group related preview pods and for traffic filtering.
    pub key: String,

    /// Optional target to copy configuration from.
    /// If not specified, creates a minimal "targetless" preview pod.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,

    /// Target namespace (used when target is specified).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_namespace: Option<String>,

    /// TTL in seconds for the preview Job (Job.spec.ttlSecondsAfterFinished).
    /// Defaults to 3600 (1 hour).
    #[serde(default = "default_ttl")]
    pub ttl_seconds: u32,

    /// mirrord-agent image to use for the sidecar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_image: Option<String>,

    /// Log level for the mirrord-agent sidecar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_log_level: Option<String>,
}

fn default_ttl() -> u32 {
    3600
}

/// Status of a preview target resource.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct PreviewTargetStatus {
    /// The session that created this preview target.
    pub creator_session: Session,

    /// Current phase of the preview.
    ///
    /// Either `InProgress`, `Ready`, or `Failed`.
    pub phase: Option<String>,

    /// Name of the Job created for this preview.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_name: Option<String>,

    /// Name of the pod created by the Job.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_name: Option<String>,

    /// Namespace where the preview pod is running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_namespace: Option<String>,

    /// Port on which the mirrord-agent sidecar accepts connections.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_port: Option<u16>,

    /// Optional message describing the reason for failure.
    /// Only set when `phase` is `Failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
}

impl PreviewTargetStatus {
    pub const PHASE_IN_PROGRESS: &'static str = "InProgress";
    pub const PHASE_READY: &'static str = "Ready";
    pub const PHASE_FAILED: &'static str = "Failed";
}
