use std::process::Command;

#[test]
fn status_reports_down_with_control_endpoint() {
    let tmp = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_porthole"));
    #[cfg(unix)]
    let endpoint = {
        command.env("PORTHOLE_RUNTIME_DIR", tmp.path());
        tmp.path().join("porthole.sock").display().to_string()
    };
    #[cfg(windows)]
    let endpoint = {
        // Override only this child's naming identity so a running user daemon
        // cannot turn this daemon-down test into a probe of a real session.
        let identity = format!("status-test-{}", tmp.path().file_name().unwrap().to_string_lossy());
        command.env("USERNAME", &identity);
        format!(r"\\.\pipe\porthole-{identity}")
    };
    let output = command.arg("status").output().unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("daemon: down"), "{stdout}");
    assert!(stdout.contains(&format!("socket: {endpoint}")), "{stdout}");
}
