use porthole_core::display::Rect;
use porthole_protocol::placement::{PlaceRequest, PlaceResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient, surface_id: String, rect: Rect, session: Option<String>) -> Result<(), ClientError> {
    let req = PlaceRequest { rect, session };
    let res: PlaceResponse = client.post_json(&format!("/surfaces/{surface_id}/place"), &req).await?;
    println!("placed surface {}", res.surface_id);
    Ok(())
}
