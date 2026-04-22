use serde::{Deserialize, Serialize};

pub use porthole_core::search::{Candidate, SearchQuery};

#[derive(Clone, Debug, Deserialize)]
pub struct SearchRequest {
    #[serde(flatten)]
    pub query: SearchQuery,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchResponse {
    pub candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TrackRequest {
    #[serde(rename = "ref")]
    pub ref_: String,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TrackResponse {
    pub surface_id: String,
    pub cg_window_id: u32,
    pub pid: u32,
    pub app_name: Option<String>,
    pub title: Option<String>,
    pub reused_existing_handle: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_request_flattens_query_fields() {
        let json = r#"{"app_name":"Ghostty","pids":[1,2],"frontmost":true}"#;
        let req: SearchRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.query.app_name, Some("Ghostty".into()));
        assert_eq!(req.query.pids, vec![1, 2]);
        assert_eq!(req.query.frontmost, Some(true));
    }

    #[test]
    fn track_request_uses_ref_wire_field() {
        let json = r#"{"ref":"ref_abc"}"#;
        let req: TrackRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.ref_, "ref_abc");
    }

    #[test]
    fn track_response_roundtrip() {
        let r = TrackResponse {
            surface_id: "surf_1".into(),
            cg_window_id: 42,
            pid: 9876,
            app_name: Some("Ghostty".into()),
            title: Some("t".into()),
            reused_existing_handle: true,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"reused_existing_handle\":true"));
    }
}
