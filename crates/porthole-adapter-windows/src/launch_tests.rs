use std::{
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
};

use super::*;

#[test]
fn creation_times_reject_recycled_parent_and_child_links() {
    assert!(valid_birth(100, 0, 200, 300));
    assert!(valid_birth(100, 250, 200, 300));
    assert!(!valid_birth(200, 0, 100, 300), "child predates recycled parent");
    assert!(!valid_birth(100, 150, 200, 300), "child postdates parent's exit");
    assert!(!valid_birth(100, 0, 400, 300), "child PID recycled after snapshot started");
    assert!(!valid_birth(100, 0, 100, 300), "equal birth times cannot prove ancestry");
    assert!(!valid_birth(100, 200, 200, 300));
    assert!(!valid_birth(100, 0, 300, 300));
}

#[test]
fn disappearing_launch_candidates_retry_but_permission_failures_abort() {
    let adapter = WindowsAdapter::new();
    assert!(launch_candidate(adapter.identify(null_mut())).unwrap().is_none());
    let window = HiddenWindow::new();
    let surface = adapter.identify(window.0).unwrap();
    assert!(launch_candidate(Ok(surface.clone())).unwrap().is_some());
    drop(window);
    let stale = adapter.resolve(&surface).map(|_| surface);
    assert!(launch_candidate(stale).unwrap().is_none());
    let denied = PortholeError::new(ErrorCode::SystemPermissionNeeded, "permission required");
    assert_eq!(launch_candidate(Err(denied)).unwrap_err().code, ErrorCode::SystemPermissionNeeded);
}

const ROLE: &str = "PORTHOLE_CORRELATION_FIXTURE_ROLE";
const DIRECTORY: &str = "PORTHOLE_CORRELATION_FIXTURE_DIRECTORY";

fn fixture(role: &str, directory: &Path) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "native::launch_tests::native_launch_fixture", "--ignored", "--nocapture"])
        .env(ROLE, role)
        .env(DIRECTORY, directory)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "native launch fixture timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn marker(directory: &Path, name: &str) {
    std::fs::write(directory.join(name), []).unwrap();
}

fn publish(directory: &Path, name: &str, contents: String) {
    let temporary = directory.join(format!("{name}.tmp"));
    std::fs::write(&temporary, contents).unwrap();
    std::fs::rename(temporary, directory.join(name)).unwrap();
}

struct HiddenWindow(HWND);
impl HiddenWindow {
    fn new() -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("launch-correlation-fixture").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                32,
                32,
                null_mut(),
                null_mut(),
                null_mut(),
                null_mut(),
            )
        };
        assert!(!hwnd.is_null());
        Self(hwnd)
    }
}
impl Drop for HiddenWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}

// Re-executed only by this module. All fixture windows stay hidden and all
// fixture processes exit on their own; no existing GUI process is touched.
#[test]
#[ignore = "subprocess fixture, invoked by native descendant regression"]
#[expect(
    clippy::zombie_processes,
    reason = "Windows fixture parents must exit first; the controller waits on held process handles"
)]
fn native_launch_fixture() {
    let Ok(role) = std::env::var(ROLE) else { return };
    let directory = PathBuf::from(std::env::var_os(DIRECTORY).unwrap());
    match role.as_str() {
        "root" => {
            let _middle = fixture("middle", &directory);
            wait_for(|| directory.join("exit-root").exists());
        }
        "middle" => {
            publish(&directory, "middle-pid", std::process::id().to_string());
            wait_for(|| directory.join("spawn-leaf").exists());
            let _leaf = fixture("leaf", &directory);
            wait_for(|| directory.join("leaf-ready").exists());
            wait_for(|| directory.join("exit-middle").exists());
        }
        "leaf" => {
            let first = HiddenWindow::new();
            let second = HiddenWindow::new();
            publish(
                &directory,
                "leaf-ready",
                format!("{} {} {}", std::process::id(), first.0 as usize, second.0 as usize),
            );
            wait_for(|| directory.join("stop-leaf").exists());
        }
        _ => panic!("unexpected native fixture role"),
    }
}

struct FixtureDirectory(PathBuf);
impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        // Release only our own short-lived fixtures, including on assertion failure.
        for name in ["exit-root", "spawn-leaf", "exit-middle", "stop-leaf"] {
            let _ = std::fs::write(self.0.join(name), []);
        }
    }
}

#[test]
fn native_descendant_windows_survive_observed_ancestor_exit() {
    let directory = FixtureDirectory(std::env::temp_dir().join(format!("porthole-correlation-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&directory.0).unwrap();
    // This test process/window predates the launched root and is unrelated.
    let unrelated = HiddenWindow::new();
    let mut root = fixture("root", &directory.0);
    let mut tree = LaunchTree::new(&root).unwrap();
    wait_for(|| directory.0.join("middle-pid").exists());
    let middle: u32 = std::fs::read_to_string(directory.0.join("middle-pid")).unwrap().parse().unwrap();
    wait_for(|| {
        tree.discover().unwrap();
        tree.processes.contains_key(&middle)
    });

    // Inject a stale parent link to a real older process. Native creation times
    // must reject it even though the snapshot numerically claims our root.
    tree.observe(
        u64::MAX,
        &[ProcessLink {
            pid: std::process::id(),
            parent: root.id(),
        }],
    )
    .unwrap();
    assert!(!tree.processes.contains_key(&std::process::id()));
    assert!(!tree.owns_window(unrelated.0 as usize));

    marker(&directory.0, "exit-root");
    wait_for(|| root.try_wait().unwrap().is_some());
    assert!(!tree.processes[&root.id()].alive());
    // The leaf is created after the root exits, then the already-observed
    // intermediate exits before we take the next descendant snapshot.
    marker(&directory.0, "spawn-leaf");
    marker(&directory.0, "exit-middle");
    wait_for(|| !tree.processes[&middle].alive());
    let ready: Vec<usize> = std::fs::read_to_string(directory.0.join("leaf-ready"))
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    let leaf = ready[0] as u32;
    let (first, second) = (ready[1], ready[2]);
    wait_for(|| {
        tree.discover().unwrap();
        tree.processes.contains_key(&leaf)
    });
    assert!(tree.owns_window(first));
    assert_eq!(tree.unique_window([unrelated.0 as usize, first]).unwrap(), Some(first));
    assert_eq!(
        tree.unique_window([first, second]).unwrap_err().code,
        ErrorCode::LaunchCorrelationAmbiguous
    );
    assert_eq!(tree.unique_window([unrelated.0 as usize, 0]).unwrap(), None);
    assert!(
        !windows().unwrap().contains(&first),
        "hidden fixture is never a production visible candidate"
    );

    let adapter = WindowsAdapter::new();
    let surface = adapter.identify(first as HWND).unwrap();
    assert_eq!(surface.pid, Some(leaf));
    assert_eq!(adapter.resolve(&surface).unwrap() as usize, first);
    marker(&directory.0, "stop-leaf");
    wait_for(|| !tree.processes[&leaf].alive());
    assert!(!tree.owns_window(first));
    assert_eq!(adapter.resolve(&surface).unwrap_err().code, ErrorCode::SurfaceDead);
}

#[test]
fn native_snapshot_order_does_not_drop_grandchildren() {
    // Deterministic reversed links to real, simultaneously live processes.
    let directory = FixtureDirectory(std::env::temp_dir().join(format!("porthole-correlation-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&directory.0).unwrap();
    let mut root = fixture("root", &directory.0);
    let mut tree = LaunchTree::new(&root).unwrap();
    wait_for(|| directory.0.join("middle-pid").exists());
    let middle: u32 = std::fs::read_to_string(directory.0.join("middle-pid")).unwrap().parse().unwrap();
    // A descendant born after a claimed snapshot cannot be accepted.
    tree.observe(
        tree.processes[&root.id()].created,
        &[ProcessLink {
            pid: middle,
            parent: root.id(),
        }],
    )
    .unwrap();
    assert!(!tree.processes.contains_key(&middle));
    marker(&directory.0, "spawn-leaf");
    wait_for(|| directory.0.join("leaf-ready").exists());
    let ready: Vec<usize> = std::fs::read_to_string(directory.0.join("leaf-ready"))
        .unwrap()
        .split_whitespace()
        .map(|s| s.parse().unwrap())
        .collect();
    let leaf = ready[0] as u32;
    let (started, _) = process_links().unwrap();
    // Without the intermediate link, the grandchild alone proves nothing.
    tree.observe(started, &[ProcessLink { pid: leaf, parent: middle }]).unwrap();
    assert!(!tree.processes.contains_key(&leaf));
    tree.observe(
        started,
        &[
            ProcessLink { pid: leaf, parent: middle },
            ProcessLink {
                pid: middle,
                parent: root.id(),
            },
        ],
    )
    .unwrap();
    assert!(tree.owns_window(ready[1]));
    marker(&directory.0, "exit-root");
    marker(&directory.0, "exit-middle");
    marker(&directory.0, "stop-leaf");
    wait_for(|| root.try_wait().unwrap().is_some() && !tree.processes[&leaf].alive() && !tree.processes[&middle].alive());
}
