use porthole_protocol::attention::DisplaysResponse;

use crate::client::{ClientError, DaemonClient};

pub async fn run(client: &DaemonClient) -> Result<(), ClientError> {
    let res: DisplaysResponse = client.get_json("/displays").await?;
    for d in res.displays {
        println!(
            "{}  bounds=({}, {}, {}x{})  scale={}  primary={}  focused={}",
            d.id.as_str(),
            d.bounds.x,
            d.bounds.y,
            d.bounds.w,
            d.bounds.h,
            d.scale,
            d.primary,
            d.focused,
        );
    }
    Ok(())
}
