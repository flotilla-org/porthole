use std::sync::Arc;

use portholed::{runtime::socket_path, server::serve};
use tracing::warn;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let adapter = build_adapter();

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
    serve(adapter, path).await
}

#[cfg(target_os = "macos")]
fn build_adapter() -> Arc<dyn porthole_core::adapter::Adapter> {
    Arc::new(porthole_adapter_macos::MacOsAdapter::new())
}

#[cfg(not(target_os = "macos"))]
fn build_adapter() -> Arc<dyn porthole_core::adapter::Adapter> {
    tracing::warn!("no native adapter for this platform; falling back to in-memory adapter");
    Arc::new(porthole_core::in_memory::InMemoryAdapter::new())
}
