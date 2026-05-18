use clap::{Subcommand, ValueEnum};
use porthole_protocol::agent_permissions::{
    AgentGrantResponse, AgentIdentityResponse, AgentPermissionConstraints, AgentPermissionDuration, AgentPermissionRequestResponse,
    ApproveAgentPermissionRequest, CreateAgentIdentityRequest, CreateAgentIdentityResponse, DenyAgentPermissionRequest,
    MintAgentTokenResponse, RevocationResponse,
};
use serde::Serialize;

use crate::client::{ClientError, DaemonClient};

#[derive(Subcommand, Clone, Debug)]
pub enum AgentsCommand {
    /// Create an agent identity and print its bearer token once.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// List agent identities.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one agent identity.
    Show {
        agent_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Revoke an agent identity.
    Revoke {
        agent_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Manage agent bearer tokens.
    Token {
        #[command(subcommand)]
        command: AgentTokenCommand,
    },
    /// List active grants.
    Grants {
        #[arg(long)]
        json: bool,
    },
    /// Manage grants.
    Grant {
        #[command(subcommand)]
        command: AgentGrantCommand,
    },
    /// List pending permission requests.
    Requests {
        #[arg(long)]
        json: bool,
    },
    /// Show one permission request.
    Request {
        request_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Approve a pending permission request.
    Approve {
        request_id: String,
        #[arg(long, value_enum)]
        duration: AgentDurationArg,
        #[arg(long)]
        json: bool,
    },
    /// Deny a pending permission request.
    Deny {
        request_id: String,
        #[arg(long)]
        remember: bool,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Clone, Debug)]
pub enum AgentTokenCommand {
    /// Mint a new bearer token for an existing agent.
    Create {
        agent_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Revoke one bearer token.
    Revoke {
        agent_id: String,
        token_id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Clone, Debug)]
pub enum AgentGrantCommand {
    /// Revoke one grant.
    Revoke {
        grant_id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum AgentDurationArg {
    Once,
    UntilSurfaceGone,
    Persistent,
}

impl From<AgentDurationArg> for AgentPermissionDuration {
    fn from(duration: AgentDurationArg) -> Self {
        match duration {
            AgentDurationArg::Once => Self::Once,
            AgentDurationArg::UntilSurfaceGone => Self::UntilSurfaceGone,
            AgentDurationArg::Persistent => Self::Persistent,
        }
    }
}

#[async_trait::async_trait(?Send)]
pub trait AgentClient {
    async fn create_identity(&mut self, request: CreateAgentIdentityRequest) -> Result<CreateAgentIdentityResponse, ClientError>;
    async fn list_identities(&mut self) -> Result<Vec<AgentIdentityResponse>, ClientError>;
    async fn get_identity(&mut self, agent_id: &str) -> Result<AgentIdentityResponse, ClientError>;
    async fn revoke_identity(&mut self, agent_id: &str) -> Result<RevocationResponse, ClientError>;
    async fn mint_token(&mut self, agent_id: &str) -> Result<MintAgentTokenResponse, ClientError>;
    async fn revoke_token(&mut self, agent_id: &str, token_id: &str) -> Result<RevocationResponse, ClientError>;
    async fn list_requests(&mut self) -> Result<Vec<AgentPermissionRequestResponse>, ClientError>;
    async fn get_request(&mut self, request_id: &str) -> Result<AgentPermissionRequestResponse, ClientError>;
    async fn approve_request(&mut self, request_id: &str, body: ApproveAgentPermissionRequest) -> Result<AgentGrantResponse, ClientError>;
    async fn deny_request(
        &mut self,
        request_id: &str,
        body: DenyAgentPermissionRequest,
    ) -> Result<AgentPermissionRequestResponse, ClientError>;
    async fn list_grants(&mut self) -> Result<Vec<AgentGrantResponse>, ClientError>;
    async fn revoke_grant(&mut self, grant_id: &str) -> Result<RevocationResponse, ClientError>;
}

pub async fn run(client: &mut DaemonClient, command: AgentsCommand) -> Result<(), ClientError> {
    let output = run_with_output(client, command).await?;
    print!("{output}");
    Ok(())
}

pub async fn run_with_output<C: AgentClient>(client: &mut C, command: AgentsCommand) -> Result<String, ClientError> {
    match command {
        AgentsCommand::Create { name, json } => {
            let response = client
                .create_identity(CreateAgentIdentityRequest {
                    display_name: name,
                    metadata: None,
                })
                .await?;
            render_create_identity(response, json)
        }
        AgentsCommand::List { json } => render_identities(client.list_identities().await?, json),
        AgentsCommand::Show { agent_id, json } => render_identity(client.get_identity(&agent_id).await?, json),
        AgentsCommand::Revoke { agent_id, json } => render_revocation(client.revoke_identity(&agent_id).await?, json),
        AgentsCommand::Token { command } => match command {
            AgentTokenCommand::Create { agent_id, json } => render_mint_token(client.mint_token(&agent_id).await?, json),
            AgentTokenCommand::Revoke { agent_id, token_id, json } => {
                render_revocation(client.revoke_token(&agent_id, &token_id).await?, json)
            }
        },
        AgentsCommand::Grants { json } => render_grants(client.list_grants().await?, json),
        AgentsCommand::Grant { command } => match command {
            AgentGrantCommand::Revoke { grant_id, json } => render_revocation(client.revoke_grant(&grant_id).await?, json),
        },
        AgentsCommand::Requests { json } => render_requests(client.list_requests().await?, json),
        AgentsCommand::Request { request_id, json } => render_request(client.get_request(&request_id).await?, json),
        AgentsCommand::Approve {
            request_id,
            duration,
            json,
        } => {
            let request = client.get_request(&request_id).await?;
            let response = client
                .approve_request(
                    &request_id,
                    ApproveAgentPermissionRequest {
                        duration: duration.into(),
                        target: request.target,
                        actions: request.actions,
                        constraints: AgentPermissionConstraints::default(),
                    },
                )
                .await?;
            render_grant(response, json)
        }
        AgentsCommand::Deny {
            request_id,
            remember,
            reason,
            json,
        } => {
            let response = client
                .deny_request(&request_id, DenyAgentPermissionRequest { remember, reason })
                .await?;
            render_request(response, json)
        }
    }
}

fn render_json<T: Serialize>(value: &T) -> Result<String, ClientError> {
    serde_json::to_string_pretty(value)
        .map(|text| format!("{text}\n"))
        .map_err(|error| ClientError::Local(format!("json encode: {error}")))
}

fn render_create_identity(response: CreateAgentIdentityResponse, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(format!("agent_id: {}\ntoken: {}\n", response.agent_id, response.token))
    }
}

fn render_mint_token(response: MintAgentTokenResponse, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(format!("token_id: {}\ntoken: {}\n", response.token_id, response.token))
    }
}

fn render_identities(response: Vec<AgentIdentityResponse>, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(response
            .into_iter()
            .map(|identity| format!("agent_id: {}\ndisplay_name: {}\n", identity.agent_id, identity.display_name))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn render_identity(response: AgentIdentityResponse, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(format!(
            "agent_id: {}\ndisplay_name: {}\ncreated_at_unix_ms: {}\nrevoked_at_unix_ms: {}\n",
            response.agent_id,
            response.display_name,
            response.created_at_unix_ms,
            response
                .revoked_at_unix_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".into())
        ))
    }
}

fn render_requests(response: Vec<AgentPermissionRequestResponse>, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(response
            .into_iter()
            .map(|request| {
                format!(
                    "request_id: {}\nagent_id: {}\nstatus: {}\n",
                    request.request_id, request.agent_id, request.status
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn render_request(response: AgentPermissionRequestResponse, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(format!(
            "request_id: {}\nagent_id: {}\nstatus: {}\n",
            response.request_id, response.agent_id, response.status
        ))
    }
}

fn render_grants(response: Vec<AgentGrantResponse>, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(response
            .into_iter()
            .map(|grant| format!("grant_id: {}\nagent_id: {}\n", grant.grant_id, grant.agent_id))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

fn render_grant(response: AgentGrantResponse, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(format!("grant_id: {}\nagent_id: {}\n", response.grant_id, response.agent_id))
    }
}

fn render_revocation(response: RevocationResponse, json: bool) -> Result<String, ClientError> {
    if json {
        render_json(&response)
    } else {
        Ok(format!("revoked: {}\n", response.revoked))
    }
}

#[async_trait::async_trait(?Send)]
impl AgentClient for DaemonClient {
    async fn create_identity(&mut self, request: CreateAgentIdentityRequest) -> Result<CreateAgentIdentityResponse, ClientError> {
        self.post_json("/agent-identities", &request).await
    }

    async fn list_identities(&mut self) -> Result<Vec<AgentIdentityResponse>, ClientError> {
        self.get_json("/agent-identities").await
    }

    async fn get_identity(&mut self, agent_id: &str) -> Result<AgentIdentityResponse, ClientError> {
        self.get_json(&format!("/agent-identities/{agent_id}")).await
    }

    async fn revoke_identity(&mut self, agent_id: &str) -> Result<RevocationResponse, ClientError> {
        self.post_json(&format!("/agent-identities/{agent_id}/revoke"), &serde_json::json!({}))
            .await
    }

    async fn mint_token(&mut self, agent_id: &str) -> Result<MintAgentTokenResponse, ClientError> {
        self.post_json(&format!("/agent-identities/{agent_id}/tokens"), &serde_json::json!({}))
            .await
    }

    async fn revoke_token(&mut self, agent_id: &str, token_id: &str) -> Result<RevocationResponse, ClientError> {
        self.post_json(
            &format!("/agent-identities/{agent_id}/tokens/{token_id}/revoke"),
            &serde_json::json!({}),
        )
        .await
    }

    async fn list_requests(&mut self) -> Result<Vec<AgentPermissionRequestResponse>, ClientError> {
        self.get_json("/agent-permissions/requests").await
    }

    async fn get_request(&mut self, request_id: &str) -> Result<AgentPermissionRequestResponse, ClientError> {
        self.get_json(&format!("/agent-permissions/requests/{request_id}")).await
    }

    async fn approve_request(&mut self, request_id: &str, body: ApproveAgentPermissionRequest) -> Result<AgentGrantResponse, ClientError> {
        self.post_json(&format!("/agent-permissions/requests/{request_id}/approve"), &body)
            .await
    }

    async fn deny_request(
        &mut self,
        request_id: &str,
        body: DenyAgentPermissionRequest,
    ) -> Result<AgentPermissionRequestResponse, ClientError> {
        self.post_json(&format!("/agent-permissions/requests/{request_id}/deny"), &body)
            .await
    }

    async fn list_grants(&mut self) -> Result<Vec<AgentGrantResponse>, ClientError> {
        self.get_json("/agent-permissions/grants").await
    }

    async fn revoke_grant(&mut self, grant_id: &str) -> Result<RevocationResponse, ClientError> {
        self.post_json(&format!("/agent-permissions/grants/{grant_id}/revoke"), &serde_json::json!({}))
            .await
    }
}
