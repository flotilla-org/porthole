//! `porthole interrupt` — Ctrl+C convenience. Mirrors `cleat interrupt`.

use porthole_core::input::{KeyEvent, Modifier};
use porthole_protocol::input::{KeyRequest, KeyResponse};

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient, surface_id: String, session: Option<String>) -> Result<(), ClientError> {
    let req = KeyRequest {
        events: vec![KeyEvent {
            key: "KeyC".into(),
            modifiers: vec![Modifier::Ctrl],
        }],
        session,
    };
    let res: KeyResponse = client.post_json(&format!("/surfaces/{surface_id}/key"), &req).await?;
    println!("interrupt: sent Ctrl+C to surface {}", res.surface_id);
    Ok(())
}
