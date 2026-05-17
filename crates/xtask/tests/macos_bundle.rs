use std::{fs, path::PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("xtask crate should live under crates/xtask")
        .to_path_buf()
}

#[test]
fn transitional_info_plist_keeps_daemon_executable() {
    let plist = fs::read_to_string(workspace_root().join("apps/macos/bundle/Info.plist")).unwrap();
    assert!(plist.contains("<key>CFBundleIdentifier</key>"));
    assert!(plist.contains("<string>org.flotilla.porthole.dev</string>"));
    assert!(plist.contains("<key>CFBundleExecutable</key>"));
    assert!(plist.contains("<string>portholed</string>"));
    assert!(plist.contains("<key>LSBackgroundOnly</key>"));
    assert!(!plist.contains("PortholeHelper"));
}

#[test]
fn macos_bundle_icon_input_exists() {
    assert!(
        workspace_root()
            .join("apps/macos/bundle/Resources/icon.png")
            .is_file()
    );
}
