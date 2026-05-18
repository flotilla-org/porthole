use std::convert::Infallible;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use tokio::sync::broadcast;

use crate::{
    events::{AgentEvent, sse_event},
    state::AppState,
};

pub async fn get_events(State(state): State<AppState>) -> Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.events.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => yield Ok(sse_event(&event)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "agent event subscriber lagged; requesting resync");
                    yield Ok(sse_event(&AgentEvent::ResyncRequired));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use tokio::sync::broadcast;

    use crate::events::{AgentEvent, EventBus};

    #[tokio::test]
    async fn lagged_receiver_observes_resync_requirement() {
        let bus = EventBus::with_capacity(1);
        let mut rx = bus.subscribe();
        bus.publish(AgentEvent::AgentPolicyChanged { resource: "one".into() });
        bus.publish(AgentEvent::AgentPolicyChanged { resource: "two".into() });

        let lag = rx.recv().await.unwrap_err();

        assert!(matches!(lag, broadcast::error::RecvError::Lagged(1)));
    }
}
