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
            println!(
                "  system permission {}: {} ({})",
                perm.name,
                if perm.granted { "granted" } else { "MISSING" },
                perm.purpose,
            );
        }
    }
    Ok(())
}
