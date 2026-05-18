use axum::response::sse::Event;
use porthole_core::agent_policy::{AgentId, PermissionRequestId};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

pub const DEFAULT_EVENT_CAPACITY: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentPermissionRequested {
        request_id: PermissionRequestId,
        agent_id: AgentId,
    },
    AgentPermissionResolved {
        request_id: PermissionRequestId,
        status: String,
    },
    AgentIdentityCreated {
        agent_id: AgentId,
        display_name: String,
    },
    AgentIdentityRevoked {
        agent_id: AgentId,
    },
    AgentPolicyChanged {
        resource: String,
    },
    ResyncRequired,
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AgentEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_EVENT_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.tx.subscribe()
    }

    pub fn publish(&self, event: AgentEvent) {
        let _ = self.tx.send(event);
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

pub fn sse_event(event: &AgentEvent) -> Event {
    let name = match event {
        AgentEvent::AgentPermissionRequested { .. } => "agent_permission_requested",
        AgentEvent::AgentPermissionResolved { .. } => "agent_permission_resolved",
        AgentEvent::AgentIdentityCreated { .. } => "agent_identity_created",
        AgentEvent::AgentIdentityRevoked { .. } => "agent_identity_revoked",
        AgentEvent::AgentPolicyChanged { .. } => "agent_policy_changed",
        AgentEvent::ResyncRequired => "resync_required",
    };
    Event::default()
        .event(name)
        .json_data(event)
        .unwrap_or_else(|_| Event::default().event("resync_required").data("{}"))
}
