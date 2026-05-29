use porthole_protocol::search::{TrackRequest, TrackResponse};

use crate::client::{ClientError, DaemonClient};

pub struct TrackArgs {
    pub ref_: String,
    pub session: Option<String>,
    pub json: bool,
}

pub async fn run(client: &DaemonClient, args: TrackArgs) -> Result<(), ClientError> {
    let req = TrackRequest {
        ref_: args.ref_,
        session: args.session,
    };
    let res: TrackResponse = client.post_json("/surfaces/track", &req).await?;
    if args.json {
        let text = serde_json::to_string_pretty(&res).map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        println!("surface_id: {}", res.surface_id);
        println!("pid: {}, platform_ref: {:?}", res.pid, res.platform_ref);
        println!("app_name: {}", res.app_name.as_deref().unwrap_or("-"));
        println!("title: {}", res.title.as_deref().unwrap_or("-"));
        println!("reused_existing_handle: {}", res.reused_existing_handle);
    }
    Ok(())
}
