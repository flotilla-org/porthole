use porthole_protocol::info::InfoResponse;

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient) -> Result<(), ClientError> {
    let info: InfoResponse = client.get_json("/info").await?;
    println!("daemon_version: {}", info.daemon_version);
    println!("uptime_seconds: {}", info.uptime_seconds);
    for adapter in info.adapters {
        println!(
            "adapter: {} (loaded={}) capabilities={}",
            adapter.name,
            adapter.loaded,
            adapter.capabilities.join(","),
        );
        for perm in &adapter.system_permissions {
            if perm.granted {
                println!("  system permission {}: granted ({})", perm.name, perm.purpose);
            } else {
                let restart_hint = if perm.name == "accessibility" {
                    "  (will trigger the OS prompt; daemon restart required after grant)"
                } else {
                    "  (will trigger the OS prompt)"
                };
                println!(
                    "  system permission {}: MISSING ({})",
                    perm.name, perm.purpose
                );
                println!("    fix: porthole onboard{restart_hint}");
            }
        }
    }
    Ok(())
}
