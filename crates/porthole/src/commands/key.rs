use porthole_core::input::{KeyEvent, Modifier};
use porthole_protocol::input::{KeyRequest, KeyResponse};

use crate::client::{ClientError, DaemonClient};

pub struct KeyArgs {
    pub surface_id: String,
    pub key: String,
    pub modifiers: Vec<Modifier>,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: KeyArgs) -> Result<(), ClientError> {
    let req = KeyRequest {
        events: vec![KeyEvent {
            key: args.key,
            modifiers: args.modifiers,
        }],
        session: args.session,
    };
    let res: KeyResponse = client.post_json(&format!("/surfaces/{}/key", args.surface_id), &req).await?;
    println!("sent {} event(s) to surface {}", res.events_sent, res.surface_id);
    Ok(())
}
