use std::{sync::Arc, time::Duration};

use porthole_core::{ErrorCode, agent_policy::ActionClass, in_memory::InMemoryAdapter, surface::SurfaceInfo};
use porthole_protocol::agent_permissions::{
    AgentPermissionConstraints, AgentPermissionDuration, AgentPermissionNeededDetails, AgentPermissionTarget,
    ApproveAgentPermissionRequest, CreateAgentIdentityRequest, CreateAgentIdentityResponse,
};
use portholed::{agent_store::AgentPolicyStore, events::EventBus, server::serve_with_agent_policy};

#[tokio::test]
async fn cli_through_daemon_key_text_click_wait_close() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("porthole.sock");

    let adapter = Arc::new(InMemoryAdapter::new());
    let socket_for_serve = socket.clone();
    let adapter_for_serve: Arc<dyn porthole_core::adapter::Adapter> = adapter.clone();
    let agent_store = AgentPolicyStore::open_in_memory().await.unwrap();
    let server_task =
        tokio::spawn(async move { serve_with_agent_policy(adapter_for_serve, socket_for_serve, agent_store, EventBus::new()).await });

    for _ in 0..200 {
        if socket.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(socket.exists(), "socket did not appear");

    // Seed a tracked surface directly so we don't have to launch.
    let info = SurfaceInfo::window(porthole_core::SurfaceId::new(), 1);
    let id = info.id.clone();
    // NB: state is inside the server task; we simulate by going through /launches
    // instead. But with the in-memory adapter, a launch returns a brand-new surface
    // we don't have a seeded reference to. The CLI-level flow is: launch → got
    // surface id → do stuff. We follow that path here:
    let _ = (info, id);

    let client = porthole::client::DaemonClient::new(&socket);
    let identity: CreateAgentIdentityResponse = client
        .post_json(
            "/agent-identities",
            &CreateAgentIdentityRequest {
                display_name: "e2e-agent".into(),
                metadata: None,
            },
        )
        .await
        .expect("create agent identity");
    let agent_client = porthole::client::DaemonClient::new(&socket).with_bearer_token(identity.token);
    let launch: porthole_protocol::launches::LaunchResponse = client
        .post_json(
            "/launches",
            &serde_json::json!({ "kind": { "type": "process", "app": "X", "args": [] } }),
        )
        .await
        .expect("launch");

    let first_key: Result<porthole_protocol::input::KeyResponse, porthole::client::ClientError> = agent_client
        .post_json(
            &format!("/surfaces/{}/key", launch.surface_id),
            &serde_json::json!({ "events": [{ "key": "Enter" }] }),
        )
        .await;
    let permission_needed = match first_key {
        Err(porthole::client::ClientError::Api(wire)) if wire.code == ErrorCode::AgentPermissionNeeded => wire,
        other => panic!("expected permission-needed response, got {other:?}"),
    };
    let details: AgentPermissionNeededDetails = serde_json::from_value(permission_needed.details.unwrap()).unwrap();
    let _: porthole_protocol::agent_permissions::AgentGrantResponse = client
        .post_json(
            &format!("/agent-permissions/requests/{}/approve", details.request_id),
            &ApproveAgentPermissionRequest {
                duration: AgentPermissionDuration::UntilSurfaceGone,
                target: AgentPermissionTarget::Surface {
                    surface_id: launch.surface_id.clone(),
                },
                actions: vec![ActionClass::Drive],
                constraints: AgentPermissionConstraints::default(),
            },
        )
        .await
        .expect("approve drive permission");

    let _: porthole_protocol::input::KeyResponse = agent_client
        .post_json(
            &format!("/surfaces/{}/key", launch.surface_id),
            &serde_json::json!({ "events": [{ "key": "Enter" }] }),
        )
        .await
        .expect("key after approval");
    // text
    let _: porthole_protocol::input::TextResponse = agent_client
        .post_json(
            &format!("/surfaces/{}/text", launch.surface_id),
            &serde_json::json!({ "text": "hi" }),
        )
        .await
        .expect("text");
    // wait exists
    let _: porthole_protocol::wait::WaitResponse = client
        .post_json(
            &format!("/surfaces/{}/wait", launch.surface_id),
            &serde_json::json!({ "condition": { "type": "exists" }, "timeout_ms": 1000 }),
        )
        .await
        .expect("wait");
    // close
    let _: porthole_protocol::close_focus::CloseResponse = client
        .post_json(&format!("/surfaces/{}/close", launch.surface_id), &serde_json::json!({}))
        .await
        .expect("close");

    server_task.abort();

    // adapter-side recorder sanity
    assert_eq!(adapter.key_calls().await.len(), 1);
    assert_eq!(adapter.text_calls().await.len(), 1);
    assert_eq!(adapter.wait_calls().await.len(), 1);
    assert_eq!(adapter.close_calls().await.len(), 1);
}
