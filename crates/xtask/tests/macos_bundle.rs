use std::{fs, path::PathBuf, process::Command};

use xtask::{
    macos_bundle::{build_command_args, parse_apple_development_identity, profile_name, validate_sign_identity},
    macos_helper::{swift_build_args, swift_build_configuration},
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("xtask crate should live under crates/xtask")
        .to_path_buf()
}

#[test]
fn refresh_requires_bridge_before_replacing_existing_bundle() {
    let root = tempfile::tempdir().unwrap();
    for path in [
        PathBuf::from("target/debug/portholed"),
        PathBuf::from("target/debug/porthole"),
        xtask::macos_helper::built_helper_path(false),
        PathBuf::from("target/debug/Porthole.app/keep-existing-bundle"),
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "existing").unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["bundle", "--platform", "macos", "--refresh", "--sign", "test identity"])
        .env_remove("JACKSTAY_BRIDGE_BIN")
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("missing binary:") && stderr.contains("jackstay-bridge"), "{stderr}");
    assert_eq!(
        fs::read_to_string(root.path().join("target/debug/Porthole.app/keep-existing-bundle")).unwrap(),
        "existing"
    );
}

#[test]
fn helper_info_plist_uses_helper_executable() {
    let plist = fs::read_to_string(workspace_root().join("apps/macos/bundle/Info.plist")).unwrap();
    assert!(plist.contains("<key>CFBundleIdentifier</key>"));
    assert!(plist.contains("<string>work.flotilla.porthole.dev</string>"));
    assert!(plist.contains("<key>CFBundleExecutable</key>"));
    assert!(plist.contains("<string>PortholeHelper</string>"));
    assert!(plist.contains("<key>LSUIElement</key>"));
    assert!(plist.contains("<true/>"));
    assert!(!plist.contains("<key>LSBackgroundOnly</key>"));
}

#[test]
fn macos_bundle_icon_input_exists() {
    assert!(workspace_root().join("apps/macos/bundle/Resources/icon.png").is_file());
}

#[test]
fn profile_name_defaults_to_debug() {
    assert_eq!(profile_name(false), "debug");
    assert_eq!(profile_name(true), "release");
}

#[test]
fn build_command_for_debug_workspace() {
    assert_eq!(build_command_args(false), vec!["build", "--workspace", "--locked"]);
}

#[test]
fn build_command_for_release_workspace() {
    assert_eq!(build_command_args(true), vec!["build", "--workspace", "--locked", "--release"]);
}

#[test]
fn parses_first_apple_development_identity() {
    let output = r#"
  1) ABCDEF1234567890 "Developer ID Application: Example Corp (1234567890)"
  2) FEDCBA0987654321 "Apple Development: Alice Example (ABCDE12345)"
  3) 1111111111111111 "Apple Development: Bob Example (ABCDE12345)"
     3 valid identities found
"#;

    assert_eq!(
        parse_apple_development_identity(output).as_deref(),
        Some("Apple Development: Alice Example (ABCDE12345)")
    );
}

#[test]
fn ignores_adhoc_and_non_apple_development_identities() {
    let output = r#"
  1) 0000000000000000 "-"
  2) ABCDEF1234567890 "Developer ID Application: Example Corp (1234567890)"
"#;

    assert_eq!(parse_apple_development_identity(output), None);
}

#[test]
fn rejects_adhoc_explicit_signing_identity() {
    assert!(validate_sign_identity(Some("-")).is_err());
    assert!(validate_sign_identity(Some("")).is_err());
    assert!(validate_sign_identity(Some("Apple Development: Alice Example (ABCDE12345)")).is_ok());
}

#[test]
fn swift_build_configuration_tracks_rust_profile() {
    assert_eq!(swift_build_configuration(false), "debug");
    assert_eq!(swift_build_configuration(true), "release");
}

#[test]
fn swift_build_uses_package_path_and_scratch_path() {
    assert_eq!(
        swift_build_args(false),
        vec![
            "build",
            "--package-path",
            "apps/macos/PortholeHelper",
            "--product",
            "PortholeHelper",
            "--scratch-path",
            &std::path::Path::new("target")
                .join("swift")
                .join("PortholeHelper")
                .to_string_lossy(),
            "-c",
            "debug",
        ]
    );
}

#[test]
fn swift_build_release_uses_release_configuration() {
    assert_eq!(
        swift_build_args(true),
        vec![
            "build",
            "--package-path",
            "apps/macos/PortholeHelper",
            "--product",
            "PortholeHelper",
            "--scratch-path",
            &std::path::Path::new("target")
                .join("swift")
                .join("PortholeHelper")
                .to_string_lossy(),
            "-c",
            "release",
        ]
    );
}
