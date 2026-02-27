use std::time::SystemTime;

use k8s_openapi::chrono::Utc;
use mirrord_analytics::{Analytics, AnalyticsHash, AnalyticsReporter, ExecutionKind, Reporter};
use mirrord_operator::{
    client::{ClientCertificateState, OperatorApi},
    crd::preview::PreviewSession,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const PREVIEW_ENV_START_EVENT: &str = "preview_env_start";
const PREVIEW_ENV_STOP_EVENT: &str = "preview_env_stop";
const PREVIEW_ENV_STATUS_EVENT: &str = "preview_env_status";
const PREVIEW_ENV_FAILED_EVENT: &str = "preview_env_failed";

#[derive(Clone)]
pub struct PreviewTelemetryContext {
    enabled: bool,
    machine_id: Uuid,
    watch: drain::Watch,
    customer_id: Option<AnalyticsHash>,
}

impl PreviewTelemetryContext {
    pub fn new<C: ClientCertificateState>(
        enabled: bool,
        machine_id: Uuid,
        watch: drain::Watch,
        operator_api: &OperatorApi<C>,
    ) -> Self {
        let customer_id = operator_api
            .operator()
            .spec
            .license
            .subscription_id
            .as_deref()
            .map(str::as_bytes)
            .map(AnalyticsHash::from_bytes);

        Self {
            enabled,
            machine_id,
            watch,
            customer_id,
        }
    }

    fn emit_event(
        &self,
        event_name: &'static str,
        preview_key: &str,
        reason: Option<PreviewFailureReason>,
        runtime_seconds: Option<u32>,
    ) {
        if !self.enabled {
            return;
        }

        let mut reporter = AnalyticsReporter::new(
            self.enabled,
            ExecutionKind::Preview,
            self.watch.clone(),
            self.machine_id,
        );

        let mut analytics = Analytics::default();
        if let Some(customer_id) = self.customer_id.clone() {
            analytics.add("customer_id", customer_id);
        }
        analytics.add("preview_key_identifier", hash_preview_key(preview_key));
        analytics.add("timestamp", unix_timestamp_seconds());

        if let Some(reason) = reason {
            analytics.add("reason", reason as u32);
        }

        if let Some(runtime_seconds) = runtime_seconds {
            analytics.add("runtime_seconds", runtime_seconds);
        }

        reporter.get_mut().add(event_name, analytics);
    }

    pub fn emit_start(&self, preview_key: &str) {
        self.emit_event(PREVIEW_ENV_START_EVENT, preview_key, None, None);
    }

    pub fn emit_status(&self, preview_key: &str) {
        self.emit_event(PREVIEW_ENV_STATUS_EVENT, preview_key, None, None);
    }

    fn emit_stop_for_session(&self, preview_key: &str, runtime_seconds: u32) {
        self.emit_event(
            PREVIEW_ENV_STOP_EVENT,
            preview_key,
            None,
            Some(runtime_seconds),
        );
    }

    pub fn emit_stop(&self, preview_key: &str, session: &PreviewSession) {
        self.emit_stop_for_session(preview_key, session_runtime_seconds(session));
    }

    pub fn emit_failed(&self, preview_key: &str, reason: PreviewFailureReason) {
        self.emit_event(PREVIEW_ENV_FAILED_EVENT, preview_key, Some(reason), None);
    }
}

#[repr(u32)]
#[derive(Clone, Copy)]
pub enum PreviewFailureReason {
    Timeout = 1,
    FailedPhase = 2,
    SessionDeleted = 3,
    WatchError = 4,
    StreamClosed = 5,
    TargetResolutionFailed = 6,
    ListFailed = 7,
    NotFound = 8,
    DeleteFailed = 9,
}

fn hash_preview_key(preview_key: &str) -> AnalyticsHash {
    AnalyticsHash::from_bytes(&Sha256::digest(preview_key.as_bytes()))
}

fn unix_timestamp_seconds() -> u32 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().try_into().unwrap_or(u32::MAX))
        .unwrap_or_default()
}

fn session_runtime_seconds(session: &PreviewSession) -> u32 {
    session
        .metadata
        .creation_timestamp
        .as_ref()
        .and_then(|creation_time| (Utc::now() - creation_time.0).to_std().ok())
        .map(|duration| duration.as_secs().try_into().unwrap_or(u32::MAX))
        .unwrap_or_default()
}
