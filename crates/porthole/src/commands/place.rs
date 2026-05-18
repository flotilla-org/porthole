use porthole_core::{display::Rect, input::CoordUnits, placement::PlacementSpec};
use porthole_protocol::placement::{PlaceRequest, PlaceResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run_rect(
    client: &DaemonClient,
    surface_id: String,
    rect: Rect,
    units: CoordUnits,
    session: Option<String>,
) -> Result<(), ClientError> {
    let req = PlaceRequest {
        rect: Some(rect),
        placement: None,
        units,
        session,
    };
    run_request(client, surface_id, req).await
}

pub async fn run_placement(
    client: &DaemonClient,
    surface_id: String,
    placement: PlacementSpec,
    session: Option<String>,
) -> Result<(), ClientError> {
    let req = PlaceRequest {
        rect: None,
        placement: Some(placement),
        units: CoordUnits::Logical,
        session,
    };
    run_request(client, surface_id, req).await
}

async fn run_request(client: &DaemonClient, surface_id: String, req: PlaceRequest) -> Result<(), ClientError> {
    let res: PlaceResponse = client.post_json(&format!("/surfaces/{surface_id}/place"), &req).await?;
    println!("placed surface {}", res.surface_id);
    Ok(())
}
