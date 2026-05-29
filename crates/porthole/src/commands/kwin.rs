use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::Subcommand;

use crate::client::ClientError;

const SCRIPT_ID: &str = "porthole-control";
const SCRIPT_ENABLED_KEY: &str = "porthole-controlEnabled";
const SCRIPT_PACKAGE_TYPE: &str = "KWin/Script";
const SCRIPT_METADATA: &str = include_str!("../../../../apps/linux/kwin/porthole-control-script/metadata.json");
const SCRIPT_MAIN: &str = include_str!("../../../../apps/linux/kwin/porthole-control-script/contents/code/main.js");
const DESKTOP_ENTRY_ID: &str = "work.flotilla.Porthole";
const DESKTOP_ENTRY_FILENAME: &str = "work.flotilla.Porthole.desktop";

#[derive(Subcommand, Clone, Debug)]
pub enum KwinCommand {
    /// Install or upgrade the per-user KWin control script.
    InstallScript,
    /// Unload and remove the per-user KWin control script.
    UninstallScript,
    /// Print KWin control script package, config, and loaded state.
    Status,
    /// Reload the installed KWin control script.
    ReloadScript,
    /// Install the per-user KDE desktop entry that authorizes KWin screenshots.
    InstallDesktopEntry,
}

pub fn run(command: KwinCommand) -> Result<(), ClientError> {
    match command {
        KwinCommand::InstallScript => install_script(),
        KwinCommand::UninstallScript => uninstall_script(),
        KwinCommand::Status => status(),
        KwinCommand::ReloadScript => reload_script(),
        KwinCommand::InstallDesktopEntry => install_desktop_entry(),
    }
    .map_err(|error| ClientError::Local(error.to_string()))
}

fn install_script() -> Result<(), KwinError> {
    let package_dir = write_temp_package()?;
    let installed = package_installed();
    let action = if installed { "--upgrade" } else { "--install" };
    let install_result = run_checked(
        "kpackagetool6",
        &["--type", SCRIPT_PACKAGE_TYPE, action, &package_dir.display().to_string()],
    );
    let _ = fs::remove_dir_all(&package_dir);
    install_result?;

    set_enabled(true)?;
    reload_script()?;
    println!("KWin control script installed: {SCRIPT_ID}");
    Ok(())
}

fn uninstall_script() -> Result<(), KwinError> {
    let _ = unload_script();
    set_enabled(false)?;
    if package_installed() {
        run_checked("kpackagetool6", &["--type", SCRIPT_PACKAGE_TYPE, "--remove", SCRIPT_ID])?;
    }
    println!("KWin control script removed: {SCRIPT_ID}");
    Ok(())
}

fn reload_script() -> Result<(), KwinError> {
    let _ = unload_script();
    load_script()?;
    start_scripts()?;
    println!("KWin control script loaded: {SCRIPT_ID}");
    Ok(())
}

fn status() -> Result<(), KwinError> {
    let installed = package_installed();
    let enabled = read_enabled()?;
    let loaded = script_loaded()?;
    println!("kwin_script_id: {SCRIPT_ID}");
    println!("package_installed: {installed}");
    println!("kwinrc_enabled: {enabled}");
    println!("kwin_loaded: {loaded}");
    println!("package_path: {}", installed_script_dir()?.display());
    println!("desktop_entry_path: {}", desktop_entry_path()?.display());
    println!("desktop_entry_installed: {}", desktop_entry_path()?.is_file());
    Ok(())
}

fn install_desktop_entry() -> Result<(), KwinError> {
    let daemon = infer_daemon_path()?;
    let path = desktop_entry_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| KwinError::Message(format!("desktop entry path has no parent: {}", path.display())))?;
    fs::create_dir_all(parent).map_err(|source| KwinError::io(parent, source))?;
    write_file(&path, &render_desktop_entry(&daemon))?;
    run_checked("kbuildsycoca6", &["--noincremental"])?;
    println!("KDE desktop entry installed: {}", path.display());
    println!("Exec={}", daemon.display());
    Ok(())
}

fn render_desktop_entry(daemon: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Porthole\n\
         Exec={}\n\
         NoDisplay=true\n\
         X-DBUS-StartupType=Unique\n\
         X-DBUS-ServiceName={DESKTOP_ENTRY_ID}\n\
         X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2\n",
        daemon.display()
    )
}

fn infer_daemon_path() -> Result<PathBuf, KwinError> {
    let mut path = env::current_exe().map_err(|source| KwinError::Exec {
        program: "current_exe".into(),
        source,
    })?;
    match path.file_name().and_then(|name| name.to_str()) {
        Some("portholed") => Ok(path),
        Some("porthole") => {
            path.set_file_name("portholed");
            Ok(path)
        }
        _ => Err(KwinError::Message(format!(
            "cannot infer portholed path from current executable {}; run this command from the porthole CLI",
            path.display()
        ))),
    }
}

fn package_installed() -> bool {
    command_ok("kpackagetool6", &["--type", SCRIPT_PACKAGE_TYPE, "--show", SCRIPT_ID])
}

fn set_enabled(enabled: bool) -> Result<(), KwinError> {
    run_checked(
        "kwriteconfig6",
        &[
            "--file",
            "kwinrc",
            "--group",
            "Plugins",
            "--key",
            SCRIPT_ENABLED_KEY,
            if enabled { "true" } else { "false" },
        ],
    )
}

fn read_enabled() -> Result<bool, KwinError> {
    let output = run_output(
        "kreadconfig6",
        &["--file", "kwinrc", "--group", "Plugins", "--key", SCRIPT_ENABLED_KEY],
    )?;
    Ok(output.trim() == "true")
}

fn unload_script() -> Result<(), KwinError> {
    run_checked(
        "busctl",
        &[
            "--user",
            "call",
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
            "unloadScript",
            "s",
            SCRIPT_ID,
        ],
    )
}

fn load_script() -> Result<(), KwinError> {
    let script_file = installed_script_file()?;
    run_checked(
        "busctl",
        &[
            "--user",
            "call",
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
            "loadScript",
            "ss",
            &script_file.display().to_string(),
            SCRIPT_ID,
        ],
    )
}

fn start_scripts() -> Result<(), KwinError> {
    run_checked(
        "busctl",
        &["--user", "call", "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting", "start"],
    )
}

fn script_loaded() -> Result<bool, KwinError> {
    let output = run_output(
        "busctl",
        &[
            "--user",
            "call",
            "org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting",
            "isScriptLoaded",
            "s",
            SCRIPT_ID,
        ],
    )?;
    Ok(output.trim() == "b true")
}

fn installed_script_dir() -> Result<PathBuf, KwinError> {
    Ok(home()?.join(format!(".local/share/kwin/scripts/{SCRIPT_ID}")))
}

fn installed_script_file() -> Result<PathBuf, KwinError> {
    Ok(installed_script_dir()?.join("contents/code/main.js"))
}

fn desktop_entry_path() -> Result<PathBuf, KwinError> {
    Ok(home()?.join(format!(".local/share/applications/{DESKTOP_ENTRY_FILENAME}")))
}

fn write_temp_package() -> Result<PathBuf, KwinError> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| KwinError::Message(error.to_string()))?
        .as_millis();
    let root = env::temp_dir().join(format!("porthole-kwin-script-{}-{suffix}", std::process::id()));
    let code_dir = root.join("contents/code");
    fs::create_dir_all(&code_dir).map_err(|source| KwinError::io(&code_dir, source))?;
    write_file(&root.join("metadata.json"), SCRIPT_METADATA)?;
    write_file(&code_dir.join("main.js"), SCRIPT_MAIN)?;
    Ok(root)
}

fn write_file(path: &Path, contents: &str) -> Result<(), KwinError> {
    fs::write(path, contents).map_err(|source| KwinError::io(path, source))
}

fn home() -> Result<PathBuf, KwinError> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| KwinError::Message("HOME env var not set".into()))
}

fn command_ok(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn run_checked(program: &str, args: &[&str]) -> Result<(), KwinError> {
    let output = Command::new(program).args(args).output().map_err(|source| KwinError::Exec {
        program: program.into(),
        source,
    })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(KwinError::Command {
            program: program.into(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

fn run_output(program: &str, args: &[&str]) -> Result<String, KwinError> {
    let output = Command::new(program).args(args).output().map_err(|source| KwinError::Exec {
        program: program.into(),
        source,
    })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(KwinError::Command {
            program: program.into(),
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
enum KwinError {
    #[error("{0}")]
    Message(String),
    #[error("{program} exec failed: {source}")]
    Exec { program: String, source: io::Error },
    #[error("{program} exited {code:?}: {stderr}")]
    Command {
        program: String,
        code: Option<i32>,
        stderr: String,
    },
    #[error("io error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

impl KwinError {
    fn io(path: &Path, source: io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_package_metadata_names_script_id() {
        assert!(SCRIPT_METADATA.contains("\"Id\": \"porthole-control\""));
        assert!(SCRIPT_MAIN.contains("work.flotilla.Porthole.KWin"));
    }

    #[test]
    fn desktop_entry_authorizes_kwin_screenshot2() {
        let entry = render_desktop_entry(Path::new("/tmp/portholed"));

        assert!(entry.contains("Exec=/tmp/portholed"));
        assert!(entry.contains("X-DBUS-ServiceName=work.flotilla.Porthole"));
        assert!(entry.contains("X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2"));
    }
}
