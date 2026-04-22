use std::collections::BTreeMap;

use porthole_protocol::launches::{LaunchKind, LaunchRequest, LaunchResponse, ProcessLaunch, WireConfidence};

use crate::client::{ClientError, DaemonClient};

pub struct LaunchArgs {
    pub app: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<String>,
    pub session: Option<String>,
    pub timeout_ms: u64,
    pub require_confidence: WireConfidence,
}

pub async fn run(client: &DaemonClient, args: LaunchArgs) -> Result<(), ClientError> {
    let mut env = BTreeMap::new();
    for (k, v) in args.env {
        env.insert(k, v);
    }
    let req = LaunchRequest {
        kind: LaunchKind::Process(ProcessLaunch { app: args.app, args: args.args, cwd: args.cwd, env }),
        session: args.session,
        require_confidence: args.require_confidence,
        timeout_ms: args.timeout_ms,
        placement: None,
        auto_dismiss_after_ms: None,
        require_fresh_surface: false,
    };
    let res: LaunchResponse = client.post_json("/launches", &req).await?;
    println!("launch_id: {}", res.launch_id);
    println!("surface_id: {}", res.surface_id);
    println!("confidence: {:?}", res.confidence);
    println!("correlation: {:?}", res.correlation);
    println!("surface_was_preexisting: {}", res.surface_was_preexisting);
    Ok(())
}
