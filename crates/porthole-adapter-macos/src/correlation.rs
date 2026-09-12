use std::collections::HashSet;

use porthole_core::{ErrorCode, PortholeError};

use crate::enumerate::WindowRecord;

/// Only the application PID returned by LaunchServices establishes ownership.
/// Wait until the deadline before accepting an old window: a new one may appear.
pub(crate) fn select_window(
    pid: i32,
    before: &HashSet<(i32, u32)>,
    windows: &[WindowRecord],
    allow_existing: bool,
) -> Result<Option<(WindowRecord, bool)>, PortholeError> {
    let owned: Vec<_> = windows.iter().filter(|w| w.owner_pid == pid).collect();
    let fresh: Vec<_> = owned
        .iter()
        .copied()
        .filter(|w| !before.contains(&(w.owner_pid, w.cg_window_id)))
        .collect();
    let (candidates, preexisting) = if fresh.is_empty() && allow_existing {
        (&owned, true)
    } else {
        (&fresh, false)
    };
    match candidates.as_slice() {
        [] => Ok(None),
        [window] => Ok(Some(((*window).clone(), preexisting))),
        _ => Err(PortholeError::new(
            ErrorCode::LaunchCorrelationAmbiguous,
            format!("launched application PID {pid} owns multiple matching windows"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(pid: i32, id: u32) -> WindowRecord {
        WindowRecord {
            cg_window_id: id,
            owner_pid: pid,
            title: None,
            app_name: None,
        }
    }

    #[test]
    fn launch_pid_correlates_without_an_environment_tag() {
        let result = select_window(42, &HashSet::new(), &[window(7, 1), window(42, 2)], false)
            .unwrap()
            .unwrap();
        assert_eq!(result.0.cg_window_id, 2);
        assert!(!result.1);
    }

    #[test]
    fn fresh_window_wins_over_existing_window_of_same_app() {
        let result = select_window(42, &HashSet::from([(42, 1)]), &[window(42, 1), window(42, 2)], true)
            .unwrap()
            .unwrap();
        assert_eq!(result.0.cg_window_id, 2);
        assert!(!result.1);
    }

    #[test]
    fn existing_window_is_deferred_and_marked_preexisting() {
        let before = HashSet::from([(42, 1)]);
        let windows = [window(42, 1)];
        assert!(select_window(42, &before, &windows, false).unwrap().is_none());
        assert!(select_window(42, &before, &windows, true).unwrap().unwrap().1);
    }

    #[test]
    fn multiple_fresh_windows_are_ambiguous() {
        let error = select_window(42, &HashSet::new(), &[window(42, 1), window(42, 2)], false).unwrap_err();
        assert_eq!(error.code, ErrorCode::LaunchCorrelationAmbiguous);
    }

    #[test]
    fn multiple_existing_windows_are_ambiguous_at_deadline() {
        let windows = [window(42, 1), window(42, 2)];
        let before = HashSet::from([(42, 1), (42, 2)]);
        assert!(select_window(42, &before, &windows, false).unwrap().is_none());
        assert_eq!(
            select_window(42, &before, &windows, true).unwrap_err().code,
            ErrorCode::LaunchCorrelationAmbiguous
        );
    }

    #[test]
    fn unrelated_windows_never_satisfy_launch() {
        assert!(select_window(42, &HashSet::new(), &[window(7, 1)], true).unwrap().is_none());
    }
}
