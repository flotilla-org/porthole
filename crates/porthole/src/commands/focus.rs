use porthole_protocol::close_focus::{FocusRequest, FocusResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient, surface_id: String, session: Option<String>) -> Result<(), ClientError> {
    let req = FocusRequest { session };
    let res: FocusResponse = client.post_json(&format!("/surfaces/{surface_id}/focus"), &req).await?;
    println!("focused surface {}", res.surface_id);
    Ok(())
}
