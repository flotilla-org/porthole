use porthole_core::input::{ClickButton, Modifier};
use porthole_protocol::input::{ClickRequest, ClickResponse};

use crate::client::{ClientError, DaemonClient};

pub struct ClickArgs {
    pub surface_id: String,
    pub x: f64,
    pub y: f64,
    pub button: ClickButton,
    pub count: u8,
    pub modifiers: Vec<Modifier>,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: ClickArgs) -> Result<(), ClientError> {
    let req = ClickRequest {
        x: args.x,
        y: args.y,
        button: args.button,
        count: args.count,
        modifiers: args.modifiers,
        session: args.session,
    };
    let res: ClickResponse = client.post_json(&format!("/surfaces/{}/click", args.surface_id), &req).await?;
    println!("clicked at ({}, {}) on surface {}", args.x, args.y, res.surface_id);
    Ok(())
}
