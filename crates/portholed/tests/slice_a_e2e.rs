#![cfg(unix)]

use std::{sync::Arc, time::Duration};

mod common;

use common::approve_permission_needed;
use porthole_core::{in_memory::InMemoryAdapter, surface::SurfaceInfo};
use porthole_protocol::agent_permissions::{AgentPermissionDuration, CreateAgentIdentityRequest, CreateAgentIdentityResponse};
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
    let first_launch: Result<porthole_protocol::launches::LaunchResponse, porthole::client::ClientError> = agent_client
        .post_json(
            "/launches",
            &serde_json::json!({ "kind": { "type": "process", "app": "X", "args": [] } }),
        )
        .await;
    approve_permission_needed(
        &client,
        first_launch.expect_err("launch should need permission"),
        AgentPermissionDuration::Once,
    )
    .await;
    let launch: porthole_protocol::launches::LaunchResponse = agent_client
        .post_json(
            "/launches",
            &serde_json::json!({ "kind": { "type": "process", "app": "X", "args": [] } }),
        )
        .await
        .expect("launch after approval");

    let first_key: Result<porthole_protocol::input::KeyResponse, porthole::client::ClientError> = agent_client
        .post_json(
            &format!("/surfaces/{}/key", launch.surface_id),
            &serde_json::json!({ "events": [{ "key": "Enter" }] }),
        )
        .await;
    approve_permission_needed(
        &client,
        first_key.expect_err("key should need permission"),
        AgentPermissionDuration::UntilSurfaceGone,
    )
    .await;

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
    let first_wait: Result<porthole_protocol::wait::WaitResponse, porthole::client::ClientError> = agent_client
        .post_json(
            &format!("/surfaces/{}/wait", launch.surface_id),
            &serde_json::json!({ "condition": { "type": "exists" }, "timeout_ms": 1000 }),
        )
        .await;
    approve_permission_needed(
        &client,
        first_wait.expect_err("wait should need permission"),
        AgentPermissionDuration::UntilSurfaceGone,
    )
    .await;
    let _: porthole_protocol::wait::WaitResponse = agent_client
        .post_json(
            &format!("/surfaces/{}/wait", launch.surface_id),
            &serde_json::json!({ "condition": { "type": "exists" }, "timeout_ms": 1000 }),
        )
        .await
        .expect("wait after approval");
    // close
    let first_close: Result<porthole_protocol::close_focus::CloseResponse, porthole::client::ClientError> = agent_client
        .post_json(&format!("/surfaces/{}/close", launch.surface_id), &serde_json::json!({}))
        .await;
    approve_permission_needed(
        &client,
        first_close.expect_err("close should need permission"),
        AgentPermissionDuration::UntilSurfaceGone,
    )
    .await;
    let _: porthole_protocol::close_focus::CloseResponse = agent_client
        .post_json(&format!("/surfaces/{}/close", launch.surface_id), &serde_json::json!({}))
        .await
        .expect("close after approval");

    server_task.abort();

    // adapter-side recorder sanity
    assert_eq!(adapter.key_calls().await.len(), 1);
    assert_eq!(adapter.text_calls().await.len(), 1);
    assert_eq!(adapter.wait_calls().await.len(), 1);
    assert_eq!(adapter.close_calls().await.len(), 1);
}
