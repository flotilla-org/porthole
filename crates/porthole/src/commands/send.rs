//! `porthole send` — high-level convenience: type the text, then press Enter.
//!
//! Mirrors cleat's `cleat send <id> 'echo hi'` — the most common terminal
//! automation primitive. `--no-enter` skips the trailing Enter, leaving the
//! command unsubmitted (useful for setting up an editor buffer or letting the
//! caller chain a follow-up `send-keys`).

use porthole_core::input::KeyEvent;
use porthole_protocol::input::{KeyRequest, KeyResponse, TextRequest, TextResponse};

use crate::client::{ClientError, DaemonClient};

pub struct SendArgs {
    pub surface_id: String,
    pub text: String,
    pub no_enter: bool,
    pub session: Option<String>,
}

/// Partial-failure note: this is two HTTP calls (text, then Enter) — there's
/// no atomic "type-and-submit" endpoint on the daemon. If the /text call
/// succeeds but the /key call fails (UDS hiccup, daemon restart mid-call),
/// the text is on screen but unsubmitted. Caller can recover by calling
/// `interrupt` to clear the line and retrying. Atomicity would mean a new
/// daemon endpoint that batches the operations server-side; not worth the
/// wire-shape expansion for an edge case the existing primitives recover
/// from cleanly.
pub async fn run(client: &DaemonClient, args: SendArgs) -> Result<(), ClientError> {
    let text_req = TextRequest {
        text: args.text,
        session: args.session.clone(),
    };
    let text_res: TextResponse = client.post_json(&format!("/surfaces/{}/text", args.surface_id), &text_req).await?;

    if args.no_enter {
        println!("send: surface {} — {} char(s) (no Enter)", text_res.surface_id, text_res.chars_sent);
        return Ok(());
    }

    let key_req = KeyRequest {
        events: vec![KeyEvent {
            key: "Enter".into(),
            modifiers: vec![],
        }],
        session: args.session,
    };
    let _: KeyResponse = client.post_json(&format!("/surfaces/{}/key", args.surface_id), &key_req).await?;
    println!("send: surface {} — {} char(s) + Enter", text_res.surface_id, text_res.chars_sent);
    Ok(())
}
