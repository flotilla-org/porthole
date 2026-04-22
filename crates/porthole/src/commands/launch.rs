use std::collections::BTreeMap;
use std::path::PathBuf;

use porthole_core::placement::PlacementSpec;
use porthole_protocol::launches::{
    ArtifactLaunch, LaunchKind, LaunchRequest, LaunchResponse, ProcessLaunch, WireConfidence,
};

use crate::client::{ClientError, DaemonClient};

pub enum LaunchKindArg {
    Process { app: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<String> },
    Artifact { path: PathBuf },
}

pub struct LaunchArgs {
    pub kind: LaunchKindArg,
    pub session: Option<String>,
    pub timeout_ms: u64,
    pub require_confidence: WireConfidence,
    pub require_fresh_surface: bool,
    pub placement: Option<PlacementSpec>,
    pub auto_dismiss_after_ms: Option<u64>,
    pub json: bool,
}

pub async fn run(client: &DaemonClient, args: LaunchArgs) -> Result<(), ClientError> {
    let kind = match args.kind {
        LaunchKindArg::Process { app, args: a, env, cwd } => {
            let mut env_map = BTreeMap::new();
            for (k, v) in env {
                env_map.insert(k, v);
            }
            LaunchKind::Process(ProcessLaunch { app, args: a, cwd, env: env_map })
        }
        LaunchKindArg::Artifact { path } => LaunchKind::Artifact(ArtifactLaunch {
            path: path.to_string_lossy().to_string(),
        }),
    };
    let req = LaunchRequest {
        kind,
        session: args.session,
        require_confidence: args.require_confidence,
        timeout_ms: args.timeout_ms,
        placement: args.placement,
        auto_dismiss_after_ms: args.auto_dismiss_after_ms,
        require_fresh_surface: args.require_fresh_surface,
    };
    let res: LaunchResponse = client.post_json("/launches", &req).await?;
    if args.json {
        let text = serde_json::to_string_pretty(&res)
            .map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        println!("launch_id: {}", res.launch_id);
        println!("surface_id: {}", res.surface_id);
        println!("confidence: {:?}", res.confidence);
        println!("correlation: {:?}", res.correlation);
        println!("surface_was_preexisting: {}", res.surface_was_preexisting);
        println!("placement: {:?}", res.placement);
    }
    Ok(())
}
