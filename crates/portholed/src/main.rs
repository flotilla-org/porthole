use std::sync::Arc;

#[cfg(target_os = "linux")]
use porthole_adapter_kwin::KWinAdapter;
use porthole_adapter_kwin::bridge::KWinBridge;
use portholed::{runtime::socket_path, server::serve_with_agent_policy_and_kwin_bridge};
use tracing::warn;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let kwin_bridge = KWinBridge::new();
    let adapter = build_adapter(kwin_bridge.clone());

    // Check for missing system permissions and warn on startup
    let perms = adapter.system_permissions().await.unwrap_or_default();
    for p in &perms {
        if !p.granted {
            warn!(
                permission = %p.name,
                "{} system permission missing; calls that need it will return system_permission_needed. Run `porthole onboard` or see docs/development.md.",
                p.name
            );
        }
    }

    let path = socket_path();
    let agent_store = portholed::agent_store::AgentPolicyStore::open_default()
        .await
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    serve_with_agent_policy_and_kwin_bridge(adapter, path, agent_store, portholed::events::EventBus::new(), kwin_bridge).await
}

#[cfg(target_os = "macos")]
fn build_adapter(_kwin_bridge: KWinBridge) -> Arc<dyn porthole_core::adapter::Adapter> {
    Arc::new(porthole_adapter_macos::MacOsAdapter::new())
}

#[cfg(target_os = "linux")]
fn build_adapter(kwin_bridge: KWinBridge) -> Arc<dyn porthole_core::adapter::Adapter> {
    if looks_like_kde_wayland() {
        return Arc::new(KWinAdapter::new(kwin_bridge));
    }
    tracing::warn!("no native adapter for this Linux session; falling back to in-memory adapter");
    Arc::new(porthole_core::in_memory::InMemoryAdapter::new())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn build_adapter(_kwin_bridge: KWinBridge) -> Arc<dyn porthole_core::adapter::Adapter> {
    tracing::warn!("no native adapter for this platform; falling back to in-memory adapter");
    Arc::new(porthole_core::in_memory::InMemoryAdapter::new())
}

#[cfg(target_os = "linux")]
fn looks_like_kde_wayland() -> bool {
    let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_lowercase();
    let session = std::env::var("DESKTOP_SESSION").unwrap_or_default().to_lowercase();
    let kde_session = std::env::var_os("KDE_FULL_SESSION").is_some();
    is_wayland && (kde_session || desktop.contains("kde") || desktop.contains("plasma") || session.contains("plasma"))
}
