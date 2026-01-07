//! Handlers for `mirrord preview` commands.
//!
//! Preview environments allow developers to deploy their code to the cluster
//! in a persistent pod that can be shared with others for testing and review.

use std::time::{Duration, Instant};

use drain::Watch;
use futures::StreamExt;
use kube::{
    Api, ResourceExt,
    api::{DeleteParams, PostParams},
    runtime::watcher::{self, Event, watcher},
};
use mirrord_analytics::NullReporter;
use mirrord_config::{LayerConfig, config::ConfigContext};
use mirrord_operator::{
    client::{NoClientCert, OperatorApi},
    crd::{PreviewTargetCrd, PreviewTargetSpec, PreviewTargetStatus},
};
use mirrord_progress::{Progress, ProgressTracker};
use tokio::{pin, time::interval};
use tracing::Level;

use crate::{
    config::{PreviewArgs, PreviewCommand, PreviewStartArgs},
    error::{CliError, CliResult},
    user_data::UserData,
};

/// Handle commands related to preview environments: `mirrord preview ...`
pub(crate) async fn preview_command(
    args: PreviewArgs,
    _watch: Watch,
    _user_data: &UserData,
) -> CliResult<()> {
    match args.command {
        PreviewCommand::Start(start_args) => preview_start(start_args).await,
    }
}

/// Handle `mirrord preview start` command.
///
/// Creates a new preview environment or updates an existing one by creating
/// a `PreviewTargetCrd` resource that the operator will reconcile.
#[tracing::instrument(level = Level::TRACE, ret, skip_all)]
async fn preview_start(args: PreviewStartArgs) -> CliResult<()> {
    let mut progress = ProgressTracker::from_env("mirrord preview start");

    let layer_config = load_preview_config(&args, &mut progress)?;

    let operator_api = connect_to_operator(&layer_config, &mut progress).await?;

    let (preview, namespace) =
        create_preview_target(&operator_api, &args, &layer_config, &mut progress).await?;

    let ready_preview =
        match wait_for_preview_ready(&operator_api, &preview, &namespace, &mut progress).await {
            Ok(p) => p,
            Err(e) => {
                // Attempt to clean up the CRD on failure
                cleanup_preview(&operator_api, &preview, &namespace, &mut progress).await;
                return Err(e);
            }
        };

    display_preview_info(&ready_preview, layer_config.key.as_str(), &mut progress);

    Ok(())
}

/// Load and resolve the mirrord configuration for preview.
fn load_preview_config(
    args: &PreviewStartArgs,
    progress: &mut ProgressTracker,
) -> CliResult<LayerConfig> {
    let mut subtask = progress.subtask("loading configuration");

    let mut cfg_context = ConfigContext::default().override_envs(args.as_env_vars());

    let config = LayerConfig::resolve(&mut cfg_context).inspect_err(|error| {
        subtask.failure(Some(&format!("failed to read config: {error}")));
    })?;

    subtask.success(Some("configuration loaded"));
    Ok(config)
}

/// Connect to the mirrord operator.
///
/// The operator is required for preview environments.
async fn connect_to_operator(
    config: &LayerConfig,
    progress: &mut ProgressTracker,
) -> CliResult<OperatorApi<NoClientCert>> {
    let mut subtask = progress.subtask("connecting to operator");

    let operator_api = OperatorApi::try_new(config, &mut NullReporter::default(), progress)
        .await?
        .ok_or_else(|| {
            subtask.failure(Some("operator not found"));
            CliError::OperatorRequiredForPreview
        })?;

    operator_api.check_license_validity(progress)?;

    subtask.success(Some("connected to operator"));
    Ok(operator_api)
}

/// Create the PreviewTargetCrd resource in the cluster.
///
/// Returns the created CRD and the namespace it was created in.
async fn create_preview_target(
    operator_api: &OperatorApi<NoClientCert>,
    args: &PreviewStartArgs,
    config: &LayerConfig,
    progress: &mut ProgressTracker,
) -> CliResult<(PreviewTargetCrd, String)> {
    let mut subtask = progress.subtask("creating preview environment");

    let namespace = args
        .target_namespace
        .as_deref()
        .or(config.target.namespace.as_deref())
        .unwrap_or("default");

    let target = config.target.path.as_ref().ok_or_else(|| {
        subtask.failure(Some("target is required for preview environments"));
        CliError::PreviewCreationFailed("target is required for preview environments".to_owned())
    })?;

    let spec = PreviewTargetSpec {
        image: args.image.clone(),
        key: config.key.as_str().to_owned(),
        target: target.clone(),
        target_namespace: args.target_namespace.clone(),
    };

    let preview_name = format!("preview-{}", config.key.as_str());
    let preview = PreviewTargetCrd::new(&preview_name, spec);

    let api: Api<PreviewTargetCrd> = Api::namespaced(operator_api.client().clone(), namespace);

    let created = api
        .create(&PostParams::default(), &preview)
        .await
        .map_err(|e| {
            subtask.failure(Some(&format!("failed to create preview: {e}")));
            CliError::PreviewCreationFailed(e.to_string())
        })?;

    subtask.success(Some(&format!("preview '{}' created", preview_name)));

    Ok((created, namespace.to_owned()))
}

/// Wait for the preview to become ready using a watcher.
///
/// Returns the updated preview CRD with status populated.
async fn wait_for_preview_ready(
    operator_api: &OperatorApi<NoClientCert>,
    preview: &PreviewTargetCrd,
    namespace: &str,
    progress: &mut ProgressTracker,
) -> CliResult<PreviewTargetCrd> {
    let mut subtask = progress.subtask("waiting for preview to be ready");

    let api: Api<PreviewTargetCrd> = Api::namespaced(operator_api.client().clone(), namespace);

    let watcher_config = watcher::Config::default()
        .fields(&format!("metadata.name={}", preview.name_any()))
        .timeout(300); // 5 minutes

    let stream = watcher(api, watcher_config);
    pin!(stream);

    let initialization_start = Instant::now();
    let mut long_initialization_timer = interval(Duration::from_secs(20));
    // First tick is instant
    long_initialization_timer.tick().await;

    let mut last_known_phase: Option<String> = None;

    loop {
        tokio::select! {
            _ = long_initialization_timer.tick() => {
                subtask.warning(&format!(
                    "preview initialization is taking over {}s, phase: {}",
                    initialization_start.elapsed().as_secs(),
                    last_known_phase.as_deref().unwrap_or("unknown")
                ));
            }
            event = stream.next() => {
                match event {
                    Some(Ok(Event::Apply(current) | Event::InitApply(current))) => {
                        if let Some(status) = &current.status {
                            last_known_phase = status.phase.clone();

                            match status.phase.as_deref() {
                                Some(PreviewTargetStatus::PHASE_READY) => {
                                    subtask.success(Some("preview is ready"));
                                    return Ok(current);
                                }
                                Some(PreviewTargetStatus::PHASE_FAILED) => {
                                    let msg = status.failure_message.as_deref().unwrap_or("unknown error");
                                    subtask.failure(Some(&format!("preview creation failed: {msg}")));
                                    return Err(CliError::PreviewCreationFailed(msg.to_string()));
                                }
                                _ => {
                                    // Still in progress, continue watching
                                }
                            }
                        }
                    }

                    Some(Ok(Event::Delete(_))) => {
                        subtask.failure(Some("preview was unexpectedly deleted"));
                        return Err(CliError::PreviewCreationFailed(
                            "preview was unexpectedly deleted".to_string(),
                        ));
                    }

                    Some(Ok(Event::Init | Event::InitDone)) => continue,

                    Some(Err(error)) => {
                        subtask.failure(Some("watch stream failed"));
                        return Err(CliError::PreviewCreationFailed(format!(
                            "watch stream failed: {error}"
                        )));
                    }

                    None => {
                        subtask.failure(Some("preview creation timed out"));
                        return Err(CliError::PreviewTimeout);
                    }
                }
            }
        }
    }
}

/// Attempt to clean up a preview CRD after a failure.
///
/// This is best-effort and errors are logged but not propagated.
async fn cleanup_preview(
    operator_api: &OperatorApi<NoClientCert>,
    preview: &PreviewTargetCrd,
    namespace: &str,
    progress: &mut ProgressTracker,
) {
    let mut subtask = progress.subtask("cleaning up preview");

    let name = preview.name_any();
    let api: Api<PreviewTargetCrd> = Api::namespaced(operator_api.client().clone(), namespace);

    match api.delete(&name, &DeleteParams::default()).await {
        Ok(_) => {
            subtask.success(Some("preview cleaned up"));
        }
        Err(e) => {
            tracing::warn!("failed to clean up preview CRD: {e}");
            subtask.failure(Some(&format!("failed to clean up: {e}")));
        }
    }
}

/// Display information about the created preview environment.
fn display_preview_info(preview: &PreviewTargetCrd, key: &str, progress: &mut ProgressTracker) {
    let namespace = preview.namespace().unwrap_or_else(|| "default".to_owned());
    let name = preview.name_any();

    let pod = preview
        .status
        .as_ref()
        .and_then(|s| s.pod_name.as_deref())
        .unwrap_or("<unknown>");

    progress.success(Some(&format!(
        r#"
preview environment created successfully

- environment key: {key}
- namespace: {namespace}
- resource: {name}
- pod: {pod}
"#
    )));
}
