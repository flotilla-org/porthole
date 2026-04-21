use porthole_protocol::close_focus::{CloseRequest, CloseResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient, surface_id: String, session: Option<String>) -> Result<(), ClientError> {
    let req = CloseRequest { session };
    let res: CloseResponse = client.post_json(&format!("/surfaces/{surface_id}/close"), &req).await?;
    println!("closed surface {}", res.surface_id);
    Ok(())
}
