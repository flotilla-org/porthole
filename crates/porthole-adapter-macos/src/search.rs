#![cfg(target_os = "macos")]

use porthole_core::search::{encode_ref, Candidate, SearchQuery};
use porthole_core::{ErrorCode, PortholeError};
use regex::Regex;

use crate::enumerate::{list_windows, WindowRecord};

pub async fn search(query: &SearchQuery) -> Result<Vec<Candidate>, PortholeError> {
    let title_regex = match &query.title_pattern {
        Some(p) => Some(Regex::new(p).map_err(|e| {
            PortholeError::new(ErrorCode::InvalidArgument, format!("invalid title_pattern regex: {e}"))
        })?),
        None => None,
    };

    let windows = list_windows()?;
    let mut matches: Vec<WindowRecord> =
        windows.into_iter().filter(|w| matches_query(w, query, title_regex.as_ref())).collect();

    if matches!(query.frontmost, Some(true)) && !matches.is_empty() {
        // list_windows returns on-screen windows in roughly Z-order
        // (front-to-back) because CGWindowListCopyWindowInfo is called with
        // kCGWindowListOptionOnScreenOnly and no explicit reordering. Take
        // the first match, which is the frontmost.
        matches.truncate(1);
    }

    Ok(matches
        .into_iter()
        .map(|w| Candidate {
            ref_: encode_ref(w.owner_pid as u32, w.cg_window_id),
            app_name: w.app_name,
            title: w.title,
            pid: w.owner_pid as u32,
            cg_window_id: w.cg_window_id,
        })
        .collect())
}

fn matches_query(w: &WindowRecord, q: &SearchQuery, title_re: Option<&Regex>) -> bool {
    if let Some(name) = &q.app_name {
        if w.app_name.as_deref() != Some(name) {
            return false;
        }
    }
    if let Some(re) = title_re {
        let title = w.title.as_deref().unwrap_or("");
        if !re.is_match(title) {
            return false;
        }
    }
    if !q.pids.is_empty() {
        let pid_u32 = w.owner_pid as u32;
        if !q.pids.contains(&pid_u32) {
            return false;
        }
    }
    if !q.cg_window_ids.is_empty() && !q.cg_window_ids.contains(&w.cg_window_id) {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(pid: i32, cg: u32, app: Option<&str>, title: Option<&str>) -> WindowRecord {
        WindowRecord {
            cg_window_id: cg,
            owner_pid: pid,
            title: title.map(str::to_string),
            app_name: app.map(str::to_string),
        }
    }

    #[test]
    fn matches_with_empty_query() {
        let w = rec(1, 42, Some("X"), Some("t"));
        assert!(matches_query(&w, &SearchQuery::default(), None));
    }

    #[test]
    fn app_name_filter_is_exact() {
        let w = rec(1, 42, Some("Ghostty"), None);
        let q = SearchQuery { app_name: Some("Ghostty".into()), ..Default::default() };
        assert!(matches_query(&w, &q, None));
        let q = SearchQuery { app_name: Some("ghostty".into()), ..Default::default() };
        assert!(!matches_query(&w, &q, None));
    }

    #[test]
    fn pids_filter_is_or_within_list() {
        let w = rec(77, 42, None, None);
        let q = SearchQuery { pids: vec![10, 77, 99], ..Default::default() };
        assert!(matches_query(&w, &q, None));
    }

    #[test]
    fn title_pattern_compiles_and_matches() {
        let re = Regex::new("^demo-").unwrap();
        let w = rec(1, 1, None, Some("demo-terminal"));
        assert!(matches_query(&w, &SearchQuery::default(), Some(&re)));
        let w = rec(1, 1, None, Some("other"));
        assert!(!matches_query(&w, &SearchQuery::default(), Some(&re)));
    }
}
