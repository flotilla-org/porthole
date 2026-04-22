use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Conditions that `wait` can block on.
///
/// `threshold_pct` on Stable and Dirty is the percentage of pixels (0.0–100.0)
/// that must differ between consecutive samples (Stable) or from the initial
/// sample (Dirty) to count as a "real" change. Tolerates things like blinking
/// terminal cursors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WaitCondition {
    Stable {
        #[serde(default = "default_stable_window_ms")]
        window_ms: u64,
        #[serde(default = "default_threshold_pct")]
        threshold_pct: f64,
    },
    Dirty {
        #[serde(default = "default_threshold_pct")]
        threshold_pct: f64,
    },
    Exists,
    Gone,
    TitleMatches {
        pattern: String,
    },
}

fn default_stable_window_ms() -> u64 {
    1500
}

fn default_threshold_pct() -> f64 {
    1.0
}

pub const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_millis(10_000);
pub const WAIT_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// Outcome of a successful wait.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaitOutcome {
    pub condition: String, // "stable" | "dirty" | "exists" | "gone" | "title_matches"
    pub elapsed_ms: u64,
}

/// Diagnostic payload returned with `wait_timeout` errors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LastObserved {
    FrameChange { last_change_ms_ago: u64, last_change_pct: f64 },
    Presence { alive: bool },
    Title { title: Option<String> },
}

/// Returned by `Adapter::wait` when the deadline passes before the condition
/// is satisfied. Contains real diagnostics populated by the adapter.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaitTimeout {
    pub last_observed: LastObserved,
    pub elapsed_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_condition_uses_defaults() {
        let json = r#"{"type":"stable"}"#;
        let c: WaitCondition = serde_json::from_str(json).unwrap();
        assert!(matches!(c, WaitCondition::Stable { window_ms: 1500, threshold_pct } if (threshold_pct - 1.0).abs() < 1e-9));
    }

    #[test]
    fn dirty_condition_uses_default_threshold() {
        let json = r#"{"type":"dirty"}"#;
        let c: WaitCondition = serde_json::from_str(json).unwrap();
        assert!(matches!(c, WaitCondition::Dirty { threshold_pct } if (threshold_pct - 1.0).abs() < 1e-9));
    }

    #[test]
    fn title_matches_requires_pattern() {
        let json = r#"{"type":"title_matches","pattern":"^foo"}"#;
        let c: WaitCondition = serde_json::from_str(json).unwrap();
        assert!(matches!(c, WaitCondition::TitleMatches { pattern } if pattern == "^foo"));
    }

    #[test]
    fn exists_and_gone_serialize_as_tagged_empties() {
        let exists_json = serde_json::to_string(&WaitCondition::Exists).unwrap();
        assert_eq!(exists_json, r#"{"type":"exists"}"#);
        let gone_json = serde_json::to_string(&WaitCondition::Gone).unwrap();
        assert_eq!(gone_json, r#"{"type":"gone"}"#);
    }
}
