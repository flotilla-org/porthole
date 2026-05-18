use porthole_core::{
    SurfaceId,
    agent_policy::{ActionClass, AgentId, PermissionRequestId},
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
pub struct AgentIdentityMetadata {
    pub bundle_id: Option<String>,
    pub executable_path: Option<String>,
    pub vendor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPermissionNeededDetails {
    pub request_id: PermissionRequestId,
    pub agent_id: AgentId,
    pub surface_id: SurfaceId,
    pub actions: Vec<ActionClass>,
    pub recommended_duration: AgentPermissionDuration,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentPermissionTarget {
    Surface { surface_id: SurfaceId },
    App { app: AgentPermissionAppSelector },
    LaunchedByAgent,
    FrontmostOnce { surface_id: SurfaceId },
    AllSurfaces,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentPermissionAppSelector {
    BundleId { bundle_id: String },
    ExecutablePath { executable_path: String },
    AppName { app_name: String },
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
            surface_id: SurfaceId::from("surf_1"),
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
