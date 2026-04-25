use std::path::PathBuf;

use base64::{Engine, engine::general_purpose::STANDARD as B64};
use porthole_protocol::screenshot::{ScreenshotRequest, ScreenshotResponse};

use crate::client::{ClientError, DaemonClient};

pub struct ScreenshotArgs {
    pub surface_id: String,
    pub output: PathBuf,
    pub session: Option<String>,
}

pub async fn run(client: &DaemonClient, args: ScreenshotArgs) -> Result<(), ClientError> {
    let req = ScreenshotRequest { session: args.session };
    let res: ScreenshotResponse = client.post_json(&format!("/surfaces/{}/screenshot", args.surface_id), &req).await?;
    let bytes = B64
        .decode(&res.png_base64)
        .map_err(|e| ClientError::Local(format!("base64 decode: {e}")))?;
    std::fs::write(&args.output, &bytes).map_err(|e| ClientError::Local(format!("write {}: {e}", args.output.display())))?;
    println!("wrote {} ({} bytes)", args.output.display(), bytes.len());
    println!(
        "window_bounds: {}x{} at {},{}",
        res.window_bounds.w, res.window_bounds.h, res.window_bounds.x, res.window_bounds.y
    );
    println!("scale: {}", res.scale);
    Ok(())
}
