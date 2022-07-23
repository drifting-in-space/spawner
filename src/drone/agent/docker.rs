use super::DockerOptions;
use anyhow::{anyhow, Result};
use bollard::{
    auth::DockerCredentials,
    container::{
        Config, CreateContainerOptions, LogOutput, LogsOptions, StartContainerOptions, Stats,
        StatsOptions, StopContainerOptions,
    },
    image::CreateImageOptions,
    models::{EventMessage, HostConfig, PortBinding},
    system::EventsOptions,
    Docker, API_DEFAULT_VERSION,
};
use std::collections::HashMap;
use tokio_stream::{Stream, StreamExt};

/// The port in the container which is exposed.
const CONTAINER_PORT: u16 = 8080;
const DEFAULT_DOCKER_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_DOCKER_THROTTLED_STATS_INTERVAL_SECS: u64 = 10;

#[derive(Clone)]
pub struct DockerInterface {
    docker: Docker,
    runtime: Option<String>,
}

/// The list of possible container events.
/// Comes from [Docker documentation](https://docs.docker.com/engine/reference/commandline/events/).
#[derive(Debug, PartialEq, Eq)]
pub enum ContainerEventType {
    Attach,
    Commit,
    Copy,
    Create,
    Destroy,
    Detach,
    Die,
    ExecCreate,
    ExecDetach,
    ExecDie,
    ExecStart,
    Export,
    HealthStatus,
    Kill,
    Oom,
    Pause,
    Rename,
    Resize,
    Restart,
    Start,
    Stop,
    Top,
    Unpause,
    Update,
}

#[allow(unused)]
#[derive(Debug)]
pub struct ContainerEvent {
    pub event: ContainerEventType,
    pub name: String,
}

impl ContainerEvent {
    pub fn from_event_message(event: &EventMessage) -> Option<Self> {
        let action = event.action.as_deref()?;
        let actor = event.actor.as_ref()?;
        let name: String = actor.attributes.as_ref()?.get("name")?.to_string();

        let event = match action {
            "attach" => ContainerEventType::Attach,
            "commit" => ContainerEventType::Commit,
            "copy" => ContainerEventType::Copy,
            "create" => ContainerEventType::Create,
            "destroy" => ContainerEventType::Destroy,
            "detach" => ContainerEventType::Detach,
            "die" => ContainerEventType::Die,
            "exec_create" => ContainerEventType::ExecCreate,
            "exec_detach" => ContainerEventType::ExecDetach,
            "exec_die" => ContainerEventType::ExecDie,
            "exec_start" => ContainerEventType::ExecStart,
            "export" => ContainerEventType::Export,
            "health_status" => ContainerEventType::HealthStatus,
            "kill" => ContainerEventType::Kill,
            "oom" => ContainerEventType::Oom,
            "pause" => ContainerEventType::Pause,
            "rename" => ContainerEventType::Rename,
            "resize" => ContainerEventType::Resize,
            "restart" => ContainerEventType::Restart,
            "start" => ContainerEventType::Start,
            "stop" => ContainerEventType::Stop,
            "top" => ContainerEventType::Top,
            "unpause" => ContainerEventType::Unpause,
            "update" => ContainerEventType::Update,
            _ => {
                tracing::info!(?action, "Unhandled container action.");
                return None;
            }
        };

        Some(ContainerEvent { event, name })
    }
}

fn make_exposed_ports(port: u16) -> Option<HashMap<String, HashMap<(), ()>>> {
    let dummy: HashMap<(), ()> = vec![].into_iter().collect();
    Some(vec![(format!("{}/tcp", port), dummy)].into_iter().collect())
}

impl DockerInterface {
    pub async fn try_new(config: &DockerOptions) -> Result<Self> {
        let docker = match &config.transport {
            super::DockerApiTransport::Socket(docker_socket) => Docker::connect_with_unix(
                docker_socket,
                DEFAULT_DOCKER_TIMEOUT_SECONDS,
                API_DEFAULT_VERSION,
            )?,
            super::DockerApiTransport::Http(docker_http) => Docker::connect_with_http(
                docker_http,
                DEFAULT_DOCKER_TIMEOUT_SECONDS,
                API_DEFAULT_VERSION,
            )?,
        };

        Ok(DockerInterface {
            docker,
            runtime: config.runtime.clone(),
        })
    }

    pub async fn container_events(&self) -> impl Stream<Item = ContainerEvent> {
        let options: EventsOptions<&str> = EventsOptions {
            since: None,
            until: None,
            filters: vec![("type", vec!["container"])].into_iter().collect(),
        };
        self.docker
            .events(Some(options))
            .filter_map(|event| match event {
                Ok(event) => ContainerEvent::from_event_message(&event),
                Err(error) => {
                    tracing::error!(?error, "Error tracking container terminations.");
                    None
                }
            })
    }

    pub fn get_logs(
        &self,
        container_name: &str,
    ) -> impl Stream<Item = Result<LogOutput, bollard::errors::Error>> {
        self.docker.logs(
            container_name,
            Some(LogsOptions {
                follow: true,
                stdout: true,
                stderr: true,
                since: 0,
                until: 0,
                timestamps: true,
                tail: "all",
            }),
        )
    }

    pub fn get_stats(
        &self,
        container_name: &str,
    ) -> impl Stream<Item = Result<Stats, bollard::errors::Error>> {
        let options = StatsOptions {
            stream: true,
            one_shot: false,
        };
        self.docker
            .stats(container_name, Some(options))
            .throttle(std::time::Duration::from_secs(
                DEFAULT_DOCKER_THROTTLED_STATS_INTERVAL_SECS,
            ))
    }

    #[allow(unused)]
    pub async fn pull_image(
        &self,
        image: &str,
        credentials: &Option<DockerCredentials>,
    ) -> Result<()> {
        let options = Some(CreateImageOptions {
            from_image: image,
            ..Default::default()
        });

        let mut result = self.docker.create_image(options, None, credentials.clone());
        while let Some(next) = result.next().await {
            next?;
        }

        Ok(())
    }

    pub async fn stop_container(&self, name: &str) -> Result<()> {
        let options = StopContainerOptions { t: 10 };

        self.docker.stop_container(name, Some(options)).await?;

        Ok(())
    }

    pub async fn is_running(&self, container_name: &str) -> Result<(bool, Option<i64>)> {
        let container = match self.docker.inspect_container(container_name, None).await {
            Ok(container) => container,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 404, ..
            }) => return Ok((false, None)),
            Err(err) => return Err(err.into()),
        };
        let state = container
            .state
            .ok_or_else(|| anyhow!("No state found for container."))?;

        let running = state
            .running
            .ok_or_else(|| anyhow!("State found but no running field for container."))?;

        let exit_code = if running { None } else { state.exit_code };

        Ok((running, exit_code))
    }

    pub async fn get_port(&self, container_name: &str) -> Option<u16> {
        let inspect = self
            .docker
            .inspect_container(container_name, None)
            .await
            .ok()?;

        let port = inspect
            .network_settings
            .as_ref()?
            .ports
            .as_ref()?
            .get(&format!("{}/tcp", CONTAINER_PORT))?
            .as_ref()?
            .first()?
            .host_port
            .as_ref()?;

        port.parse().ok()
    }

    /// Run the specified image and return the name of the created container.
    pub async fn run_container(
        &self,
        name: &str,
        image: &str,
        env: &HashMap<String, String>,
    ) -> Result<()> {
        let env: Vec<String> = env.iter().map(|(k, v)| format!("{}={}", k, v)).collect();

        // Build the container.
        let container_id = {
            let options: Option<CreateContainerOptions<String>> = Some(CreateContainerOptions {
                name: name.to_string(),
            });

            let config: Config<String> = Config {
                image: Some(image.to_string()),
                env: Some(env),
                exposed_ports: make_exposed_ports(CONTAINER_PORT),
                labels: Some(
                    vec![
                        ("dev.spawner.managed".to_string(), "true".to_string()),
                        ("dev.spawner.backend".to_string(), name.to_string()),
                    ]
                    .into_iter()
                    .collect(),
                ),
                host_config: Some(HostConfig {
                    port_bindings: Some(
                        vec![(
                            format!("{}/tcp", CONTAINER_PORT),
                            Some(vec![PortBinding {
                                host_ip: None,
                                host_port: Some("0".to_string()),
                            }]),
                        )]
                        .into_iter()
                        .collect(),
                    ),
                    runtime: self.runtime.clone(),
                    ..HostConfig::default()
                }),
                ..Config::default()
            };

            let result = self.docker.create_container(options, config).await?;
            result.id
        };

        // Start the container.
        {
            let options: Option<StartContainerOptions<&str>> = None;

            self.docker.start_container(&container_id, options).await?;
        };

        Ok(())
    }
}
