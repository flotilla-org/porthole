//! Disposable local fixture for exercising the real approval API and TUI.
//! Run with an explicit isolated endpoint; never connects to a desktop adapter.
use std::sync::Arc;

use porthole_core::{
    SurfaceId,
    agent_policy::{ActionClass, DurationSpec, TargetSelector},
    in_memory::InMemoryAdapter,
    surface::SurfaceInfo,
};
use porthole_protocol::agent_permissions::{PermissionOperation, PermissionRequestContext, PermissionSurface};
use portholed::{
    agent_store::AgentPolicyStore,
    events::{AgentEvent, EventBus},
    server::build_router,
    state::AppState,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::args()
        .nth(1)
        .expect("usage: approval_inbox <isolated socket path or named pipe>");
    #[cfg(unix)]
    let endpoint = porthole_transport::Endpoint::from_socket_path(endpoint.into());
    #[cfg(windows)]
    let endpoint = porthole_transport::Endpoint::from_named_pipe_name(endpoint);
    let state = AppState::new_with_agent_policy(
        Arc::new(InMemoryAdapter::new()),
        AgentPolicyStore::open_in_memory().await?,
        EventBus::new(),
    );
    let agent = state.agent_store.create_identity("KS presenter", None, 1_789_600_000_000).await?;
    let mut surface = SurfaceInfo::window(SurfaceId::from("surf_demo"), 42);
    surface.app_name = Some("Simulator".into());
    surface.title = Some("iPhone demo".into());
    state.handles.insert(surface).await;
    let context = PermissionRequestContext {
        surface: Some(PermissionSurface {
            app_name: Some("Simulator".into()),
            title: Some("iPhone demo".into()),
            pid: Some(42),
        }),
        operation: Some(PermissionOperation::Capture { native: false }),
    };
    state
        .agent_store
        .find_or_create_pending_request(
            agent.agent_id.clone(),
            TargetSelector::Surface {
                surface_id: SurfaceId::from("surf_demo"),
            },
            vec![ActionClass::Observe, ActionClass::Record],
            Some("record surface".into()),
            context,
            1_789_600_000_001,
        )
        .await?;
    let request = state
        .agent_store
        .create_pending_request(
            agent.agent_id,
            TargetSelector::AllSurfaces,
            vec![ActionClass::Observe],
            Some("search surfaces".into()),
            1_789_600_000_002,
        )
        .await?;
    state
        .agent_store
        .approve_request(&request.request_id, DurationSpec::Persistent, vec![], 1_789_600_000_003)
        .await?;
    // A later arrival proves that the selected row survives a live update.
    let later = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let agent = later
            .agent_store
            .create_identity("Build agent", None, 1_789_600_008_000)
            .await
            .unwrap();
        let request = later
            .agent_store
            .create_pending_request(
                agent.agent_id.clone(),
                TargetSelector::AllSurfaces,
                vec![ActionClass::Observe],
                Some("search surfaces".into()),
                1_789_600_008_001,
            )
            .await
            .unwrap();
        later.events.publish(AgentEvent::AgentPermissionRequested {
            request_id: request.request_id,
            agent_id: agent.agent_id,
        });
    });
    println!("Disposable approval fixture: {}", endpoint.display_name());
    porthole_transport::serve(endpoint, build_router(state)).await?;
    Ok(())
}
