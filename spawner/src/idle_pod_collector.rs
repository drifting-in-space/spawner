use futures::{FutureExt, StreamExt};
use k8s_openapi::api::core::v1::Pod;
use kube::{
    api::ListParams,
    runtime::{
        controller::{Context, ReconcilerAction},
        Controller,
    },
    Api, Client, ResourceExt,
};
use std::{fmt::Display, future::Future, pin::Pin, time::Duration};

use crate::{kubernetes::delete_pod, pod_state::get_pod_state, SpawnerState};

#[derive(Debug)]
enum IdlePodCollectorError {
    ErrorCheckingStatus,
    ErrorDeletingPod,
}

impl std::error::Error for IdlePodCollectorError {}

impl Display for IdlePodCollectorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub struct IdlePodCollector;

impl IdlePodCollector {
    pub async fn new(state: SpawnerState) -> (Self, Pin<Box<dyn Future<Output = ()>>>) {
        let client = Client::try_default()
            .await
            .expect("Couldn't create kube client.");

        let pods = Api::<Pod>::namespaced(client, &state.namespace);

        let context = Context::new(state);

        let drainer = Controller::new(pods, ListParams::default())
            .run(reconcile, error_policy, context)
            .for_each(|_| futures::future::ready(()))
            .boxed();

        (IdlePodCollector, drainer)
    }
}

async fn reconcile(
    pod: Pod,
    ctx: Context<SpawnerState>,
) -> Result<ReconcilerAction, IdlePodCollectorError> {
    tracing::info!("reconcile called for pod: {}", pod.name());
    let ctx = ctx.get_ref();

    let name = pod.name();
    let pod_state = get_pod_state(&name, &ctx.namespace, ctx.application_port)
        .await
        .map_err(|_| IdlePodCollectorError::ErrorCheckingStatus)?;

    let seconds_until_expired = ctx.cleanup_frequency_seconds as i32 - pod_state.seconds_inactive as i32;

    if seconds_until_expired <= 0 {
        delete_pod(&name, &ctx.namespace)
            .await
            .map_err(|_| IdlePodCollectorError::ErrorDeletingPod)?;
    }

    Ok(ReconcilerAction {
        requeue_after: Some(Duration::from_secs(seconds_until_expired as u64)),
    })
}

fn error_policy(error: &IdlePodCollectorError, _ctx: Context<SpawnerState>) -> ReconcilerAction {
    tracing::warn!("Encountered error; retrying. {:?}", error);

    ReconcilerAction {
        requeue_after: Some(Duration::from_secs(360)),
    }
}