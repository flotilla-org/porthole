use std::path::{Path, PathBuf};

pub const INFO_PLIST: &str = "apps/macos/bundle/Info.plist";
pub const ICON: &str = "apps/macos/bundle/Resources/icon.png";

pub fn app_path(profile: &str) -> PathBuf {
    Path::new("target").join(profile).join("Porthole.app")
}
