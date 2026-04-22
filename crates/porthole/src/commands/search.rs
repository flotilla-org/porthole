use porthole_core::search::SearchQuery;
use porthole_protocol::search::{SearchRequest, SearchResponse};

use crate::client::{ClientError, DaemonClient};

pub struct SearchArgs {
    pub app_name: Option<String>,
    pub title_pattern: Option<String>,
    pub pids: Vec<u32>,
    pub cg_window_ids: Vec<u32>,
    pub frontmost: Option<bool>,
    pub session: Option<String>,
    pub json: bool,
}

pub async fn run(client: &DaemonClient, args: SearchArgs) -> Result<(), ClientError> {
    let query = SearchQuery {
        app_name: args.app_name,
        title_pattern: args.title_pattern,
        pids: args.pids,
        cg_window_ids: args.cg_window_ids,
        frontmost: args.frontmost,
    };
    let req = SearchRequest { query, session: args.session };
    let res: SearchResponse = client.post_json("/surfaces/search", &req).await?;
    if args.json {
        let text = serde_json::to_string_pretty(&res.candidates)
            .map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        for c in &res.candidates {
            println!(
                "{}  pid={}  cg={}  app={}  title={}",
                c.ref_,
                c.pid,
                c.cg_window_id,
                c.app_name.as_deref().unwrap_or("-"),
                c.title.as_deref().unwrap_or("-"),
            );
        }
    }
    Ok(())
}
