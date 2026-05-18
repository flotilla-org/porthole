use porthole_core::ErrorCode;
use porthole_protocol::agent_permissions::{
    AgentPermissionConstraints, AgentPermissionDuration, AgentPermissionNeededDetails, ApproveAgentPermissionRequest,
};

pub async fn approve_permission_needed(
    operator: &porthole::client::DaemonClient,
    error: porthole::client::ClientError,
    duration: AgentPermissionDuration,
) {
    let permission_needed = match error {
        porthole::client::ClientError::Api(wire) if wire.code == ErrorCode::AgentPermissionNeeded => wire,
        other => panic!("expected permission-needed response, got {other:?}"),
    };
    let details: AgentPermissionNeededDetails = serde_json::from_value(permission_needed.details.unwrap()).unwrap();
    let _: porthole_protocol::agent_permissions::AgentGrantResponse = operator
        .post_json(
            &format!("/agent-permissions/requests/{}/approve", details.request_id),
            &ApproveAgentPermissionRequest {
                duration,
                target: details.target,
                actions: details.actions,
                constraints: AgentPermissionConstraints::default(),
            },
        )
        .await
        .expect("approve permission");
}
