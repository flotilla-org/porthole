use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

use crate::macos_helper;

pub const INFO_PLIST: &str = "apps/macos/bundle/Info.plist";
pub const ICON: &str = "apps/macos/bundle/Resources/icon.png";
/// Bundled LaunchAgent plist registered via SMAppService.agent so portholed
/// runs as its own launchd job and can vend the attach MachService.
pub const LAUNCH_AGENT_PLIST: &str = "apps/macos/bundle/LaunchAgents/work.flotilla.porthole.daemon.plist";
pub const LAUNCH_AGENT_PLIST_NAME: &str = "work.flotilla.porthole.daemon.plist";

#[derive(Debug)]
pub struct BundleOptions {
    pub release: bool,
    pub refresh: bool,
    pub sign: Option<String>,
}

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("{0}")]
    InvalidSigningIdentity(String),
    #[error(
        "Apple Development signing identity required for Porthole dev bundles.\n\n\
         Ad-hoc signing is a hard failure because macOS TCC grants are keyed to the\n\
         bundle's designated requirement. For ad-hoc signatures that requirement is\n\
         cdhash-based and changes on every rebuild, so Accessibility and Screen\n\
         Recording grants silently stop matching the installed app.\n\n\
         Install or import an Apple Development certificate, then rerun this command:\n\
           Xcode -> Settings -> Accounts -> Manage Certificates -> + -> Apple Development\n\
           security import <file>.p12 -k ~/Library/Keychains/login.keychain-db"
    )]
    MissingSigningIdentity,
    #[error("failed to run `{command}`: {source}")]
    CommandIo { command: String, source: io::Error },
    #[error("`{command}` failed with status {status}")]
    CommandFailed { command: String, status: String },
    #[error("{context}: {source}")]
    Io { context: String, source: io::Error },
}

pub fn app_path(profile: &str) -> PathBuf {
    Path::new("target").join(profile).join("Porthole.app")
}

pub fn profile_name(release: bool) -> &'static str {
    if release { "release" } else { "debug" }
}

pub fn build_command_args(release: bool) -> Vec<&'static str> {
    if release {
        vec!["build", "--workspace", "--locked", "--release"]
    } else {
        vec!["build", "--workspace", "--locked"]
    }
}

pub fn parse_apple_development_identity(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let start = line.find('"')?;
        let end = line.rfind('"')?;
        if end <= start {
            return None;
        }
        let identity = &line[start + 1..end];
        identity.starts_with("Apple Development:").then(|| identity.to_owned())
    })
}

pub fn validate_sign_identity(sign: Option<&str>) -> Result<Option<String>, BundleError> {
    match sign {
        Some(identity) if identity.trim().is_empty() || identity == "-" => Err(BundleError::InvalidSigningIdentity(
            "--sign requires an Apple Development signing identity; ad-hoc signing is not supported for Porthole dev bundles".to_owned(),
        )),
        Some(identity) => Ok(Some(identity.to_owned())),
        None => Ok(None),
    }
}

pub fn run(options: BundleOptions) -> Result<(), BundleError> {
    let profile = profile_name(options.release);
    let identity = choose_sign_identity(options.sign.as_deref())?;

    if !options.refresh {
        run_status("cargo", &build_command_args(options.release))?;
        run_status_owned("swift", &macos_helper::swift_build_args(options.release))?;
    }

    let daemon_bin = Path::new("target").join(profile).join("portholed");
    let cli_bin = Path::new("target").join(profile).join("porthole");
    let helper_bin = macos_helper::built_helper_path(options.release);
    ensure_file(&daemon_bin)?;
    ensure_file(&cli_bin)?;
    ensure_file(&helper_bin)?;

    let app = app_path(profile);
    if app.exists() {
        fs::remove_dir_all(&app).map_err(|source| BundleError::Io {
            context: format!("failed to remove {}", app.display()),
            source,
        })?;
    }

    let contents = app.join("Contents");
    let macos_dir = contents.join("MacOS");
    let resources_dir = contents.join("Resources");
    let launch_agents_dir = contents.join("Library").join("LaunchAgents");
    create_dir_all(&macos_dir)?;
    create_dir_all(&resources_dir)?;
    create_dir_all(&launch_agents_dir)?;

    copy_file(Path::new(INFO_PLIST), &contents.join("Info.plist"))?;
    copy_file(Path::new(ICON), &resources_dir.join("icon.png"))?;
    // Copy before codesign so the LaunchAgent plist is sealed into the app's
    // signature (SMAppService validates the bundled plist against it).
    copy_file(Path::new(LAUNCH_AGENT_PLIST), &launch_agents_dir.join(LAUNCH_AGENT_PLIST_NAME))?;
    copy_executable(&helper_bin, &macos_dir.join("PortholeHelper"))?;
    copy_executable(&daemon_bin, &macos_dir.join("portholed"))?;
    copy_executable(&cli_bin, &macos_dir.join("porthole"))?;

    let app_arg = app.to_string_lossy().to_string();
    let sign_args = ["-s", identity.as_str(), "--force", "--deep", app_arg.as_str()];
    // `--deep` is acceptable for this flat transitional development bundle.
    // Production/notarized signing should sign components in a defined order
    // before signing the app.
    run_status("codesign", &sign_args)?;

    println!("bundle mode: helper app");
    println!("bundle built: {}", app.display());
    println!("signed with:      {identity}");
    println!(
        "install/restart:   \"{}\" install --user --force",
        macos_dir.join("porthole").display()
    );
    println!("grant permissions: porthole onboard");
    println!();
    println!("Do not launch portholed directly from a terminal for onboarding; macOS may");
    println!("attribute TCC prompts to the terminal app instead of Porthole.");

    Ok(())
}

fn choose_sign_identity(sign: Option<&str>) -> Result<String, BundleError> {
    if let Some(identity) = validate_sign_identity(sign)? {
        return Ok(identity);
    }

    let output = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()
        .map_err(|source| BundleError::CommandIo {
            command: "security find-identity -v -p codesigning".to_owned(),
            source,
        })?;

    parse_apple_development_identity(&String::from_utf8_lossy(&output.stdout)).ok_or(BundleError::MissingSigningIdentity)
}

fn ensure_file(path: &Path) -> Result<(), BundleError> {
    path.is_file().then_some(()).ok_or_else(|| BundleError::Io {
        context: format!("missing binary: {}", path.display()),
        source: io::Error::new(io::ErrorKind::NotFound, "file not found"),
    })
}

fn create_dir_all(path: &Path) -> Result<(), BundleError> {
    fs::create_dir_all(path).map_err(|source| BundleError::Io {
        context: format!("failed to create {}", path.display()),
        source,
    })
}

fn copy_file(from: &Path, to: &Path) -> Result<(), BundleError> {
    fs::copy(from, to).map(|_| ()).map_err(|source| BundleError::Io {
        context: format!("failed to copy {} to {}", from.display(), to.display()),
        source,
    })
}

fn copy_executable(from: &Path, to: &Path) -> Result<(), BundleError> {
    copy_file(from, to)?;
    let mut permissions = fs::metadata(to)
        .map_err(|source| BundleError::Io {
            context: format!("failed to read permissions for {}", to.display()),
            source,
        })?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(to, permissions).map_err(|source| BundleError::Io {
        context: format!("failed to make {} executable", to.display()),
        source,
    })
}

fn run_status(command: &str, args: &[&str]) -> Result<(), BundleError> {
    let status = Command::new(command).args(args).status().map_err(|source| BundleError::CommandIo {
        command: format_command(command, args),
        source,
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(BundleError::CommandFailed {
            command: format_command(command, args),
            status: status.to_string(),
        })
    }
}

fn run_status_owned(command: &str, args: &[String]) -> Result<(), BundleError> {
    let status = Command::new(command).args(args).status().map_err(|source| BundleError::CommandIo {
        command: format_command_owned(command, args),
        source,
    })?;

    if status.success() {
        Ok(())
    } else {
        Err(BundleError::CommandFailed {
            command: format_command_owned(command, args),
            status: status.to_string(),
        })
    }
}

fn format_command(command: &str, args: &[&str]) -> String {
    std::iter::once(command).chain(args.iter().copied()).collect::<Vec<_>>().join(" ")
}

fn format_command_owned(command: &str, args: &[String]) -> String {
    std::iter::once(command.to_owned())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    /// Drift guard: the bundled daemon plist declares the attach MachService by
    /// literal string (a static plist can't reference a Rust constant), but
    /// portholed only answers on `MACOS_NATIVE_ATTACH_MACH_SERVICE`. If the two
    /// ever diverge the attach server silently won't be found, so pin them here.
    #[test]
    fn bundled_plist_declares_the_attach_mach_service() {
        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let plist = std::fs::read_to_string(workspace_root.join(super::LAUNCH_AGENT_PLIST))
            .expect("bundled daemon plist should be readable from the workspace root");
        let service = porthole_protocol::capture_sessions::MACOS_NATIVE_ATTACH_MACH_SERVICE;
        assert!(
            plist.contains(&format!("<key>{service}</key>")),
            "bundled daemon plist must declare the MachServices key {service:?} (got:\n{plist})"
        );

        // The Label must match the plist file name (sans .plist), which is the
        // launchd job id onboard's `kickstart`/`is_loaded` target. A mismatch
        // would silently misdirect restarts to a non-existent job.
        let label = super::LAUNCH_AGENT_PLIST_NAME
            .strip_suffix(".plist")
            .expect("plist name ends in .plist");
        assert!(
            plist.contains(&format!("<string>{label}</string>")),
            "bundled daemon plist Label must be {label:?} (got:\n{plist})"
        );
    }
}
