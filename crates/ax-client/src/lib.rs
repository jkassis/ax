//! Product-neutral AX transport for an admitted agent-session specification.

use agent_session_contract::{RouteProtocol, SessionSpec};
use anyhow::{Context, Result};
use std::{collections::BTreeMap, time::Duration};
use tonic::{transport::Channel, Code, Request};

mod wire {
    tonic::include_proto!("ax.v1alpha1");
}

use wire::{
    ax_client::AxClient, DeleteGatewayRequest, DeleteTaskRequest, EgressAllowlist, EgressConfig,
    EnvVar, Gateway, GatewayRef, GatewaySpec, GetTaskRequest, HostRule, Listener, ObjectMeta,
    ResourceList, ResourceReqs, Task, TaskSpec, UpdateGatewayRequest, UpdateTaskRequest,
    WorkspaceRef,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeLocator {
    pub atespace: String,
    pub task: String,
    pub actor: String,
}

impl RuntimeLocator {
    pub fn upstream(&self) -> String {
        format!("ax://{}/{}/", self.atespace, self.actor)
    }
}

#[derive(Clone)]
pub struct AgentAxClient {
    client: AxClient<Channel>,
    timeout: Duration,
}

impl AgentAxClient {
    pub fn connect_lazy(endpoint: String) -> Result<Self> {
        let endpoint = tonic::transport::Endpoint::from_shared(endpoint)?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(300));
        Ok(Self {
            client: AxClient::new(endpoint.connect_lazy()),
            timeout: Duration::from_secs(180),
        })
    }

    pub async fn ensure(
        &self,
        atespace: &str,
        task_name: &str,
        spec: &SessionSpec,
        runtime_env: BTreeMap<String, String>,
        egress_hosts: Vec<(String, u16)>,
    ) -> Result<RuntimeLocator> {
        spec.validate()?;
        validate_ax_name(atespace)?;
        validate_ax_name(task_name)?;
        let metadata = ObjectMeta {
            name: task_name.into(),
            atespace: atespace.into(),
            creation_timestamp: None,
        };
        let gateway = Gateway {
            api_version: "ax.io/v1alpha1".into(),
            kind: "Gateway".into(),
            metadata: Some(metadata.clone()),
            spec: Some(GatewaySpec {
                listeners: spec
                    .routes
                    .iter()
                    .map(|route| Listener {
                        name: route.port_name.clone(),
                        port: i32::from(route.port),
                        protocol: match route.protocol {
                            RouteProtocol::Http => "HTTP",
                            RouteProtocol::Grpc => "HTTP2",
                        }
                        .into(),
                    })
                    .collect(),
                egress: Some(EgressConfig {
                    allowlist: Some(EgressAllowlist {
                        hosts: egress_hosts
                            .into_iter()
                            .map(|(host, port)| HostRule {
                                host,
                                port: i32::from(port),
                            })
                            .collect(),
                    }),
                }),
            }),
        };
        let task = Task {
            api_version: "ax.io/v1alpha1".into(),
            kind: "Task".into(),
            metadata: Some(metadata),
            spec: Some(TaskSpec {
                suspend: false,
                image: spec.runtime.image.clone(),
                command: spec.runtime.command.clone(),
                env: runtime_env
                    .into_iter()
                    .map(|(name, value)| EnvVar { name, value })
                    .collect(),
                resources: Some(ResourceReqs {
                    requests: Some(ResourceList {
                        cpu: spec.runtime.resources.request_cpu.clone(),
                        memory: spec.runtime.resources.request_memory.clone(),
                    }),
                    limits: Some(ResourceList {
                        cpu: spec.runtime.resources.limit_cpu.clone(),
                        memory: spec.runtime.resources.limit_memory.clone(),
                    }),
                }),
                workspaces: spec
                    .workspaces
                    .iter()
                    .map(|workspace| WorkspaceRef {
                        name: workspace.name.clone(),
                        path: workspace.mount_path.clone(),
                        goal: String::new(),
                    })
                    .collect(),
                gateway: Some(GatewayRef {
                    name: task_name.into(),
                }),
                debug: false,
                working_directory: String::new(),
            }),
            status: None,
        };
        let mut client = self.client.clone();
        client
            .update_gateway(self.request(UpdateGatewayRequest {
                gateway: Some(gateway),
            }))
            .await
            .context("apply AX gateway")?;
        match client
            .get_task(self.request(GetTaskRequest {
                atespace: atespace.into(),
                name: task_name.into(),
            }))
            .await
        {
            Ok(response) => {
                let existing = response.into_inner();
                if let Some(status) = existing.status.as_ref() {
                    if status.phase == "Running" && validate_ax_name(&status.actor).is_ok() {
                        return Ok(RuntimeLocator {
                            atespace: atespace.into(),
                            task: task_name.into(),
                            actor: status.actor.clone(),
                        });
                    }
                }
                client
                    .delete_task(self.request(DeleteTaskRequest {
                        atespace: atespace.into(),
                        name: task_name.into(),
                    }))
                    .await
                    .context("delete prior AX task")?;
                self.await_absent(atespace, task_name).await?;
            }
            Err(status) if status.code() == Code::NotFound => {}
            Err(status) => return Err(status).context("inspect AX task"),
        }
        let mut current = match client
            .update_task(self.request(UpdateTaskRequest { task: Some(task) }))
            .await
        {
            Ok(response) => response.into_inner(),
            Err(status) if uncertain_mutation(status.code()) => {
                self.observe_uncertain_task_update(&mut client, atespace, task_name, status)
                    .await?
            }
            Err(status) => return Err(status).context("apply AX task"),
        };
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let status = current.status.as_ref().context("AX task omitted status")?;
            if status.phase == "Running" {
                validate_ax_name(&status.actor)?;
                return Ok(RuntimeLocator {
                    atespace: atespace.into(),
                    task: task_name.into(),
                    actor: status.actor.clone(),
                });
            }
            if status.phase == "Failed" {
                anyhow::bail!("AX task entered failed phase");
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("AX task did not reach running before deadline");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            current = tokio::time::timeout_at(
                deadline,
                client.get_task(self.request(GetTaskRequest {
                    atespace: atespace.into(),
                    name: task_name.into(),
                })),
            )
            .await
            .context("AX readiness observation exceeded deadline")?
            .context("observe AX task")?
            .into_inner();
        }
    }

    pub async fn remove(&self, atespace: &str, task_name: &str) -> Result<()> {
        validate_ax_name(atespace)?;
        validate_ax_name(task_name)?;
        let mut client = self.client.clone();
        if let Err(status) = client
            .delete_task(self.request(DeleteTaskRequest {
                atespace: atespace.into(),
                name: task_name.into(),
            }))
            .await
        {
            if status.code() != Code::NotFound {
                return Err(status).context("delete AX task");
            }
        }
        if let Err(status) = client
            .delete_gateway(self.request(DeleteGatewayRequest {
                atespace: atespace.into(),
                name: task_name.into(),
            }))
            .await
        {
            if status.code() != Code::NotFound {
                return Err(status).context("delete AX gateway");
            }
        }
        Ok(())
    }

    async fn await_absent(&self, atespace: &str, task_name: &str) -> Result<()> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        let mut client = self.client.clone();
        loop {
            let observation = tokio::time::timeout_at(
                deadline,
                client.get_task(self.request(GetTaskRequest {
                    atespace: atespace.into(),
                    name: task_name.into(),
                })),
            )
            .await
            .context("AX task-removal observation exceeded deadline")?;
            match observation {
                Err(status) if status.code() == Code::NotFound => return Ok(()),
                Ok(_) => {}
                Err(status) => return Err(status).context("await AX task removal"),
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("prior AX task was not removed before deadline");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn observe_uncertain_task_update(
        &self,
        client: &mut AxClient<Channel>,
        atespace: &str,
        task_name: &str,
        submission_error: tonic::Status,
    ) -> Result<Task> {
        let deadline = tokio::time::Instant::now() + self.timeout;
        loop {
            let observation = match tokio::time::timeout_at(
                deadline,
                client.get_task(self.request(GetTaskRequest {
                    atespace: atespace.into(),
                    name: task_name.into(),
                })),
            )
            .await
            {
                Ok(observation) => observation,
                Err(_) => {
                    return Err(submission_error)
                        .context("AX task update outcome remained uncertain before deadline")
                }
            };
            match classify_uncertain_observation(observation) {
                Ok(Some(task)) => return Ok(task),
                Ok(None) => {}
                Err(status) => return Err(status).context("observe uncertain AX task update"),
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(submission_error)
                    .context("AX task update outcome remained uncertain before deadline");
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    fn request<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request.set_timeout(self.timeout);
        request
    }
}

fn uncertain_mutation(code: Code) -> bool {
    matches!(
        code,
        Code::Cancelled | Code::Unknown | Code::DeadlineExceeded | Code::Unavailable
    )
}

fn classify_uncertain_observation(
    observation: std::result::Result<tonic::Response<Task>, tonic::Status>,
) -> std::result::Result<Option<Task>, tonic::Status> {
    match observation {
        Ok(response) => Ok(Some(response.into_inner())),
        Err(status) if status.code() == Code::NotFound || uncertain_mutation(status.code()) => {
            Ok(None)
        }
        Err(status) => Err(status),
    }
}

fn validate_ax_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 63
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        anyhow::bail!("invalid AX name");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_transport_names_and_locator() {
        assert!(validate_ax_name("coach-0199d9d2").is_ok());
        assert!(validate_ax_name("Coach/unsafe").is_err());
        let locator = RuntimeLocator {
            atespace: "coach-user".into(),
            task: "session-one".into(),
            actor: "actor-one".into(),
        };
        assert_eq!(locator.upstream(), "ax://coach-user/actor-one/");
    }

    #[test]
    fn classifies_only_ambiguous_transport_failures_as_uncertain() {
        for code in [
            Code::Cancelled,
            Code::Unknown,
            Code::DeadlineExceeded,
            Code::Unavailable,
        ] {
            assert!(uncertain_mutation(code), "{code:?}");
        }
        for code in [
            Code::InvalidArgument,
            Code::NotFound,
            Code::AlreadyExists,
            Code::FailedPrecondition,
            Code::PermissionDenied,
            Code::Unauthenticated,
        ] {
            assert!(!uncertain_mutation(code), "{code:?}");
        }
    }

    #[test]
    fn uncertain_start_observation_converges_on_the_committed_named_task() {
        let committed = Task {
            api_version: "ax.io/v1alpha1".into(),
            kind: "Task".into(),
            metadata: Some(ObjectMeta {
                name: "coach-session".into(),
                atespace: "coach-user".into(),
                creation_timestamp: None,
            }),
            spec: None,
            status: None,
        };
        let observations = [
            Err(tonic::Status::not_found("not visible yet")),
            Err(tonic::Status::unavailable("retry observation")),
            Ok(tonic::Response::new(committed.clone())),
        ];

        let mut resolved = None;
        for observation in observations {
            if let Some(task) = classify_uncertain_observation(observation).unwrap() {
                resolved = Some(task);
                break;
            }
        }
        assert_eq!(resolved, Some(committed));

        let terminal =
            classify_uncertain_observation(Err(tonic::Status::permission_denied("terminal")))
                .unwrap_err();
        assert_eq!(terminal.code(), Code::PermissionDenied);
    }
}
