//! Isolated native server for the Windows foreground acceptance probe.
//! Uses a unique pipe and an in-memory policy store, preserving existing agents.
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    use std::sync::Arc;

    let suffix = std::env::args().nth(1).expect("provide a unique test pipe suffix");
    assert!(suffix.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    let endpoint = porthole_transport::Endpoint::from_named_pipe_name(format!(r"\\.\pipe\porthole-foreground-{suffix}"));
    let store = portholed::agent_store::AgentPolicyStore::open_in_memory()
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    portholed::server::serve_with_agent_policy(
        Arc::new(porthole_adapter_windows::WindowsAdapter::new()),
        endpoint,
        store,
        portholed::events::EventBus::new(),
    )
    .await
}
