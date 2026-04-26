use porthole_core::search::SearchQuery;
use porthole_protocol::search::{SearchRequest, SearchResponse, TrackRequest, TrackResponse};

use crate::{
    ancestry::containing_ancestors,
    client::{ClientError, DaemonClient},
};

pub struct AttachArgs {
    pub app_name: Option<String>,
    pub title_pattern: Option<String>,
    pub pids: Vec<u32>,
    pub containing_pids: Vec<u32>,
    pub cg_window_ids: Vec<u32>,
    pub frontmost: Option<bool>,
    pub session: Option<String>,
    pub json: bool,
}

pub async fn run(client: &DaemonClient, args: AttachArgs) -> Result<(), ClientError> {
    // Union --pid and --containing-pid into a single list.
    let mut pids = args.pids.clone();
    for root in &args.containing_pids {
        pids.extend(containing_ancestors(*root));
    }
    pids.sort_unstable();
    pids.dedup();

    let query = SearchQuery {
        app_name: args.app_name,
        title_pattern: args.title_pattern,
        pids,
        cg_window_ids: args.cg_window_ids,
        frontmost: args.frontmost,
    };

    // Search, pick if unique, track.
    let search: SearchResponse = client
        .post_json(
            "/surfaces/search",
            &SearchRequest {
                query,
                session: args.session.clone(),
            },
        )
        .await?;

    if search.candidates.is_empty() {
        return Err(ClientError::Local("attach: no matching windows".to_string()));
    }
    if search.candidates.len() > 1 {
        let list = search
            .candidates
            .iter()
            .map(|c| {
                format!(
                    "  {}  pid={}  cg={}  app={:?}  title={:?}",
                    c.ref_, c.pid, c.cg_window_id, c.app_name, c.title,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(ClientError::Local(format!(
            "attach: {} candidates matched — use `porthole search` with stricter filters or pass --frontmost. Matches:\n{list}",
            search.candidates.len()
        )));
    }
    let chosen = search.candidates.into_iter().next().unwrap();
    let res: TrackResponse = client
        .post_json(
            "/surfaces/track",
            &TrackRequest {
                ref_: chosen.ref_,
                session: args.session,
            },
        )
        .await?;
    if args.json {
        let text = serde_json::to_string_pretty(&res).map_err(|e| ClientError::Local(format!("json encode: {e}")))?;
        println!("{text}");
    } else {
        println!("{}", res.surface_id);
    }
    Ok(())
}
