//! Isolated native server for Windows native capture checks (#186). Serves
//! `\\.\pipe\porthole-<suffix>` with an in-memory policy store, so it never
//! touches the user's daemon, pipe or agents. Point the CLI at it by running
//! it with `USERNAME=<suffix>`.
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    use std::sync::Arc;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();
    let suffix = std::env::args().nth(1).expect("provide a unique test pipe suffix");
    assert!(!suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    let endpoint = porthole_transport::Endpoint::from_named_pipe_name(format!(r"\\.\pipe\porthole-{suffix}"));
    let store = portholed::agent_store::AgentPolicyStore::open_in_memory()
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    portholed::server::serve_with_agent_policy_and_windows_adapter(
        Arc::new(porthole_adapter_windows::WindowsAdapter::new()),
        endpoint,
        store,
        portholed::events::EventBus::new(),
    )
    .await
}
