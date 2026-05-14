use porthole_core::{display::Rect, input::CoordUnits};
use porthole_protocol::placement::{PlaceRequest, PlaceResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(
    client: &DaemonClient,
    surface_id: String,
    rect: Rect,
    units: CoordUnits,
    session: Option<String>,
) -> Result<(), ClientError> {
    let req = PlaceRequest { rect, units, session };
    let res: PlaceResponse = client.post_json(&format!("/surfaces/{surface_id}/place"), &req).await?;
    println!("placed surface {}", res.surface_id);
    Ok(())
}
