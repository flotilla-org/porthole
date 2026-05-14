use porthole_core::input::CoordUnits;
use porthole_protocol::input::{PointerMoveRequest, PointerMoveResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run_move(
    client: &DaemonClient,
    surface_id: String,
    x: f64,
    y: f64,
    units: CoordUnits,
    session: Option<String>,
) -> Result<(), ClientError> {
    let req = PointerMoveRequest { x, y, units, session };
    let res: PointerMoveResponse = client.post_json(&format!("/surfaces/{surface_id}/pointer/move"), &req).await?;
    println!("pointer moved on surface {}", res.surface_id);
    Ok(())
}
