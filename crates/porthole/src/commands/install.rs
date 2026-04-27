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
    process::Command,
};

use crate::client::ClientError;

const LAUNCH_AGENT_LABEL: &str = "org.flotilla.porthole";
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
    #[error("launchctl {action} failed (exit {code:?}): {stderr}")]
    Launchctl {
        action: &'static str,
        code: Option<i32>,
        stderr: String,
    },
    #[error("HOME env var not set")]
    NoHome,
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
        let _ = launchctl_bootout(&plist_path);
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
        let daemon_path = dst_bundle.join("Contents/MacOS/portholed");
        let log_dir = home()?.join("Library/Logs/porthole");
        fs::create_dir_all(&log_dir).map_err(|e| io_err(&log_dir, e))?;
        let plist_xml = render_launch_agent_plist(&daemon_path, &log_dir.join("portholed.log"));
        if let Some(parent) = plist_path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        fs::write(&plist_path, plist_xml).map_err(|e| io_err(&plist_path, e))?;
        println!("wrote LaunchAgent: {}", plist_path.display());
        launchctl_bootstrap(&plist_path)?;
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
        let _ = launchctl_bootout(&plist_path);
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
    println!("clear with: tccutil reset Accessibility org.flotilla.porthole.dev");
    println!("            tccutil reset ScreenCapture org.flotilla.porthole.dev");
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
    <key>StandardOutPath</key>
    <string>{log}</string>
    <key>StandardErrorPath</key>
    <string>{log}</string>
</dict>
</plist>
"#,
        label = LAUNCH_AGENT_LABEL,
        program = xml_escape(&program.display().to_string()),
        log = xml_escape(&log_path.display().to_string()),
    )
}

fn current_uid() -> u32 {
    // SAFETY: getuid() has no preconditions and always succeeds on POSIX.
    unsafe { libc_getuid() }
}

unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

fn launchctl_bootstrap(plist_path: &Path) -> Result<(), InstallError> {
    let target = format!("gui/{}", current_uid());
    let output = Command::new("launchctl")
        .args(["bootstrap", &target])
        .arg(plist_path)
        .output()
        .map_err(|e| io_err(Path::new("launchctl"), e))?;
    if !output.status.success() {
        return Err(InstallError::Launchctl {
            action: "bootstrap",
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(())
}

fn launchctl_bootout(plist_path: &Path) -> Result<(), InstallError> {
    let target = format!("gui/{}", current_uid());
    let output = Command::new("launchctl")
        .args(["bootout", &target])
        .arg(plist_path)
        .output()
        .map_err(|e| io_err(Path::new("launchctl"), e))?;
    // Ignore non-zero exits — launchctl bootout exits 113/EALREADY when the
    // service isn't loaded, which is the expected case on a fresh install
    // and on the second call within an idempotent re-install. Exec failure
    // (launchctl missing entirely) does still surface above as an Io error.
    let _ = output;
    Ok(())
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
        assert!(plist.contains("<string>org.flotilla.porthole</string>"));
        assert!(plist.contains("<string>/Applications/Porthole.app/Contents/MacOS/portholed</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(plist.contains("<key>LimitLoadToSessionType</key>\n    <string>Aqua</string>"));
        assert!(plist.contains("/Users/x/Library/Logs/porthole/portholed.log"));
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
