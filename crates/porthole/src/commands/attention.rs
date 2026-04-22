use porthole_core::attention::AttentionInfo;

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient) -> Result<(), ClientError> {
    let info: AttentionInfo = client.get_json("/attention").await?;
    println!("focused_surface_id: {:?}", info.focused_surface_id);
    println!("focused_app_name: {:?}", info.focused_app_name);
    println!("focused_display_id: {:?}", info.focused_display_id);
    println!("cursor: ({}, {}) display_id={:?}", info.cursor.x, info.cursor.y, info.cursor.display_id);
    println!("recently_active: {:?}", info.recently_active_surface_ids);
    Ok(())
}
