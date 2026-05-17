use std::{fs, path::PathBuf};

use xtask::macos_helper::{swift_build_args, swift_build_configuration};
use xtask::macos_bundle::{build_command_args, parse_apple_development_identity, profile_name, validate_sign_identity};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("xtask crate should live under crates/xtask")
        .to_path_buf()
}

#[test]
fn helper_info_plist_uses_helper_executable() {
    let plist = fs::read_to_string(workspace_root().join("apps/macos/bundle/Info.plist")).unwrap();
    assert!(plist.contains("<key>CFBundleIdentifier</key>"));
    assert!(plist.contains("<string>org.flotilla.porthole.dev</string>"));
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
            "--scratch-path",
            "target/swift/PortholeHelper",
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
            "--scratch-path",
            "target/swift/PortholeHelper",
            "-c",
            "release",
        ]
    );
}
