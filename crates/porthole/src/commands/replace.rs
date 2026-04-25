use porthole_protocol::launches::{LaunchRequest, LaunchResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient, surface_id: String, req: LaunchRequest, json: bool) -> Result<(), ClientError> {
    let res: LaunchResponse = client.post_json(&format!("/surfaces/{surface_id}/replace"), &req).await?;
    if json {
        let text = serde_json::to_string_pretty(&res).map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        println!("new surface_id: {}", res.surface_id);
        println!("old surface closed.");
    }
    Ok(())
}
