//! `porthole install` / `porthole uninstall` — make the daemon ambient.
//!
//! Three concerns, in order:
//!
//! 1. **Bundle placement.** Copy the running `.app` to `/Applications/Porthole.app`
//!    (system-wide, requires admin) or `~/Applications/Porthole.app` (per-user
//!    fallback). TCC keys off bundle identity, so the install destination is
//!    where future grants will be attributed.
//! 2. **CLI on PATH.** Symlink `~/.local/bin/porthole` to the bundle's CLI so
//!    `porthole` resolves on the shell. Detect `$PATH` membership and print
//!    a one-liner the user can paste into their shell rc if missing — we don't
//!    auto-edit user dotfiles.
//! 3. **LaunchAgent.** Drop a plist into `~/Library/LaunchAgents/` and
//!    `launchctl bootstrap` it. `RunAtLoad=true` + `KeepAlive(Crashed=true)`
//!    so the daemon comes up at login and restarts on crash, scoped to the
//!    Aqua session (no headless ssh hosts).

use std::{
    env, fs, io,
    os::unix,
    path::{Path, PathBuf},
};

use crate::{
    client::ClientError,
    launchd::{self, LAUNCH_AGENT_LABEL, LaunchctlError},
};

const BUNDLE_NAME: &str = "Porthole.app";

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("not running from a .app bundle: install can only run from inside Porthole.app (got {0})")]
    NotInBundle(PathBuf),
    #[error("source and destination are the same path ({0}); install would self-delete the bundle")]
    AlreadyAtDestination(PathBuf),
    #[error("destination {0} already exists; pass --force to overwrite")]
    DestinationExists(PathBuf),
    #[error("no write permission for {0}; re-run with --user for a per-user install at ~/Applications")]
    SystemInstallNoPermission(PathBuf),
    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    Launchctl(#[from] LaunchctlError),
    #[error("HOME env var not set")]
    NoHome,
    #[error(
        "$HOME is on an external volume (plist resolves to {plist_canonical}); macOS launchd refuses to bootstrap user LaunchAgents from /Volumes/* and returns EIO.\n\n\
The plist has been written to {plist_in_home}. To finish the install, relocate it to the boot volume (one-time sudo):\n\n  \
sudo install -m 644 -o root -g wheel \\\n    \"{plist_in_home}\" \\\n    /Library/LaunchAgents/\n  \
rm \"{plist_in_home}\"\n  \
launchctl bootstrap gui/$(id -u) \"/Library/LaunchAgents/{plist_filename}\"\n\n\
The relocated plist runs as you (not root) because it's in LaunchAgents, not LaunchDaemons. Bundle and CLI stay where they are."
    )]
    PlistRequiresSystemRelocation {
        plist_in_home: PathBuf,
        plist_canonical: PathBuf,
        plist_filename: String,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum InstallPrefix {
    System,
    User,
}

impl InstallPrefix {
    fn applications_dir(self) -> Result<PathBuf, InstallError> {
        match self {
            InstallPrefix::System => Ok(PathBuf::from("/Applications")),
            InstallPrefix::User => Ok(home()?.join("Applications")),
        }
    }
}

pub struct InstallOptions {
    pub prefix: InstallPrefix,
    pub force: bool,
    pub skip_symlink: bool,
    pub skip_launch_agent: bool,
}

pub struct UninstallOptions {
    pub prefix: InstallPrefix,
    pub keep_bundle: bool,
}

/// Run `porthole install`. Returns Ok(()) on success; prints progress and the
/// PATH hint to stdout. The caller (CLI) handles exit code mapping.
pub async fn install(opts: InstallOptions) -> Result<(), ClientError> {
    do_install(opts).map_err(client_err)
}

pub async fn uninstall(opts: UninstallOptions) -> Result<(), ClientError> {
    do_uninstall(opts).map_err(client_err)
}

fn client_err(e: InstallError) -> ClientError {
    ClientError::Local(e.to_string())
}

fn do_install(opts: InstallOptions) -> Result<(), InstallError> {
    let src_bundle = locate_running_bundle()?;
    let dst_apps = opts.prefix.applications_dir()?;
    let dst_bundle = dst_apps.join(BUNDLE_NAME);

    // Guard against running install from inside the install destination —
    // remove_path on dst would delete src, then copy_dir_recursive would fail
    // with the source gone. `current_exe` returns the bundle path on macOS
    // when invoked directly (not via a symlink), so this is reachable.
    if src_bundle == dst_bundle {
        return Err(InstallError::AlreadyAtDestination(dst_bundle));
    }

    fs::create_dir_all(&dst_apps).map_err(|e| io_err(&dst_apps, e))?;

    // Probe for write permission before the expensive bundle copy. The
    // create_dir_all above is a no-op on /Applications (always exists), so
    // this is the first call that would actually fail under no-admin. Without
    // this probe the user would hit a generic "permission denied" mid-install
    // with no hint that --user is the fix.
    if matches!(opts.prefix, InstallPrefix::System) {
        check_writable(&dst_apps)?;
    }

    // Stop any prior install's daemon before touching its files. Always
    // bootout on --force, regardless of --no-launch-agent: a stale daemon
    // running on the old binary is surprising even if we're not re-registering
    // a launch agent. Bootout is a no-op if nothing's loaded.
    let plist_path = launch_agent_plist_path()?;
    if !opts.skip_launch_agent || opts.force {
        let _ = launchd::bootout(&plist_path);
    }

    if dst_bundle.exists() {
        if !opts.force {
            return Err(InstallError::DestinationExists(dst_bundle));
        }
        println!("removing existing bundle: {}", dst_bundle.display());
        remove_path(&dst_bundle)?;
    }

    println!("installing bundle: {} -> {}", src_bundle.display(), dst_bundle.display());
    copy_dir_recursive(&src_bundle, &dst_bundle)?;

    let dst_cli = dst_bundle.join("Contents/MacOS/porthole");

    if !opts.skip_symlink {
        let local_bin = home()?.join(".local/bin");
        fs::create_dir_all(&local_bin).map_err(|e| io_err(&local_bin, e))?;
        let symlink_path = local_bin.join("porthole");
        if symlink_path.exists() || symlink_path.is_symlink() {
            fs::remove_file(&symlink_path).map_err(|e| io_err(&symlink_path, e))?;
        }
        unix::fs::symlink(&dst_cli, &symlink_path).map_err(|e| io_err(&symlink_path, e))?;
        println!("symlinked: {} -> {}", symlink_path.display(), dst_cli.display());

        let path_env = env::var("PATH").unwrap_or_default();
        if !path_contains(&path_env, &local_bin) {
            println!();
            println!("Note: {} is not on your PATH.", local_bin.display());
            println!("Add to ~/.zshrc or ~/.bashrc:");
            println!("    export PATH=\"$HOME/.local/bin:$PATH\"");
            println!();
        }
    }

    if !opts.skip_launch_agent {
        let startup_program = startup_program_for_bundle(&dst_bundle);
        let log_dir = home()?.join("Library/Logs/porthole");
        fs::create_dir_all(&log_dir).map_err(|e| io_err(&log_dir, e))?;
        let plist_xml = render_launch_agent_plist(&startup_program, &log_dir.join("portholed.log"));
        if let Some(parent) = plist_path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        fs::write(&plist_path, plist_xml).map_err(|e| io_err(&plist_path, e))?;
        println!("wrote LaunchAgent: {}", plist_path.display());

        // launchd refuses to bootstrap user agents whose plist file resolves
        // under /Volumes/*, returning EIO with no useful diagnostic. This is
        // the common case when $HOME has been relocated to an external APFS
        // volume (typical on storage-constrained Mac minis). Detect that
        // here, leave the plist on disk as a reference, and tell the user how
        // to relocate it to /Library/LaunchAgents with one-time sudo. We
        // deliberately don't try to elevate ourselves — install code that
        // silently shells out to sudo is a surprise vector.
        if let Some(canonical) = canonical_plist_under_volumes(&plist_path) {
            let plist_filename = plist_path
                .file_name()
                .and_then(|n| n.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("{LAUNCH_AGENT_LABEL}.plist"));
            return Err(InstallError::PlistRequiresSystemRelocation {
                plist_in_home: plist_path,
                plist_canonical: canonical,
                plist_filename,
            });
        }

        launchd::bootstrap(&plist_path)?;
        println!("daemon registered with launchd; will start at login (and now).");
    }

    println!();
    println!("done. next: run `porthole onboard` if you haven't already to grant TCC permissions.");
    Ok(())
}

fn do_uninstall(opts: UninstallOptions) -> Result<(), InstallError> {
    let plist_path = launch_agent_plist_path()?;
    if plist_path.exists() {
        println!("unloading LaunchAgent: {}", plist_path.display());
        let _ = launchd::bootout(&plist_path);
        fs::remove_file(&plist_path).map_err(|e| io_err(&plist_path, e))?;
    }

    let symlink_path = home()?.join(".local/bin/porthole");
    if symlink_path.is_symlink() {
        println!("removing symlink: {}", symlink_path.display());
        fs::remove_file(&symlink_path).map_err(|e| io_err(&symlink_path, e))?;
    }

    if !opts.keep_bundle {
        let dst_bundle = opts.prefix.applications_dir()?.join(BUNDLE_NAME);
        if dst_bundle.exists() {
            println!("removing bundle: {}", dst_bundle.display());
            remove_path(&dst_bundle)?;
        }
    } else {
        println!("(bundle left in place)");
    }

    println!();
    println!("done. TCC grants for Porthole.app remain in System Settings;");
    println!("clear with: tccutil reset Accessibility work.flotilla.porthole.dev");
    println!("            tccutil reset ScreenCapture work.flotilla.porthole.dev");
    Ok(())
}

/// Verify we can write into `dir` by creating and removing a probe file.
/// On the system install path this catches the no-admin case before we
/// touch the existing bundle, surfacing a clear `--user` hint instead of a
/// generic mid-install permission-denied.
fn check_writable(dir: &Path) -> Result<(), InstallError> {
    let probe = dir.join(".porthole-install-probe");
    match fs::write(&probe, b"") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Err(InstallError::SystemInstallNoPermission(dir.to_path_buf())),
        Err(e) => Err(io_err(dir, e)),
    }
}

fn home() -> Result<PathBuf, InstallError> {
    env::var_os("HOME").map(PathBuf::from).ok_or(InstallError::NoHome)
}

fn launch_agent_plist_path() -> Result<PathBuf, InstallError> {
    Ok(home()?.join(format!("Library/LaunchAgents/{LAUNCH_AGENT_LABEL}.plist")))
}

fn io_err(path: &Path, source: io::Error) -> InstallError {
    InstallError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Walks up from `current_exe` to find the enclosing `.app` bundle. Returns
/// the bundle directory, or NotInBundle if there isn't one.
fn locate_running_bundle() -> Result<PathBuf, InstallError> {
    let exe = env::current_exe().map_err(|e| io_err(Path::new("<current_exe>"), e))?;
    locate_bundle_from(&exe).ok_or(InstallError::NotInBundle(exe))
}

fn locate_bundle_from(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .find(|p| p.extension().map(|e| e == "app").unwrap_or(false))
        .map(|p| p.to_path_buf())
}

fn path_contains(path_env: &str, dir: &Path) -> bool {
    path_env.split(':').any(|p| Path::new(p) == dir)
}

fn startup_program_for_bundle(bundle: &Path) -> PathBuf {
    let macos = bundle.join("Contents/MacOS");
    let helper = macos.join("PortholeHelper");
    if helper.is_file() { helper } else { macos.join("portholed") }
}

fn remove_path(p: &Path) -> Result<(), InstallError> {
    if p.is_dir() {
        fs::remove_dir_all(p).map_err(|e| io_err(p, e))
    } else {
        fs::remove_file(p).map_err(|e| io_err(p, e))
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), InstallError> {
    fs::create_dir_all(dst).map_err(|e| io_err(dst, e))?;
    for entry in fs::read_dir(src).map_err(|e| io_err(src, e))? {
        let entry = entry.map_err(|e| io_err(src, e))?;
        let entry_src = entry.path();
        let entry_dst = dst.join(entry.file_name());
        let ft = entry.file_type().map_err(|e| io_err(&entry_src, e))?;
        if ft.is_dir() {
            copy_dir_recursive(&entry_src, &entry_dst)?;
        } else if ft.is_symlink() {
            let target = fs::read_link(&entry_src).map_err(|e| io_err(&entry_src, e))?;
            unix::fs::symlink(&target, &entry_dst).map_err(|e| io_err(&entry_dst, e))?;
        } else {
            fs::copy(&entry_src, &entry_dst).map_err(|e| io_err(&entry_src, e))?;
        }
    }
    Ok(())
}

/// If `plist_path`'s canonical form resolves under `/Volumes/`, return that
/// canonical path. macOS launchd rejects user LaunchAgent plists from there
/// with an opaque EIO, regardless of file permissions or volume mount flags.
/// Canonicalize first so a symlinked `~/Library/LaunchAgents` from / to an
/// external volume is also caught.
///
/// Returns `None` if the path can't be canonicalized (caller's plist write
/// would have failed first) or the canonical path doesn't live under
/// `/Volumes/` — both are non-error outcomes.
fn canonical_plist_under_volumes(plist_path: &Path) -> Option<PathBuf> {
    canonical_plist_under_volumes_inner(plist_path, |p| fs::canonicalize(p))
}

/// Testable inner with `canonicalize` injected: the wiring of
/// `canonicalize → path_under_volumes` is what we want to pin, and we
/// can't manufacture a real path that canonicalizes to `/Volumes/*` in
/// CI without a real external mount. Tests pass a fake closure that
/// returns whatever canonical path the scenario calls for.
fn canonical_plist_under_volumes_inner<F>(plist_path: &Path, canonicalize: F) -> Option<PathBuf>
where
    F: FnOnce(&Path) -> io::Result<PathBuf>,
{
    let canonical = canonicalize(plist_path).ok()?;
    if path_under_volumes(&canonical) { Some(canonical) } else { None }
}

/// True if `path` lives under the `/Volumes/` mount-point prefix.
/// `Path::starts_with` is component-aware so `/VolumesNotAMatch/foo`
/// doesn't false-positive. Split out so the predicate is testable without
/// having to manufacture a real /Volumes/* path on disk.
fn path_under_volumes(path: &Path) -> bool {
    path.starts_with("/Volumes/")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn render_launch_agent_plist(program: &Path, log_path: &Path) -> String {
    // Paths can legally contain `&`, `<`, `>` on macOS HFS+/APFS. Without
    // escaping, those would produce malformed XML and launchctl would reject
    // the plist with a cryptic error.
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>Program</key>
    <string>{program}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>Crashed</key>
        <true/>
    </dict>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>MachServices</key>
    <dict>
        <key>{mach_service}</key>
        <true/>
    </dict>
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
        label = LAUNCH_AGENT_LABEL,
        program = xml_escape(&program.display().to_string()),
        mach_service = porthole_protocol::capture_sessions::MACOS_NATIVE_ATTACH_MACH_SERVICE,
        log = xml_escape(&log_path.display().to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_bundle_finds_app_ancestor() {
        let exe = Path::new("/Applications/Porthole.app/Contents/MacOS/porthole");
        assert_eq!(locate_bundle_from(exe), Some(PathBuf::from("/Applications/Porthole.app")));
    }

    #[test]
    fn locate_bundle_returns_none_when_not_in_app() {
        let exe = Path::new("/Users/x/.cargo/bin/porthole");
        assert_eq!(locate_bundle_from(exe), None);
    }

    #[test]
    fn locate_bundle_handles_nested_app_picks_innermost() {
        // .app inside .app — locate_bundle_from walks ancestors which goes
        // bottom-up, so the innermost .app wins. (Not a real scenario but
        // pins the semantics.)
        let exe = Path::new("/A.app/Contents/Helpers/B.app/Contents/MacOS/x");
        assert_eq!(locate_bundle_from(exe), Some(PathBuf::from("/A.app/Contents/Helpers/B.app")));
    }

    #[test]
    fn path_contains_handles_exact_match() {
        let p = "/usr/bin:/Users/x/.local/bin:/usr/local/bin";
        assert!(path_contains(p, Path::new("/Users/x/.local/bin")));
    }

    #[test]
    fn path_contains_rejects_substring_match() {
        let p = "/usr/bin:/some/.local/bin/extra";
        assert!(!path_contains(p, Path::new("/Users/x/.local/bin")));
    }

    #[test]
    fn path_contains_handles_empty_path() {
        assert!(!path_contains("", Path::new("/Users/x/.local/bin")));
    }

    #[test]
    fn xml_escape_handles_special_characters() {
        assert_eq!(xml_escape("path/with & ampersand"), "path/with &amp; ampersand");
        assert_eq!(xml_escape("a<b>c"), "a&lt;b&gt;c");
        assert_eq!(xml_escape("plain/path"), "plain/path");
        // Order matters: & must escape first, otherwise &lt; becomes &amp;lt;.
        assert_eq!(xml_escape("&<"), "&amp;&lt;");
    }

    #[test]
    fn render_plist_escapes_xml_special_chars_in_paths() {
        let plist = render_launch_agent_plist(
            Path::new("/Users/a&b/Porthole.app/Contents/MacOS/portholed"),
            Path::new("/Users/a&b/Library/Logs/porthole/portholed.log"),
        );
        assert!(plist.contains("/Users/a&amp;b/Porthole.app/Contents/MacOS/portholed"));
        assert!(!plist.contains("/Users/a&b/Porthole.app"));
    }

    #[test]
    fn render_plist_includes_program_path_and_label() {
        let plist = render_launch_agent_plist(
            Path::new("/Applications/Porthole.app/Contents/MacOS/portholed"),
            Path::new("/Users/x/Library/Logs/porthole/portholed.log"),
        );
        assert!(plist.contains("<string>work.flotilla.porthole</string>"));
        assert!(plist.contains("<string>/Applications/Porthole.app/Contents/MacOS/portholed</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(plist.contains("<key>LimitLoadToSessionType</key>\n    <string>Aqua</string>"));
        assert!(plist.contains("/Users/x/Library/Logs/porthole/portholed.log"));
    }

    #[test]
    fn startup_program_prefers_helper_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Porthole.app");
        let macos = bundle.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("PortholeHelper"), "").unwrap();
        fs::write(macos.join("portholed"), "").unwrap();

        assert_eq!(startup_program_for_bundle(&bundle), macos.join("PortholeHelper"));
    }

    #[test]
    fn startup_program_falls_back_to_daemon_for_transitional_bundle() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("Porthole.app");
        let macos = bundle.join("Contents/MacOS");
        fs::create_dir_all(&macos).unwrap();
        fs::write(macos.join("portholed"), "").unwrap();

        assert_eq!(startup_program_for_bundle(&bundle), macos.join("portholed"));
    }

    #[test]
    fn check_writable_returns_ok_for_writable_dir() {
        let tmp = tempfile::tempdir().unwrap();
        check_writable(tmp.path()).unwrap();
        // Probe must be cleaned up.
        assert!(!tmp.path().join(".porthole-install-probe").exists());
    }

    #[test]
    fn check_writable_returns_no_permission_for_readonly_dir() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let original = fs::metadata(tmp.path()).unwrap().permissions();
        // Read-only for owner: r-x------
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o500)).unwrap();

        let result = check_writable(tmp.path());

        // Restore so tempdir's Drop can clean up.
        fs::set_permissions(tmp.path(), original).unwrap();

        match result {
            Err(InstallError::SystemInstallNoPermission(_)) => {}
            other => panic!("expected SystemInstallNoPermission, got {other:?}"),
        }
    }

    #[test]
    fn system_install_no_permission_error_mentions_user_flag() {
        let err = InstallError::SystemInstallNoPermission(PathBuf::from("/Applications"));
        let msg = err.to_string();
        assert!(msg.contains("--user"), "expected --user hint, got: {msg}");
    }

    #[test]
    fn already_at_destination_error_mentions_self_delete() {
        let err = InstallError::AlreadyAtDestination(PathBuf::from("/Applications/Porthole.app"));
        let msg = err.to_string();
        assert!(msg.contains("self-delete"), "expected self-delete hint, got: {msg}");
    }

    #[test]
    fn copy_dir_recursive_copies_files_and_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), "hello").unwrap();
        fs::write(src.join("sub/b.txt"), "world").unwrap();

        copy_dir_recursive(&src, &dst).unwrap();

        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "hello");
        assert_eq!(fs::read_to_string(dst.join("sub/b.txt")).unwrap(), "world");
    }

    #[test]
    fn path_under_volumes_flags_paths_under_volumes_prefix() {
        // Positive case — the actual rule launchd enforces.
        assert!(path_under_volumes(Path::new(
            "/Volumes/MiniHomeX/Users/robert/Library/LaunchAgents"
        )));
        assert!(path_under_volumes(Path::new("/Volumes/x")));
    }

    #[test]
    fn path_under_volumes_rejects_non_volumes_paths() {
        // Boot volume paths must NOT trigger.
        assert!(!path_under_volumes(Path::new("/Users/x/Library/LaunchAgents")));
        assert!(!path_under_volumes(Path::new("/Library/LaunchAgents")));
        assert!(!path_under_volumes(Path::new("/System/Volumes/Data/Users/x")));
    }

    #[test]
    fn path_under_volumes_uses_component_aware_prefix() {
        // `/VolumesNotAMatch/foo` is not under `/Volumes/`. String-prefix
        // would false-positive here; Path::starts_with must not.
        assert!(!path_under_volumes(Path::new("/VolumesNotAMatch/foo")));
    }

    #[test]
    fn canonical_plist_under_volumes_returns_none_for_boot_volume_path() {
        // Integration check on the canonicalize half: a real tempdir file
        // (which lives on the boot volume) must canonicalize to a non-/Volumes/
        // path, so the helper returns None.
        let tmp = tempfile::tempdir().unwrap();
        let plist = tmp.path().join("x.plist");
        fs::write(&plist, "x").unwrap();
        assert_eq!(canonical_plist_under_volumes(&plist), None);
    }

    #[test]
    fn canonical_plist_under_volumes_returns_none_for_nonexistent_path() {
        // canonicalize fails on a path that doesn't exist; helper returns None.
        let result = canonical_plist_under_volumes(Path::new("/nonexistent/Library/LaunchAgents/x.plist"));
        assert_eq!(result, None);
    }

    #[test]
    fn canonical_plist_under_volumes_inner_flags_volumes_canonical() {
        // Wiring test for the canonicalize → path_under_volumes composition,
        // using an injected canonicalize. Catches regressions like passing
        // the unresolved path to path_under_volumes by accident, inverting
        // the branches, or dropping the check entirely.
        let result = canonical_plist_under_volumes_inner(Path::new("any/path"), |_| {
            Ok(PathBuf::from("/Volumes/MiniHomeX/Library/LaunchAgents/x.plist"))
        });
        assert_eq!(result, Some(PathBuf::from("/Volumes/MiniHomeX/Library/LaunchAgents/x.plist")));
    }

    #[test]
    fn canonical_plist_under_volumes_inner_returns_none_for_boot_canonical() {
        let result = canonical_plist_under_volumes_inner(Path::new("any/path"), |_| {
            Ok(PathBuf::from("/Users/x/Library/LaunchAgents/x.plist"))
        });
        assert_eq!(result, None);
    }

    #[test]
    fn canonical_plist_under_volumes_inner_returns_none_on_canonicalize_err() {
        let result = canonical_plist_under_volumes_inner(Path::new("any/path"), |_| Err(io::Error::from(io::ErrorKind::NotFound)));
        assert_eq!(result, None);
    }

    #[test]
    fn plist_requires_system_relocation_error_has_copyable_commands() {
        let err = InstallError::PlistRequiresSystemRelocation {
            plist_in_home: PathBuf::from("/Volumes/MiniHomeX/Users/robert/Library/LaunchAgents/work.flotilla.porthole.plist"),
            plist_canonical: PathBuf::from("/Volumes/MiniHomeX/Users/robert/Library/LaunchAgents/work.flotilla.porthole.plist"),
            plist_filename: "work.flotilla.porthole.plist".to_string(),
        };
        let msg = err.to_string();
        // The three commands the user needs to run, in order. Paths are
        // double-quoted so a $HOME with spaces doesn't break the copy-paste.
        assert!(
            msg.contains("sudo install -m 644 -o root -g wheel"),
            "missing sudo install line, got: {msg}"
        );
        assert!(
            msg.contains("\"/Volumes/MiniHomeX/Users/robert/Library/LaunchAgents/work.flotilla.porthole.plist\""),
            "missing quoted plist path, got: {msg}"
        );
        assert!(
            msg.contains("rm \"/Volumes/MiniHomeX/Users/robert/Library/LaunchAgents/work.flotilla.porthole.plist\""),
            "missing quoted rm line, got: {msg}"
        );
        assert!(
            msg.contains("launchctl bootstrap gui/$(id -u) \"/Library/LaunchAgents/work.flotilla.porthole.plist\""),
            "missing quoted bootstrap line, got: {msg}"
        );
        // The why, so the user understands what they're doing.
        assert!(msg.contains("external volume"), "missing 'external volume' explanation, got: {msg}");
        assert!(msg.contains("EIO"), "missing EIO mention, got: {msg}");
    }

    #[test]
    fn copy_dir_recursive_preserves_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("real"), "ok").unwrap();
        unix::fs::symlink("real", src.join("link")).unwrap();

        copy_dir_recursive(&src, &dst).unwrap();

        assert!(dst.join("link").is_symlink());
        assert_eq!(fs::read_link(dst.join("link")).unwrap(), Path::new("real"));
    }
}
