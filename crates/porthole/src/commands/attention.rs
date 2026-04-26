use porthole_core::attention::AttentionInfo;

use crate::client::{ClientError, DaemonClient};

fn fmt_surface_id(v: &Option<porthole_core::surface::SurfaceId>) -> String {
    match v {
        Some(id) => id.as_str().to_string(),
        None => "(none)".to_string(),
    }
}

fn fmt_display_id(v: &Option<porthole_core::display::DisplayId>) -> String {
    match v {
        Some(id) => id.as_str().to_string(),
        None => "(none)".to_string(),
    }
}

fn fmt_opt_str(v: &Option<String>) -> String {
    v.as_deref().unwrap_or("(none)").to_string()
}

pub async fn run(client: &DaemonClient) -> Result<(), ClientError> {
    let info: AttentionInfo = client.get_json("/attention").await?;
    println!("focused_surface_id: {}", fmt_surface_id(&info.focused_surface_id));
    println!("focused_app_name: {}", fmt_opt_str(&info.focused_app_name));
    println!("focused_display_id: {}", fmt_display_id(&info.focused_display_id));
    println!(
        "cursor: ({:.1}, {:.1}) display_id={}",
        info.cursor.x,
        info.cursor.y,
        fmt_display_id(&info.cursor.display_id),
    );
    if info.recently_active_surface_ids.is_empty() {
        println!("recently_active: (none)");
    } else {
        let ids: Vec<String> = info.recently_active_surface_ids.iter().map(|s| s.as_str().to_string()).collect();
        println!("recently_active: {}", ids.join(", "));
    }
    Ok(())
}
