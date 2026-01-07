use kube::CustomResource;
use mirrord_config::target::Target;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// This resource represents a preview environment pod created by copying a target's
/// pod spec and replacing the container image with the user's image.
///
/// The preview pod is isolated from the original Service by modified labels.
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

    /// Target to copy pod configuration from (deployment, pod, statefulset, etc.).
    /// The preview pod will be a copy of the target's pod spec with the user's image.
    pub target: Target,

    /// Target namespace (defaults to the PreviewTarget's namespace if not specified).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_namespace: Option<String>,
}

/// Status of a preview target resource.
#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct PreviewTargetStatus {
    /// Current phase of the preview.
    ///
    /// Either `InProgress`, `Ready`, or `Failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,

    /// Name of the preview pod.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_name: Option<String>,

    /// Namespace where the preview pod is running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pod_namespace: Option<String>,

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
