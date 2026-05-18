use porthole_core::{
    SurfaceId,
    agent_policy::{ActionClass, AgentId, AppSelector, Constraint, DurationSpec, GrantId, PermissionRequestId, TargetSelector},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAgentIdentityRequest {
    pub display_name: String,
    #[serde(default)]
    pub metadata: Option<AgentIdentityMetadata>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAgentIdentityResponse {
    pub agent_id: AgentId,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MintAgentTokenResponse {
    pub token_id: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentityResponse {
    pub agent_id: AgentId,
    pub display_name: String,
    pub metadata: Option<AgentIdentityMetadata>,
    pub created_at_unix_ms: u64,
    pub revoked_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTokenResponse {
    pub token_id: String,
    pub agent_id: AgentId,
    pub created_at_unix_ms: u64,
    pub revoked_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentityMetadata {
    pub bundle_id: Option<String>,
    pub executable_path: Option<String>,
    pub vendor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPermissionNeededDetails {
    pub request_id: PermissionRequestId,
    pub agent_id: AgentId,
    pub target: AgentPermissionTarget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_id: Option<SurfaceId>,
    pub actions: Vec<ActionClass>,
    pub recommended_duration: AgentPermissionDuration,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPermissionRequestResponse {
    pub request_id: PermissionRequestId,
    pub agent_id: AgentId,
    pub target: AgentPermissionTarget,
    pub actions: Vec<ActionClass>,
    pub reason: Option<String>,
    pub status: String,
    pub created_at_unix_ms: u64,
    pub resolved_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentGrantResponse {
    pub grant_id: GrantId,
    pub agent_id: AgentId,
    pub origin_request_id: Option<PermissionRequestId>,
    pub target: AgentPermissionTarget,
    pub actions: Vec<ActionClass>,
    pub duration: AgentPermissionDuration,
    pub constraints: AgentPermissionConstraints,
    pub created_at_unix_ms: u64,
    pub expires_at_unix_ms: Option<u64>,
    pub consumed_at_unix_ms: Option<u64>,
    pub revoked_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationResponse {
    pub revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentPermissionDuration {
    Once,
    UntilSurfaceGone,
    Session { session: String },
    TimeBounded { expires_at_unix_ms: u64 },
    Persistent,
}

impl From<DurationSpec> for AgentPermissionDuration {
    fn from(duration: DurationSpec) -> Self {
        match duration {
            DurationSpec::Once => Self::Once,
            DurationSpec::UntilSurfaceGone => Self::UntilSurfaceGone,
            DurationSpec::Session { session } => Self::Session { session },
            DurationSpec::TimeBounded { expires_at_unix_ms } => Self::TimeBounded { expires_at_unix_ms },
            DurationSpec::Persistent => Self::Persistent,
        }
    }
}

impl From<AgentPermissionDuration> for DurationSpec {
    fn from(duration: AgentPermissionDuration) -> Self {
        match duration {
            AgentPermissionDuration::Once => Self::Once,
            AgentPermissionDuration::UntilSurfaceGone => Self::UntilSurfaceGone,
            AgentPermissionDuration::Session { session } => Self::Session { session },
            AgentPermissionDuration::TimeBounded { expires_at_unix_ms } => Self::TimeBounded { expires_at_unix_ms },
            AgentPermissionDuration::Persistent => Self::Persistent,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentPermissionTarget {
    Surface { surface_id: SurfaceId },
    App { app: AgentPermissionAppSelector },
    LaunchedByAgent,
    FrontmostOnce { surface_id: SurfaceId },
    AllSurfaces,
}

impl From<TargetSelector> for AgentPermissionTarget {
    fn from(target: TargetSelector) -> Self {
        match target {
            TargetSelector::Surface { surface_id } => Self::Surface { surface_id },
            TargetSelector::App { app } => Self::App { app: app.into() },
            TargetSelector::LaunchedByAgent => Self::LaunchedByAgent,
            TargetSelector::FrontmostOnce { surface_id } => Self::FrontmostOnce { surface_id },
            TargetSelector::AllSurfaces => Self::AllSurfaces,
        }
    }
}

impl From<AgentPermissionTarget> for TargetSelector {
    fn from(target: AgentPermissionTarget) -> Self {
        match target {
            AgentPermissionTarget::Surface { surface_id } => Self::Surface { surface_id },
            AgentPermissionTarget::App { app } => Self::App { app: app.into() },
            AgentPermissionTarget::LaunchedByAgent => Self::LaunchedByAgent,
            AgentPermissionTarget::FrontmostOnce { surface_id } => Self::FrontmostOnce { surface_id },
            AgentPermissionTarget::AllSurfaces => Self::AllSurfaces,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentPermissionAppSelector {
    BundleId { bundle_id: String },
    ExecutablePath { executable_path: String },
    AppName { app_name: String },
}

impl From<AppSelector> for AgentPermissionAppSelector {
    fn from(app: AppSelector) -> Self {
        match app {
            AppSelector::BundleId(bundle_id) => Self::BundleId { bundle_id },
            AppSelector::ExecutablePath(executable_path) => Self::ExecutablePath { executable_path },
            AppSelector::AppName(app_name) => Self::AppName { app_name },
        }
    }
}

impl From<AgentPermissionAppSelector> for AppSelector {
    fn from(app: AgentPermissionAppSelector) -> Self {
        match app {
            AgentPermissionAppSelector::BundleId { bundle_id } => Self::BundleId(bundle_id),
            AgentPermissionAppSelector::ExecutablePath { executable_path } => Self::ExecutablePath(executable_path),
            AgentPermissionAppSelector::AppName { app_name } => Self::AppName(app_name),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPermissionConstraints {
    #[serde(default)]
    pub requires_frontmost: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_input: Vec<String>,
}

impl From<Vec<Constraint>> for AgentPermissionConstraints {
    fn from(constraints: Vec<Constraint>) -> Self {
        let mut wire = Self::default();
        for constraint in constraints {
            match constraint {
                Constraint::RequiresFrontmost => wire.requires_frontmost = true,
                Constraint::MaxDurationMs(value) => wire.max_duration_ms = Some(value),
                Constraint::AllowedInput(values) => wire.allowed_input = values,
            }
        }
        wire
    }
}

impl From<AgentPermissionConstraints> for Vec<Constraint> {
    fn from(constraints: AgentPermissionConstraints) -> Self {
        let mut core = Vec::new();
        if constraints.requires_frontmost {
            core.push(Constraint::RequiresFrontmost);
        }
        if let Some(max_duration_ms) = constraints.max_duration_ms {
            core.push(Constraint::MaxDurationMs(max_duration_ms));
        }
        if !constraints.allowed_input.is_empty() {
            core.push(Constraint::AllowedInput(constraints.allowed_input));
        }
        core
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproveAgentPermissionRequest {
    pub duration: AgentPermissionDuration,
    pub target: AgentPermissionTarget,
    pub actions: Vec<ActionClass>,
    #[serde(default)]
    pub constraints: AgentPermissionConstraints,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DenyAgentPermissionRequest {
    pub remember: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_identity_request_metadata_roundtrips() {
        let request = CreateAgentIdentityRequest {
            display_name: "Build Agent".into(),
            metadata: Some(AgentIdentityMetadata {
                bundle_id: Some("com.example.agent".into()),
                executable_path: None,
                vendor: Some("Example".into()),
            }),
        };

        let json = serde_json::to_string(&request).unwrap();
        let back: CreateAgentIdentityRequest = serde_json::from_str(&json).unwrap();

        assert_eq!(back, request);
        let empty_metadata: AgentIdentityMetadata = serde_json::from_str("{}").unwrap();
        assert_eq!(
            empty_metadata,
            AgentIdentityMetadata {
                bundle_id: None,
                executable_path: None,
                vendor: None,
            }
        );
    }

    #[test]
    fn create_identity_response_returns_agent_id_and_token() {
        let response = CreateAgentIdentityResponse {
            agent_id: AgentId::from("agent_1"),
            token: "pta_agent_1.secret".into(),
        };

        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(json["agent_id"], "agent_1");
        assert_eq!(json["token"], "pta_agent_1.secret");
        let back: CreateAgentIdentityResponse = serde_json::from_value(json).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn mint_agent_token_response_returns_token_id_and_plaintext_token() {
        let response = MintAgentTokenResponse {
            token_id: "tok_1".into(),
            token: "pta_agent_1.second".into(),
        };

        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(json["token_id"], "tok_1");
        assert_eq!(json["token"], "pta_agent_1.second");
        let back: MintAgentTokenResponse = serde_json::from_value(json).unwrap();
        assert_eq!(back, response);
    }

    #[test]
    fn permission_needed_details_roundtrip_with_nested_duration() {
        let details = AgentPermissionNeededDetails {
            request_id: PermissionRequestId::from("apr_1"),
            agent_id: AgentId::from("agent_1"),
            target: AgentPermissionTarget::Surface {
                surface_id: SurfaceId::from("surf_1"),
            },
            surface_id: Some(SurfaceId::from("surf_1")),
            actions: vec![ActionClass::Drive],
            recommended_duration: AgentPermissionDuration::UntilSurfaceGone,
        };

        let json = serde_json::to_value(&details).unwrap();

        assert_eq!(json["request_id"], "apr_1");
        assert_eq!(json["actions"], serde_json::json!(["drive"]));
        assert_eq!(json["recommended_duration"], serde_json::json!({ "type": "until_surface_gone" }));
        let back: AgentPermissionNeededDetails = serde_json::from_value(json).unwrap();
        assert_eq!(back, details);
    }

    #[test]
    fn approve_request_roundtrips_target_actions_duration_and_constraints() {
        let request = ApproveAgentPermissionRequest {
            duration: AgentPermissionDuration::UntilSurfaceGone,
            target: AgentPermissionTarget::Surface {
                surface_id: SurfaceId::from("surf_1"),
            },
            actions: vec![ActionClass::Drive],
            constraints: AgentPermissionConstraints {
                requires_frontmost: false,
                max_duration_ms: Some(5_000),
                allowed_input: vec!["text".into()],
            },
        };

        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(json["duration"], serde_json::json!({ "type": "until_surface_gone" }));
        assert_eq!(json["target"], serde_json::json!({ "type": "surface", "surface_id": "surf_1" }));
        assert_eq!(json["actions"], serde_json::json!(["drive"]));
        assert_eq!(
            json["constraints"],
            serde_json::json!({
                "requires_frontmost": false,
                "max_duration_ms": 5000,
                "allowed_input": ["text"]
            })
        );
        let back: ApproveAgentPermissionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, request);
    }

    #[test]
    fn deny_request_roundtrips_remember_and_reason() {
        let request = DenyAgentPermissionRequest {
            remember: false,
            reason: Some("not_now".into()),
        };

        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(json, serde_json::json!({ "remember": false, "reason": "not_now" }));
        let back: DenyAgentPermissionRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, request);
    }
}
