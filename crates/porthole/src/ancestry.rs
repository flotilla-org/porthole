//! Walks up a process ancestry chain via `/bin/ps -o ppid= -p <pid>`.
//!
//! Best-effort: returns whatever ancestors could be walked. Failures mid-walk
//! log via `tracing::warn!` and the partial chain is returned.

use std::process::Command;

const MAX_DEPTH: usize = 128;

/// Returns the ancestry chain starting from `pid`. Includes `pid` itself as
/// the first element, followed by parent, grandparent, etc. Stops at PID 1,
/// at `MAX_DEPTH`, or at the first `ps` failure.
pub fn containing_ancestors(pid: u32) -> Vec<u32> {
    let mut out = vec![pid];
    let mut current = pid;
    for _ in 0..MAX_DEPTH {
        if current <= 1 {
            break;
        }
        match parent_of(current) {
            Some(parent) if parent != current => {
                out.push(parent);
                current = parent;
            }
            Some(_) => break, // self-loop guard
            None => break,
        }
    }
    out
}

fn parent_of(pid: u32) -> Option<u32> {
    let output = Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output();
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(pid, error = %e, "ancestry: ps invocation failed");
            return None;
        }
    };
    if !output.status.success() {
        tracing::warn!(pid, status = ?output.status, "ancestry: ps exited non-zero");
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim();
    match trimmed.parse::<u32>() {
        Ok(ppid) => Some(ppid),
        Err(_) => {
            tracing::warn!(pid, output = %trimmed, "ancestry: could not parse ppid");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_from_current_process_returns_self_plus_ancestors() {
        let me = std::process::id();
        let chain = containing_ancestors(me);
        assert!(!chain.is_empty());
        assert_eq!(chain[0], me);
        // Most likely there is at least one ancestor (the test runner / shell).
        // Don't assert exact depth — depends on runner.
    }

    #[test]
    fn walk_stops_at_pid_1() {
        let chain = containing_ancestors(1);
        assert_eq!(chain, vec![1]);
    }

    #[test]
    fn walk_on_nonexistent_pid_returns_just_the_pid() {
        // Pick a PID that's very unlikely to exist. If ps fails, we still
        // return the seed pid.
        let chain = containing_ancestors(999_999_999);
        assert_eq!(chain[0], 999_999_999);
    }
}
