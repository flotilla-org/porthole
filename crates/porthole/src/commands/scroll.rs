use porthole_protocol::input::{ScrollRequest, ScrollResponse};

use crate::client::{ClientError, DaemonClient};

pub struct ScrollArgs {
    pub surface_id: String,
    pub x: f64,
    pub y: f64,
    pub delta_x: f64,
    pub delta_y: f64,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: ScrollArgs) -> Result<(), ClientError> {
    let req = ScrollRequest { x: args.x, y: args.y, delta_x: args.delta_x, delta_y: args.delta_y, session: args.session };
    let res: ScrollResponse = client.post_json(&format!("/surfaces/{}/scroll", args.surface_id), &req).await?;
    println!("scrolled at ({}, {}) delta=({}, {}) on surface {}", args.x, args.y, args.delta_x, args.delta_y, res.surface_id);
    Ok(())
}
