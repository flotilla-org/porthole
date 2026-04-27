//! `porthole send-keys` — tmux-style key-sequence dispatcher.
//!
//! Parses the argv tokens via `key_syntax`, then dispatches each token to the
//! matching wire endpoint: `KeyToken::Text` → `POST /text`, `KeyToken::Key`
//! → `POST /key`. Adjacent text fragments are pre-coalesced by the parser, so
//! `send-keys SID hello world Enter` is two HTTP calls (one /text, one /key)
//! rather than three.

use std::time::Duration;

use porthole_core::input::KeyEvent;
use porthole_protocol::input::{KeyRequest, KeyResponse, TextRequest, TextResponse};

use crate::{
    client::{ClientError, DaemonClient},
    key_syntax::{self, KeyToken},
};

pub struct SendKeysArgs {
    pub surface_id: String,
    pub tokens: Vec<String>,
    pub literal: bool,
    pub repeat: u32,
    /// Optional inter-event delay. CGEvent.post can drop events when fired
    /// faster than the focused app drains them; a small pause between
    /// dispatched tokens makes input land reliably for fast-typing flows.
    pub inter_event_delay_ms: u64,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: SendKeysArgs) -> Result<(), ClientError> {
    if args.repeat == 0 {
        return Err(ClientError::Local("repeat must be at least 1".into()));
    }

    let parsed = if args.literal {
        vec![key_syntax::parse_literal(&args.tokens)]
    } else {
        key_syntax::parse_tokens(&args.tokens).map_err(|e| ClientError::Local(format!("send-keys: {e}")))?
    };

    let delay = Duration::from_millis(args.inter_event_delay_ms);
    let mut total_chars = 0usize;
    let mut total_keys = 0usize;
    // Track whether we've dispatched anything yet across all repeats. The
    // delay should fire between *every* dispatched event, including across
    // repeat boundaries and within single-token sequences (--repeat 5 on a
    // single key needs the inter-event pause too).
    let mut first_dispatch = true;
    for _ in 0..args.repeat {
        for tok in parsed.iter() {
            if !first_dispatch && !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            first_dispatch = false;
            match tok {
                KeyToken::Text(s) => {
                    let req = TextRequest {
                        text: s.clone(),
                        session: args.session.clone(),
                    };
                    let res: TextResponse = client.post_json(&format!("/surfaces/{}/text", args.surface_id), &req).await?;
                    total_chars += res.chars_sent as usize;
                }
                KeyToken::Key { name, modifiers } => {
                    let req = KeyRequest {
                        events: vec![KeyEvent {
                            key: name.clone(),
                            modifiers: modifiers.clone(),
                        }],
                        session: args.session.clone(),
                    };
                    let res: KeyResponse = client.post_json(&format!("/surfaces/{}/key", args.surface_id), &req).await?;
                    total_keys += res.events_sent as usize;
                }
            }
        }
    }

    println!(
        "send-keys: surface {} — {} char(s), {} key event(s)",
        args.surface_id, total_chars, total_keys
    );
    Ok(())
}
