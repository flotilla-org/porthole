use std::path::{Path, PathBuf};

pub const PACKAGE_PATH: &str = "apps/macos/PortholeHelper";

pub fn swift_build_configuration(release: bool) -> &'static str {
    if release { "release" } else { "debug" }
}

pub fn scratch_path() -> PathBuf {
    Path::new("target").join("swift").join("PortholeHelper")
}

pub fn swift_build_args(release: bool) -> Vec<String> {
    vec![
        "build".to_owned(),
        "--package-path".to_owned(),
        PACKAGE_PATH.to_owned(),
        "--scratch-path".to_owned(),
        scratch_path().to_string_lossy().into_owned(),
        "-c".to_owned(),
        swift_build_configuration(release).to_owned(),
    ]
}

pub fn built_helper_path(release: bool) -> PathBuf {
    scratch_path().join(swift_build_configuration(release)).join("PortholeHelper")
}
