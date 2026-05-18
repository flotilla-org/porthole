use std::process::Command;

#[test]
fn status_reports_down_with_socket_path() {
    let tmp = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_porthole"))
        .env("PORTHOLE_RUNTIME_DIR", tmp.path())
        .arg("status")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("daemon: down"), "{stdout}");
    assert!(
        stdout.contains(&format!("socket: {}", tmp.path().join("porthole.sock").display())),
        "{stdout}"
    );
}
