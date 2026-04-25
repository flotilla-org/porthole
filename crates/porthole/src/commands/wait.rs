use porthole_core::wait::WaitCondition;
use porthole_protocol::wait::{WaitRequest, WaitResponse};

use crate::client::{ClientError, DaemonClient};

pub struct WaitArgs {
    pub surface_id: String,
    pub condition: WaitCondition,
    pub timeout_ms: u64,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: WaitArgs) -> Result<(), ClientError> {
    let req = WaitRequest {
        condition: args.condition,
        timeout_ms: args.timeout_ms,
        session: args.session,
    };
    let res: WaitResponse = client.post_json(&format!("/surfaces/{}/wait", args.surface_id), &req).await?;
    println!(
        "waited {}ms for condition '{}' on surface {}",
        res.elapsed_ms, res.condition, res.surface_id
    );
    Ok(())
}
