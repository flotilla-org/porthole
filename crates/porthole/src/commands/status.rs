use std::path::Path;

use porthole_protocol::info::{AdapterInfo, InfoResponse};

use crate::client::{ClientError, DaemonClient};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StatusOutcome {
    Up,
    Down,
}

pub async fn run(client: &DaemonClient, socket_path: &Path) -> StatusOutcome {
    match client.get_json::<InfoResponse>("/info").await {
        Ok(info) => {
            print!("{}", render_up(socket_path, &info));
            StatusOutcome::Up
        }
        Err(error) => {
            print!("{}", render_down(socket_path, &error));
            StatusOutcome::Down
        }
    }
}

pub fn render_up(socket_path: &Path, info: &InfoResponse) -> String {
    let mut out = String::new();
    out.push_str("daemon: up\n");
    out.push_str(&format!("socket: {}\n", socket_path.display()));
    out.push_str(&format!("version: {}\n", info.daemon_version));
    out.push_str(&format!("uptime_seconds: {}\n", info.uptime_seconds));
    out.push_str(&format!("surfaces: {}\n", info.surface_count));
    for adapter in &info.adapters {
        out.push_str(&render_adapter(adapter));
    }
    out
}

pub fn render_down(socket_path: &Path, error: &ClientError) -> String {
    format!("daemon: down\nsocket: {}\nerror: {error}\n", socket_path.display())
}

fn render_adapter(adapter: &AdapterInfo) -> String {
    format!(
        "adapter: {} (loaded={}) capabilities={}\n",
        adapter.name,
        adapter.loaded,
        adapter.capabilities.join(","),
    )
}

#[cfg(test)]
mod tests {
    use porthole_protocol::info::AdapterInfo;

    use super::*;

    fn info(surface_count: u64) -> InfoResponse {
        InfoResponse {
            daemon_version: "0.0.0-test".into(),
            uptime_seconds: 7,
            surface_count,
            adapters: vec![AdapterInfo {
                name: "in-memory".into(),
                loaded: true,
                capabilities: vec!["launch".into(), "screenshot".into()],
                system_permissions: vec![],
            }],
        }
    }

    #[test]
    fn render_up_includes_socket_version_and_surface_count() {
        let output = render_up(Path::new("/tmp/porthole.sock"), &info(3));

        assert_eq!(
            output,
            "daemon: up\n\
             socket: /tmp/porthole.sock\n\
             version: 0.0.0-test\n\
             uptime_seconds: 7\n\
             surfaces: 3\n\
             adapter: in-memory (loaded=true) capabilities=launch,screenshot\n"
        );
    }

    #[test]
    fn render_down_includes_socket_and_error() {
        let output = render_down(Path::new("/tmp/porthole.sock"), &ClientError::Local("connection refused".into()));

        assert_eq!(
            output,
            "daemon: down\n\
             socket: /tmp/porthole.sock\n\
             error: connection refused\n"
        );
    }
}
