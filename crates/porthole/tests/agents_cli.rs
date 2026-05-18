use porthole::{
    client::ClientError,
    commands::agents::{AgentClient, AgentDurationArg, AgentGrantCommand, AgentTokenCommand, AgentsCommand, run_with_output},
};
use porthole_core::{
    SurfaceId,
    agent_policy::{ActionClass, AgentId, GrantId, PermissionRequestId},
};
use porthole_protocol::agent_permissions::{
    AgentGrantResponse, AgentIdentityMetadata, AgentIdentityResponse, AgentPermissionConstraints, AgentPermissionDuration,
    AgentPermissionRequestResponse, AgentPermissionTarget, ApproveAgentPermissionRequest, CreateAgentIdentityRequest,
    CreateAgentIdentityResponse, DenyAgentPermissionRequest, MintAgentTokenResponse, RevocationResponse,
};

#[tokio::test]
async fn agents_create_json_renders_token_once() {
    let mut client = FakeAgentClient {
        create_identity_response: Some(CreateAgentIdentityResponse {
            agent_id: AgentId::from("agent_1"),
            token: "pta_agent_1.secret".into(),
        }),
        ..FakeAgentClient::default()
    };

    let output = run_with_output(
        &mut client,
        AgentsCommand::Create {
            name: "test".into(),
            json: true,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        client.create_identity_request,
        Some(CreateAgentIdentityRequest {
            display_name: "test".into(),
            metadata: None,
        })
    );
    assert_eq!(output.matches("pta_agent_1.secret").count(), 1);
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["agent_id"], "agent_1");
    assert_eq!(json["token"], "pta_agent_1.secret");
}

#[tokio::test]
async fn agents_show_json_omits_token_secrets() {
    let mut client = FakeAgentClient {
        identity_response: Some(identity_response()),
        ..FakeAgentClient::default()
    };

    let output = run_with_output(
        &mut client,
        AgentsCommand::Show {
            agent_id: "agent_1".into(),
            json: true,
        },
    )
    .await
    .unwrap();

    assert_eq!(client.shown_agent_id.as_deref(), Some("agent_1"));
    assert!(!output.contains("pta_"));
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["display_name"], "Test Agent");
    assert_eq!(json["metadata"]["vendor"], "Acme");
}

#[tokio::test]
async fn agents_token_create_json_renders_second_token_once() {
    let mut client = FakeAgentClient {
        mint_token_response: Some(MintAgentTokenResponse {
            token_id: "tok_2".into(),
            token: "pta_agent_1.second".into(),
        }),
        ..FakeAgentClient::default()
    };

    let output = run_with_output(
        &mut client,
        AgentsCommand::Token {
            command: AgentTokenCommand::Create {
                agent_id: "agent_1".into(),
                json: true,
            },
        },
    )
    .await
    .unwrap();

    assert_eq!(client.minted_token_agent_id.as_deref(), Some("agent_1"));
    assert_eq!(output.matches("pta_agent_1.second").count(), 1);
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["token_id"], "tok_2");
    assert_eq!(json["token"], "pta_agent_1.second");
}

#[tokio::test]
async fn agents_revoke_commands_call_revocation_endpoints() {
    let mut client = FakeAgentClient {
        revoke_identity_response: Some(RevocationResponse { revoked: true }),
        revoke_token_response: Some(RevocationResponse { revoked: true }),
        ..FakeAgentClient::default()
    };

    let identity_output = run_with_output(
        &mut client,
        AgentsCommand::Revoke {
            agent_id: "agent_1".into(),
            json: false,
        },
    )
    .await
    .unwrap();
    let token_output = run_with_output(
        &mut client,
        AgentsCommand::Token {
            command: AgentTokenCommand::Revoke {
                agent_id: "agent_1".into(),
                token_id: "tok_1".into(),
                json: false,
            },
        },
    )
    .await
    .unwrap();

    assert_eq!(client.revoked_identity_agent_id.as_deref(), Some("agent_1"));
    assert_eq!(client.revoked_token.as_deref(), Some("agent_1/tok_1"));
    assert!(identity_output.contains("revoked: true"));
    assert!(token_output.contains("revoked: true"));
}

#[tokio::test]
async fn agents_grants_json_renders_active_grants() {
    let mut client = FakeAgentClient {
        grants_response: vec![grant_response()],
        ..FakeAgentClient::default()
    };

    let output = run_with_output(&mut client, AgentsCommand::Grants { json: true }).await.unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json[0]["grant_id"], "grant_1");
    assert_eq!(json[0]["target"]["surface_id"], "surf_1");
}

#[tokio::test]
async fn agents_grant_revoke_calls_revocation_endpoint() {
    let mut client = FakeAgentClient {
        revoke_grant_response: Some(RevocationResponse { revoked: true }),
        ..FakeAgentClient::default()
    };

    let output = run_with_output(
        &mut client,
        AgentsCommand::Grant {
            command: AgentGrantCommand::Revoke {
                grant_id: "grant_1".into(),
                json: false,
            },
        },
    )
    .await
    .unwrap();

    assert_eq!(client.revoked_grant_id.as_deref(), Some("grant_1"));
    assert!(output.contains("revoked: true"));
}

#[tokio::test]
async fn agents_requests_json_renders_pending_request_ids() {
    let mut client = FakeAgentClient {
        requests_response: vec![request_response()],
        ..FakeAgentClient::default()
    };

    let output = run_with_output(&mut client, AgentsCommand::Requests { json: true }).await.unwrap();

    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json[0]["request_id"], "apr_1");
    assert_eq!(json[0]["status"], "pending");
}

#[tokio::test]
async fn agents_request_json_renders_one_request_status() {
    let mut client = FakeAgentClient {
        request_response: Some(request_response()),
        ..FakeAgentClient::default()
    };

    let output = run_with_output(
        &mut client,
        AgentsCommand::Request {
            request_id: "apr_1".into(),
            json: true,
        },
    )
    .await
    .unwrap();

    assert_eq!(client.fetched_request_id.as_deref(), Some("apr_1"));
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["request_id"], "apr_1");
    assert_eq!(json["status"], "pending");
}

#[tokio::test]
async fn agents_approve_prefetches_request_and_posts_matching_target_actions() {
    let mut client = FakeAgentClient {
        request_response: Some(request_response()),
        approve_response: Some(grant_response()),
        ..FakeAgentClient::default()
    };

    let output = run_with_output(
        &mut client,
        AgentsCommand::Approve {
            request_id: "apr_1".into(),
            duration: AgentDurationArg::UntilSurfaceGone,
            json: true,
        },
    )
    .await
    .unwrap();

    assert_eq!(client.fetched_request_id.as_deref(), Some("apr_1"));
    assert_eq!(client.approved_request_id.as_deref(), Some("apr_1"));
    assert_eq!(
        client.approve_body,
        Some(ApproveAgentPermissionRequest {
            duration: AgentPermissionDuration::UntilSurfaceGone,
            target: AgentPermissionTarget::Surface {
                surface_id: SurfaceId::from("surf_1"),
            },
            actions: vec![ActionClass::Drive],
            constraints: AgentPermissionConstraints::default(),
        })
    );
    let json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(json["grant_id"], "grant_1");
}

#[tokio::test]
async fn agents_deny_posts_reason_and_remember_flag() {
    let mut client = FakeAgentClient {
        deny_response: Some(request_response()),
        ..FakeAgentClient::default()
    };

    let output = run_with_output(
        &mut client,
        AgentsCommand::Deny {
            request_id: "apr_1".into(),
            remember: true,
            reason: Some("not_now".into()),
            json: false,
        },
    )
    .await
    .unwrap();

    assert_eq!(client.denied_request_id.as_deref(), Some("apr_1"));
    assert_eq!(
        client.deny_body,
        Some(DenyAgentPermissionRequest {
            remember: true,
            reason: Some("not_now".into()),
        })
    );
    assert!(output.contains("status: pending"));
}

#[derive(Default)]
struct FakeAgentClient {
    create_identity_request: Option<CreateAgentIdentityRequest>,
    create_identity_response: Option<CreateAgentIdentityResponse>,
    identity_response: Option<AgentIdentityResponse>,
    shown_agent_id: Option<String>,
    minted_token_agent_id: Option<String>,
    mint_token_response: Option<MintAgentTokenResponse>,
    revoked_identity_agent_id: Option<String>,
    revoke_identity_response: Option<RevocationResponse>,
    revoked_token: Option<String>,
    revoke_token_response: Option<RevocationResponse>,
    requests_response: Vec<AgentPermissionRequestResponse>,
    request_response: Option<AgentPermissionRequestResponse>,
    fetched_request_id: Option<String>,
    approve_body: Option<ApproveAgentPermissionRequest>,
    approved_request_id: Option<String>,
    approve_response: Option<AgentGrantResponse>,
    deny_body: Option<DenyAgentPermissionRequest>,
    denied_request_id: Option<String>,
    deny_response: Option<AgentPermissionRequestResponse>,
    grants_response: Vec<AgentGrantResponse>,
    revoked_grant_id: Option<String>,
    revoke_grant_response: Option<RevocationResponse>,
}

#[async_trait::async_trait(?Send)]
impl AgentClient for FakeAgentClient {
    async fn create_identity(&mut self, request: CreateAgentIdentityRequest) -> Result<CreateAgentIdentityResponse, ClientError> {
        self.create_identity_request = Some(request);
        self.create_identity_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing create identity response".into()))
    }

    async fn list_identities(&mut self) -> Result<Vec<AgentIdentityResponse>, ClientError> {
        Ok(vec![identity_response()])
    }

    async fn get_identity(&mut self, agent_id: &str) -> Result<AgentIdentityResponse, ClientError> {
        self.shown_agent_id = Some(agent_id.into());
        self.identity_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing identity response".into()))
    }

    async fn revoke_identity(&mut self, agent_id: &str) -> Result<RevocationResponse, ClientError> {
        self.revoked_identity_agent_id = Some(agent_id.into());
        self.revoke_identity_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing revoke identity response".into()))
    }

    async fn mint_token(&mut self, agent_id: &str) -> Result<MintAgentTokenResponse, ClientError> {
        self.minted_token_agent_id = Some(agent_id.into());
        self.mint_token_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing mint token response".into()))
    }

    async fn revoke_token(&mut self, agent_id: &str, token_id: &str) -> Result<RevocationResponse, ClientError> {
        self.revoked_token = Some(format!("{agent_id}/{token_id}"));
        self.revoke_token_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing revoke token response".into()))
    }

    async fn list_requests(&mut self) -> Result<Vec<AgentPermissionRequestResponse>, ClientError> {
        Ok(self.requests_response.clone())
    }

    async fn get_request(&mut self, request_id: &str) -> Result<AgentPermissionRequestResponse, ClientError> {
        self.fetched_request_id = Some(request_id.into());
        self.request_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing request response".into()))
    }

    async fn approve_request(&mut self, request_id: &str, body: ApproveAgentPermissionRequest) -> Result<AgentGrantResponse, ClientError> {
        self.approved_request_id = Some(request_id.into());
        self.approve_body = Some(body);
        self.approve_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing approve response".into()))
    }

    async fn deny_request(
        &mut self,
        request_id: &str,
        body: DenyAgentPermissionRequest,
    ) -> Result<AgentPermissionRequestResponse, ClientError> {
        self.denied_request_id = Some(request_id.into());
        self.deny_body = Some(body);
        self.deny_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing deny response".into()))
    }

    async fn list_grants(&mut self) -> Result<Vec<AgentGrantResponse>, ClientError> {
        Ok(self.grants_response.clone())
    }

    async fn revoke_grant(&mut self, grant_id: &str) -> Result<RevocationResponse, ClientError> {
        self.revoked_grant_id = Some(grant_id.into());
        self.revoke_grant_response
            .clone()
            .ok_or_else(|| ClientError::Local("missing revoke grant response".into()))
    }
}

fn identity_response() -> AgentIdentityResponse {
    AgentIdentityResponse {
        agent_id: AgentId::from("agent_1"),
        display_name: "Test Agent".into(),
        metadata: Some(AgentIdentityMetadata {
            bundle_id: Some("com.example.agent".into()),
            executable_path: None,
            vendor: Some("Acme".into()),
        }),
        created_at_unix_ms: 1_000,
        revoked_at_unix_ms: None,
    }
}

fn request_response() -> AgentPermissionRequestResponse {
    AgentPermissionRequestResponse {
        request_id: PermissionRequestId::from("apr_1"),
        agent_id: AgentId::from("agent_1"),
        target: AgentPermissionTarget::Surface {
            surface_id: SurfaceId::from("surf_1"),
        },
        actions: vec![ActionClass::Drive],
        reason: Some("drive".into()),
        status: "pending".into(),
        created_at_unix_ms: 1_000,
        resolved_at_unix_ms: None,
    }
}

fn grant_response() -> AgentGrantResponse {
    AgentGrantResponse {
        grant_id: GrantId::from("grant_1"),
        agent_id: AgentId::from("agent_1"),
        origin_request_id: Some(PermissionRequestId::from("apr_1")),
        target: AgentPermissionTarget::Surface {
            surface_id: SurfaceId::from("surf_1"),
        },
        actions: vec![ActionClass::Drive],
        duration: AgentPermissionDuration::UntilSurfaceGone,
        constraints: AgentPermissionConstraints::default(),
        created_at_unix_ms: 1_001,
        expires_at_unix_ms: None,
        consumed_at_unix_ms: None,
        revoked_at_unix_ms: None,
    }
}
