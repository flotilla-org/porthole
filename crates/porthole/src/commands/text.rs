use porthole_protocol::input::{TextRequest, TextResponse};

use crate::client::{ClientError, DaemonClient};

pub struct TextArgs {
    pub surface_id: String,
    pub text: String,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: TextArgs) -> Result<(), ClientError> {
    let req = TextRequest {
        text: args.text,
        session: args.session,
    };
    let res: TextResponse = client.post_json(&format!("/surfaces/{}/text", args.surface_id), &req).await?;
    println!("sent {} char(s) to surface {}", res.chars_sent, res.surface_id);
    Ok(())
}
