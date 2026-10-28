//! Product-neutral agent-session specification and lifecycle contract.
//!
//! Products translate their authorized resources into this contract. This
//! crate deliberately contains no product identifiers, database access, or
//! browser policy.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub const SESSION_SCHEMA_VERSION: u32 = 1;
pub const SESSION_LEASE_SECONDS: u64 = 2 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSpec {
    pub schema_version: u32,
    pub session_id: Uuid,
    pub room_id: Uuid,
    pub lease_id: Uuid,
    pub product: String,
    pub product_resource_ref: String,
    pub principal: String,
    pub lease_seconds: u64,
    pub generation: u64,
    pub fencing_token: Uuid,
    pub runtime: NativeRuntimeSpec,
    pub workspaces: Vec<WorkspaceAttachment>,
    pub routes: Vec<NamedRoute>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRuntimeSpec {
    pub image: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub resources: RuntimeResources,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeResources {
    pub request_cpu: String,
    pub request_memory: String,
    pub limit_cpu: String,
    pub limit_memory: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceAttachment {
    pub name: String,
    pub source: String,
    pub mount_path: String,
    pub access: WorkspaceAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_policy: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedRoute {
    pub name: String,
    pub port_name: String,
    pub port: u16,
    pub protocol: RouteProtocol,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RouteProtocol {
    #[serde(rename = "HTTP")]
    Http,
    #[serde(rename = "gRPC")]
    Grpc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatus {
    pub session_id: Uuid,
    pub room_id: Uuid,
    pub lease_id: Uuid,
    pub phase: SessionPhase,
    pub generation: u64,
    pub fencing_token: Uuid,
    pub spec_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default)]
    pub observations: Vec<RuntimeObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    Requested,
    Starting,
    Active,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeObservation {
    pub kind: String,
    pub status: String,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservedRuntime {
    Missing,
    Starting,
    RunningCompatible,
    RunningStale,
    SuspendedCompatible,
    SuspendedStale,
    Crashed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconcileAction {
    Ensure,
    Wait,
    Reuse,
    Resume,
    Replace,
    Remove,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnsureSessionRequest {
    pub idempotency_key: String,
    pub request_id: String,
    pub spec: SessionSpec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnsureDecision {
    Create,
    Replay,
    Conflict,
    NewGeneration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedRouteResolution {
    pub schema_version: u32,
    pub request_id: String,
    pub session_id: Uuid,
    pub lease_id: Uuid,
    pub generation: u64,
    pub fencing_token: Uuid,
    pub audience: String,
    pub route_name: String,
    pub upstream: String,
}

impl SessionSpec {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SESSION_SCHEMA_VERSION {
            bail!("unsupported session schema version")
        }
        if self.session_id.is_nil()
            || self.room_id.is_nil()
            || self.lease_id.is_nil()
            || self.fencing_token.is_nil()
            || self.generation == 0
        {
            bail!("session, room, lease, generation, and fencing identities are required")
        }
        validate_name(&self.product)?;
        if !valid_opaque_ref(&self.product_resource_ref)
            || self.principal.trim().is_empty()
            || self.lease_seconds != SESSION_LEASE_SECONDS
        {
            bail!("product, opaque resource reference, principal, and two-hour lease are required")
        }
        if !is_digest_pinned_image(&self.runtime.image) || self.runtime.command.is_empty() {
            bail!("runtime requires a digest-pinned image and command")
        }
        let mut workspace_names = BTreeSet::new();
        let mut writable_paths = Vec::new();
        for workspace in &self.workspaces {
            validate_name(&workspace.name)?;
            if !workspace_names.insert(workspace.name.as_str())
                || !valid_source(&workspace.source)
                || !valid_mount_path(&workspace.mount_path)
            {
                bail!("invalid or duplicate workspace attachment")
            }
            if workspace.access == WorkspaceAccess::ReadWrite {
                if writable_paths
                    .iter()
                    .any(|path: &&str| paths_overlap(path, &workspace.mount_path))
                {
                    bail!("writable workspace mount paths overlap")
                }
                writable_paths.push(workspace.mount_path.as_str());
            }
        }
        let mut route_names = BTreeSet::new();
        let mut ports = BTreeSet::new();
        for route in &self.routes {
            validate_name(&route.name)?;
            validate_name(&route.port_name)?;
            if route.port < 1024
                || !route_names.insert(route.name.as_str())
                || !ports.insert(route.port)
            {
                bail!("routes require unique names and unprivileged ports")
            }
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

impl EnsureSessionRequest {
    pub fn validate(&self) -> Result<()> {
        if self.idempotency_key.is_empty()
            || self.idempotency_key.len() > 128
            || !self.idempotency_key.is_ascii()
            || self.request_id.is_empty()
            || self.request_id.len() > 128
            || !self.request_id.is_ascii()
        {
            bail!("bounded ASCII idempotency and request identifiers are required")
        }
        self.spec.validate()
    }
}

pub fn ensure_decision(
    existing_key: Option<&str>,
    existing_digest: Option<&str>,
    existing_generation: Option<u64>,
    requested: &EnsureSessionRequest,
) -> Result<EnsureDecision> {
    requested.validate()?;
    let digest = requested.spec.digest()?;
    match (existing_key, existing_digest, existing_generation) {
        (None, None, None) => Ok(EnsureDecision::Create),
        (Some(key), Some(prior), Some(generation)) if key == requested.idempotency_key => {
            if prior == digest && generation == requested.spec.generation {
                Ok(EnsureDecision::Replay)
            } else {
                Ok(EnsureDecision::Conflict)
            }
        }
        (Some(_), Some(prior), Some(generation))
            if prior != digest && requested.spec.generation > generation =>
        {
            Ok(EnsureDecision::NewGeneration)
        }
        _ => Ok(EnsureDecision::Conflict),
    }
}

impl NamedRouteResolution {
    pub fn validate_for(
        &self,
        session_id: Uuid,
        audience: &str,
        route_name: &str,
        minimum_generation: u64,
        fencing_token: Uuid,
    ) -> Result<()> {
        if self.schema_version != SESSION_SCHEMA_VERSION
            || self.session_id != session_id
            || self.lease_id.is_nil()
            || self.generation < minimum_generation
            || self.fencing_token != fencing_token
            || self.audience != audience
            || self.route_name != route_name
            || self.request_id.is_empty()
        {
            bail!("named route resolution does not match the live session authority")
        }
        validate_name(&self.route_name)?;
        if !valid_upstream(&self.upstream) {
            bail!("named route resolution contains an invalid upstream")
        }
        Ok(())
    }
}

pub fn reconcile_action(desired_active: bool, observed: ObservedRuntime) -> ReconcileAction {
    if !desired_active {
        return match observed {
            ObservedRuntime::Missing => ReconcileAction::None,
            _ => ReconcileAction::Remove,
        };
    }
    match observed {
        ObservedRuntime::Missing => ReconcileAction::Ensure,
        ObservedRuntime::Starting => ReconcileAction::Wait,
        ObservedRuntime::RunningCompatible => ReconcileAction::Reuse,
        ObservedRuntime::SuspendedCompatible => ReconcileAction::Resume,
        ObservedRuntime::RunningStale
        | ObservedRuntime::SuspendedStale
        | ObservedRuntime::Crashed => ReconcileAction::Replace,
    }
}

fn is_digest_pinned_image(image: &str) -> bool {
    let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
        return false;
    };
    !repository.is_empty()
        && digest.len() == 64
        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_source(source: &str) -> bool {
    matches!(
      source.split_once("://"),
      Some(("pvc" | "git" | "snapshot" | "s3" | "ephemeral", value)) if !value.is_empty()
    )
}

fn valid_opaque_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.is_ascii()
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
}

fn valid_upstream(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("ax://")
}

fn valid_mount_path(path: &str) -> bool {
    path.starts_with("/workspace/")
        && !path.ends_with('/')
        && !path.split('/').any(|part| part == "." || part == "..")
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn validate_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 63
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        bail!("invalid DNS-label name")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SessionSpec {
        SessionSpec {
            schema_version: SESSION_SCHEMA_VERSION,
            session_id: Uuid::now_v7(),
            room_id: Uuid::now_v7(),
            lease_id: Uuid::now_v7(),
            product: "fixture-product".into(),
            product_resource_ref: "resource:opaque-123".into(),
            principal: "subject:pairwise-123".into(),
            lease_seconds: SESSION_LEASE_SECONDS,
            generation: 1,
            fencing_token: Uuid::now_v7(),
            runtime: NativeRuntimeSpec {
                image: format!(
                    "registry.example/product-development@sha256:{}",
                    "a".repeat(64)
                ),
                command: vec!["/opt/product/start-development-session".into()],
                env: BTreeMap::new(),
                resources: RuntimeResources {
                    request_cpu: "500m".into(),
                    request_memory: "1Gi".into(),
                    limit_cpu: "2".into(),
                    limit_memory: "2Gi".into(),
                },
            },
            workspaces: vec![
                WorkspaceAttachment {
                    name: "source".into(),
                    source: "pvc://projects/test/source".into(),
                    mount_path: "/workspace/project".into(),
                    access: WorkspaceAccess::ReadWrite,
                    lifecycle_policy: None,
                },
                WorkspaceAttachment {
                    name: "agent-state".into(),
                    source: "snapshot://rooms/test".into(),
                    mount_path: "/workspace/agent-state".into(),
                    access: WorkspaceAccess::ReadWrite,
                    lifecycle_policy: Some("retain".into()),
                },
            ],
            routes: vec![
                NamedRoute {
                    name: "agent".into(),
                    port_name: "agent".into(),
                    port: 3030,
                    protocol: RouteProtocol::Http,
                },
                NamedRoute {
                    name: "application".into(),
                    port_name: "application".into(),
                    port: 4173,
                    protocol: RouteProtocol::Http,
                },
            ],
        }
    }

    #[test]
    fn accepts_ordered_multiple_nonidentical_attachments_and_named_routes() {
        let value = spec();
        value.validate().unwrap();
        let encoded = serde_json::to_string(&value).unwrap();
        let decoded: SessionSpec = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, value);
        assert_eq!(decoded.workspaces[0].name, "source");
        assert_eq!(decoded.workspaces[1].name, "agent-state");
        assert_ne!(decoded.digest().unwrap(), "");
    }

    #[test]
    fn rejects_overlapping_writable_mounts_and_mutable_images() {
        let mut value = spec();
        value.workspaces[1].mount_path = "/workspace/project/cache".into();
        assert!(value
            .validate()
            .unwrap_err()
            .to_string()
            .contains("overlap"));
        value.workspaces[1].mount_path = "/workspace/agent-state".into();
        value.runtime.image = "registry.example/product-development:latest".into();
        assert!(value
            .validate()
            .unwrap_err()
            .to_string()
            .contains("digest-pinned"));
    }

    #[test]
    fn lifecycle_converges_idempotently_and_replaces_stale_or_crashed_tasks() {
        assert_eq!(
            reconcile_action(true, ObservedRuntime::Missing),
            ReconcileAction::Ensure
        );
        assert_eq!(
            reconcile_action(true, ObservedRuntime::Starting),
            ReconcileAction::Wait
        );
        assert_eq!(
            reconcile_action(true, ObservedRuntime::RunningCompatible),
            ReconcileAction::Reuse
        );
        assert_eq!(
            reconcile_action(true, ObservedRuntime::SuspendedCompatible),
            ReconcileAction::Resume
        );
        for state in [
            ObservedRuntime::RunningStale,
            ObservedRuntime::SuspendedStale,
            ObservedRuntime::Crashed,
        ] {
            assert_eq!(reconcile_action(true, state), ReconcileAction::Replace);
        }
        assert_eq!(
            reconcile_action(false, ObservedRuntime::RunningCompatible),
            ReconcileAction::Remove
        );
        assert_eq!(
            reconcile_action(false, ObservedRuntime::Missing),
            ReconcileAction::None
        );
    }

    #[test]
    fn session_contract_contains_no_git_or_product_usability_gate() {
        let encoded = serde_json::to_value(spec()).unwrap();
        assert!(encoded.get("gitRefresh").is_none());
        assert!(encoded.get("usable").is_none());
        assert!(encoded.get("componentMounted").is_none());
    }

    #[test]
    fn ensure_is_idempotent_and_changed_specs_require_a_new_generation() {
        let first = EnsureSessionRequest {
            idempotency_key: "ensure-1".into(),
            request_id: "request-1".into(),
            spec: spec(),
        };
        let digest = first.spec.digest().unwrap();
        assert_eq!(
            ensure_decision(None, None, None, &first).unwrap(),
            EnsureDecision::Create
        );
        assert_eq!(
            ensure_decision(Some("ensure-1"), Some(&digest), Some(1), &first).unwrap(),
            EnsureDecision::Replay
        );
        let mut changed = first.clone();
        changed.idempotency_key = "ensure-2".into();
        changed.spec.generation = 2;
        changed.spec.fencing_token = Uuid::now_v7();
        assert_eq!(
            ensure_decision(Some("ensure-1"), Some(&digest), Some(1), &changed).unwrap(),
            EnsureDecision::NewGeneration
        );
        changed.spec.generation = 1;
        assert_eq!(
            ensure_decision(Some("ensure-1"), Some(&digest), Some(1), &changed).unwrap(),
            EnsureDecision::Conflict
        );
    }

    #[test]
    fn named_routes_reject_wrong_audience_name_generation_and_fence() {
        let value = spec();
        let route = NamedRouteResolution {
            schema_version: SESSION_SCHEMA_VERSION,
            request_id: "request-1".into(),
            session_id: value.session_id,
            lease_id: value.lease_id,
            generation: value.generation,
            fencing_token: value.fencing_token,
            audience: "session.example.test".into(),
            route_name: "agent".into(),
            upstream: "ax://space/actor/".into(),
        };
        route
            .validate_for(
                value.session_id,
                "session.example.test",
                "agent",
                1,
                value.fencing_token,
            )
            .unwrap();
        assert!(route
            .validate_for(
                value.session_id,
                "other.example.test",
                "agent",
                1,
                value.fencing_token
            )
            .is_err());
        assert!(route
            .validate_for(
                value.session_id,
                "session.example.test",
                "application",
                1,
                value.fencing_token
            )
            .is_err());
        assert!(route
            .validate_for(
                value.session_id,
                "session.example.test",
                "agent",
                2,
                value.fencing_token
            )
            .is_err());
        assert!(route
            .validate_for(
                value.session_id,
                "session.example.test",
                "agent",
                1,
                Uuid::now_v7()
            )
            .is_err());
    }
}
