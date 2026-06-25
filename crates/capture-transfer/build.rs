fn main() {
    // The macOS native backend shim: compiled only when the feature is on
    // AND the target is macOS (the feature may be enabled by a dependent on
    // any platform; it must be a no-op elsewhere).
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let backend_macos = std::env::var_os("CARGO_FEATURE_BACKEND_MACOS").is_some();
    let backend_linux = std::env::var_os("CARGO_FEATURE_BACKEND_LINUX").is_some();
    if target_os == "macos" && backend_macos {
        build_macos_shim();
    }
    if target_os == "linux" && backend_linux {
        build_linux_shim();
    }
}

#[cfg(feature = "backend-macos")]
fn build_macos_shim() {
    println!("cargo:rerun-if-changed=src/native/macos_shim.m");
    println!("cargo:rerun-if-changed=src/native/macos_xpc_shim.m");
    println!("cargo:rerun-if-changed=include/capture_transfer.h");
    println!("cargo:rerun-if-changed=src/native/c_abi_header_smoke.c");
    cc::Build::new()
        .include("include")
        .file("src/native/c_abi_header_smoke.c")
        .compile("porthole_capture_transfer_c_abi_header_smoke");
    cc::Build::new()
        .include("include")
        .file("src/native/macos_shim.m")
        .file("src/native/macos_xpc_shim.m")
        .flag("-fobjc-arc")
        .flag("-mmacosx-version-min=13.0")
        .compile("porthole_native_macos");
    println!("cargo:rustc-link-lib=framework=Foundation");
    println!("cargo:rustc-link-lib=framework=Metal");
    println!("cargo:rustc-link-lib=framework=IOSurface");
    println!("cargo:rustc-link-lib=framework=CoreFoundation");
}

#[cfg(not(feature = "backend-macos"))]
fn build_macos_shim() {
    unreachable!("CARGO_FEATURE_BACKEND_MACOS implies the feature is enabled");
}

#[cfg(feature = "backend-linux")]
fn build_linux_shim() {
    println!("cargo:rerun-if-changed=src/native/linux_drm_shim.c");
    println!("cargo:rerun-if-changed=src/native/linux_vulkan_shim.c");
    println!("cargo:rerun-if-changed=src/native/linux_pipewire_shim.c");
    println!("cargo:rerun-if-changed=include/capture_transfer.h");
    println!("cargo:rerun-if-changed=src/native/c_abi_header_smoke.c");
    let mut build = cc::Build::new();
    build
        .include("include")
        .file("src/native/c_abi_header_smoke.c")
        .file("src/native/linux_drm_shim.c")
        .file("src/native/linux_vulkan_shim.c")
        .file("src/native/linux_pipewire_shim.c");
    add_pkg_config_to_cc(&mut build, "libpipewire-0.3");
    build.compile("porthole_native_linux");
    emit_pkg_config_link("libpipewire-0.3");
    println!("cargo:rustc-link-lib=dylib=dl");
}

#[cfg(not(feature = "backend-linux"))]
fn build_linux_shim() {
    unreachable!("CARGO_FEATURE_BACKEND_LINUX implies the feature is enabled");
}

#[cfg(feature = "backend-linux")]
fn pkg_config(package: &str, args: &[&str]) -> Vec<String> {
    let output = std::process::Command::new("pkg-config")
        .args(args)
        .arg(package)
        .output()
        .unwrap_or_else(|error| panic!("failed to run pkg-config for {package}: {error}"));
    if !output.status.success() {
        panic!(
            "pkg-config failed for {package}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("pkg-config output for {package} was not UTF-8: {error}"))
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

#[cfg(feature = "backend-linux")]
fn add_pkg_config_to_cc(build: &mut cc::Build, package: &str) {
    for flag in pkg_config(package, &["--cflags"]) {
        if let Some(path) = flag.strip_prefix("-I") {
            build.include(path);
        } else if let Some(define) = flag.strip_prefix("-D") {
            let (name, value) = define.split_once('=').unwrap_or((define, "1"));
            build.define(name, Some(value));
        } else {
            build.flag(&flag);
        }
    }
}

#[cfg(feature = "backend-linux")]
fn emit_pkg_config_link(package: &str) {
    for flag in pkg_config(package, &["--libs"]) {
        if let Some(path) = flag.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={path}");
        } else if let Some(lib) = flag.strip_prefix("-l") {
            println!("cargo:rustc-link-lib=dylib={lib}");
        } else if let Some(path) = flag.strip_prefix("-Wl,-rpath,") {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
        }
    }
}
