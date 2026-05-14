use porthole_core::input::CoordUnits;
use porthole_protocol::content_rect::ContentRectResponse;

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient, surface_id: String, units: CoordUnits) -> Result<(), ClientError> {
    let units_param = match units {
        CoordUnits::Logical => "logical",
        CoordUnits::Physical => "physical",
    };
    let path = format!("/surfaces/{surface_id}/content-rect?units={units_param}");
    let res: ContentRectResponse = client.get_json(&path).await?;
    let units_str = match res.units {
        CoordUnits::Logical => "logical",
        CoordUnits::Physical => "physical",
    };
    let descent_str = match res.descent {
        porthole_core::content_rect::Descent::Contents => "contents",
        porthole_core::content_rect::Descent::LargestChild => "largest_child",
    };
    println!("x: {}", res.x);
    println!("y: {}", res.y);
    println!("w: {}", res.w);
    println!("h: {}", res.h);
    println!("units: {units_str}");
    println!("ax_role: {}", res.ax_role);
    println!("descent: {descent_str}");
    Ok(())
}
