use std::path::PathBuf;

use clap::{Parser, Subcommand};
use porthole::client::DaemonClient;
use porthole::commands::launch::LaunchArgs;
use porthole::commands::screenshot::ScreenshotArgs;
use porthole::runtime::socket_path;
use porthole_core::display::Rect;
use porthole_core::input::{ClickButton, Modifier};
use porthole_core::placement::{Anchor, DisplayTarget, PlacementSpec};
use porthole_core::wait::WaitCondition;
use porthole::commands::{
    attention, click as click_cmd, close as close_cmd, displays, focus as focus_cmd,
    key as key_cmd, launch as launch_cmd, replace as replace_cmd, scroll as scroll_cmd,
    text as text_cmd, wait as wait_cmd,
};
use porthole_protocol::launches::WireConfidence;

#[derive(Parser)]
#[command(version, about = "porthole — OS-level presentation substrate")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print daemon info and loaded adapters.
    Info,
    /// Launch a process or an artifact.
    Launch {
        /// "process" or "artifact". Default "process".
        #[arg(long, value_enum, default_value_t = LaunchKindArg::Process)]
        kind: LaunchKindArg,
        /// For process launches: an app bundle path (.app) or executable path. For artifact launches: a file path.
        #[arg(long = "app")]
        app: String,
        /// Process: extra args (repeatable).
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        /// Process: KEY=VALUE env vars.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Process: working directory.
        #[arg(long)]
        cwd: Option<String>,
        /// Session tag.
        #[arg(long)]
        session: Option<String>,
        /// Launch timeout in milliseconds.
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        /// Minimum required correlation confidence.
        #[arg(long, value_enum, default_value_t = ConfidenceArg::Strong)]
        require_confidence: ConfidenceArg,
        /// Fail if a preexisting surface is returned instead of a fresh one.
        #[arg(long)]
        require_fresh_surface: bool,
        /// Placement: which display ("focused", "primary", or a display ID).
        #[arg(long, value_parser = parse_display_target)]
        on_display: Option<DisplayTarget>,
        /// Placement: x position (display-local logical points).
        #[arg(long)]
        geom_x: Option<f64>,
        /// Placement: y position.
        #[arg(long)]
        geom_y: Option<f64>,
        /// Placement: width.
        #[arg(long)]
        geom_w: Option<f64>,
        /// Placement: height.
        #[arg(long)]
        geom_h: Option<f64>,
        /// Placement: anchor strategy when no explicit geometry.
        #[arg(long, value_enum)]
        anchor: Option<AnchorArg>,
        /// Auto-dismiss delay in milliseconds.
        #[arg(long)]
        auto_dismiss_ms: Option<u64>,
        /// Print response as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Replace a tracked surface — close the old, launch the new in its slot.
    Replace {
        /// Surface to replace.
        surface_id: String,
        /// "process" or "artifact". Default "process".
        #[arg(long, value_enum, default_value_t = LaunchKindArg::Process)]
        kind: LaunchKindArg,
        /// For process launches: an app bundle path (.app) or executable path. For artifact launches: a file path.
        #[arg(long = "app")]
        app: String,
        /// Process: extra args (repeatable).
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        /// Process: KEY=VALUE env vars.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Process: working directory.
        #[arg(long)]
        cwd: Option<String>,
        /// Session tag.
        #[arg(long)]
        session: Option<String>,
        /// Launch timeout in milliseconds.
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        /// Minimum required correlation confidence.
        #[arg(long, value_enum, default_value_t = ConfidenceArg::Strong)]
        require_confidence: ConfidenceArg,
        /// Fail if a preexisting surface is returned instead of a fresh one.
        #[arg(long)]
        require_fresh_surface: bool,
        /// Placement: which display ("focused", "primary", or a display ID).
        #[arg(long, value_parser = parse_display_target, conflicts_with = "inherit_placement")]
        on_display: Option<DisplayTarget>,
        /// Placement: x position (display-local logical points).
        #[arg(long)]
        geom_x: Option<f64>,
        /// Placement: y position.
        #[arg(long)]
        geom_y: Option<f64>,
        /// Placement: width.
        #[arg(long)]
        geom_w: Option<f64>,
        /// Placement: height.
        #[arg(long)]
        geom_h: Option<f64>,
        /// Placement: anchor strategy when no explicit geometry.
        #[arg(long, value_enum, conflicts_with = "inherit_placement")]
        anchor: Option<AnchorArg>,
        /// Auto-dismiss delay in milliseconds.
        #[arg(long)]
        auto_dismiss_ms: Option<u64>,
        /// Omit placement block entirely — inherit geometry from the old surface.
        #[arg(long, conflicts_with_all = ["on_display", "geom_x", "anchor"])]
        inherit_placement: bool,
        /// Print response as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Screenshot a surface.
    Screenshot {
        /// Surface id returned by `launch`.
        surface_id: String,
        /// Output path (PNG).
        #[arg(long)]
        out: PathBuf,
        /// Session tag.
        #[arg(long)]
        session: Option<String>,
    },
    /// Send key events to a surface.
    Key {
        surface_id: String,
        #[arg(long)]
        key: String,
        #[arg(long = "mod", value_enum)]
        modifiers: Vec<ModifierArg>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Type literal text into a surface.
    Text {
        surface_id: String,
        text: String,
        #[arg(long)]
        session: Option<String>,
    },
    /// Click at window-local coordinates.
    Click {
        surface_id: String,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long, value_enum, default_value_t = ButtonArg::Left)]
        button: ButtonArg,
        #[arg(long, default_value_t = 1)]
        count: u8,
        #[arg(long = "mod", value_enum)]
        modifiers: Vec<ModifierArg>,
        #[arg(long)]
        session: Option<String>,
    },
    /// Scroll at window-local coordinates.
    Scroll {
        surface_id: String,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long, default_value_t = 0.0)]
        delta_x: f64,
        #[arg(long, default_value_t = 0.0)]
        delta_y: f64,
        #[arg(long)]
        session: Option<String>,
    },
    /// Wait for a condition on a surface.
    Wait {
        surface_id: String,
        #[arg(long, value_enum)]
        condition: ConditionArg,
        #[arg(long)]
        pattern: Option<String>,
        #[arg(long, default_value_t = 1500)]
        window_ms: u64,
        #[arg(long, default_value_t = 1.0)]
        threshold_pct: f64,
        #[arg(long, default_value_t = 10_000)]
        timeout_ms: u64,
        #[arg(long)]
        session: Option<String>,
    },
    /// Close a surface.
    Close {
        surface_id: String,
        #[arg(long)]
        session: Option<String>,
    },
    /// Focus a surface.
    Focus {
        surface_id: String,
        #[arg(long)]
        session: Option<String>,
    },
    /// Print focus / cursor / recently active.
    Attention,
    /// Print monitor list.
    Displays,
    /// Search for candidate windows.
    Search {
        #[arg(long)]
        app_name: Option<String>,
        #[arg(long)]
        title_pattern: Option<String>,
        #[arg(long = "pid")]
        pids: Vec<u32>,
        #[arg(long = "cg-window-id")]
        cg_window_ids: Vec<u32>,
        #[arg(long)]
        frontmost: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Track a candidate ref, minting a surface handle.
    Track {
        #[arg(value_name = "REF")]
        ref_: String,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Search + pick-if-unique + track. Exits non-zero on 0 or >1 matches.
    Attach {
        #[arg(long)]
        app_name: Option<String>,
        #[arg(long)]
        title_pattern: Option<String>,
        #[arg(long = "pid")]
        pids: Vec<u32>,
        #[arg(long = "containing-pid")]
        containing_pids: Vec<u32>,
        #[arg(long = "cg-window-id")]
        cg_window_ids: Vec<u32>,
        #[arg(long)]
        frontmost: bool,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ConfidenceArg {
    Strong,
    Plausible,
    Weak,
}

impl From<ConfidenceArg> for WireConfidence {
    fn from(c: ConfidenceArg) -> Self {
        match c {
            ConfidenceArg::Strong => WireConfidence::Strong,
            ConfidenceArg::Plausible => WireConfidence::Plausible,
            ConfidenceArg::Weak => WireConfidence::Weak,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum LaunchKindArg {
    Process,
    Artifact,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum AnchorArg {
    FocusedDisplay,
    Cursor,
}

impl From<AnchorArg> for Anchor {
    fn from(a: AnchorArg) -> Self {
        match a {
            AnchorArg::FocusedDisplay => Anchor::FocusedDisplay,
            AnchorArg::Cursor => Anchor::Cursor,
        }
    }
}

/// Validates that geometry flags are either all provided or all absent.
/// A partial set (e.g. three of four flags) produces a clear error rather than
/// silently discarding the partial input.
fn require_full_geometry(
    x: Option<f64>,
    y: Option<f64>,
    w: Option<f64>,
    h: Option<f64>,
) -> Result<Option<Rect>, String> {
    match (x, y, w, h) {
        (None, None, None, None) => Ok(None),
        (Some(x), Some(y), Some(w), Some(h)) => Ok(Some(Rect { x, y, w, h })),
        _ => Err(
            "partial geometry: must specify all of --geom-x, --geom-y, --geom-w, --geom-h together"
                .into(),
        ),
    }
}

fn parse_display_target(s: &str) -> Result<DisplayTarget, String> {
    Ok(match s {
        "focused" => DisplayTarget::Focused,
        "primary" => DisplayTarget::Primary,
        _ => DisplayTarget::Id(porthole_core::display::DisplayId::new(s)),
    })
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ModifierArg {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

impl From<ModifierArg> for Modifier {
    fn from(m: ModifierArg) -> Self {
        match m {
            ModifierArg::Cmd => Modifier::Cmd,
            ModifierArg::Ctrl => Modifier::Ctrl,
            ModifierArg::Alt => Modifier::Alt,
            ModifierArg::Shift => Modifier::Shift,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ButtonArg {
    Left,
    Right,
    Middle,
}

impl From<ButtonArg> for ClickButton {
    fn from(b: ButtonArg) -> Self {
        match b {
            ButtonArg::Left => ClickButton::Left,
            ButtonArg::Right => ClickButton::Right,
            ButtonArg::Middle => ClickButton::Middle,
        }
    }
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum ConditionArg {
    Stable,
    Dirty,
    Exists,
    Gone,
    TitleMatches,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let client = DaemonClient::new(socket_path());
    let result = match cli.command {
        Command::Info => porthole::commands::info::run(&client).await,
        Command::Launch {
            kind,
            app,
            args,
            env,
            cwd,
            session,
            timeout_ms,
            require_confidence,
            require_fresh_surface,
            on_display,
            geom_x,
            geom_y,
            geom_w,
            geom_h,
            anchor,
            auto_dismiss_ms,
            json,
        } => {
            let kind_arg = match kind {
                LaunchKindArg::Process => {
                    let parsed_env: Vec<(String, String)> = env
                        .into_iter()
                        .filter_map(|s| {
                            s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))
                        })
                        .collect();
                    launch_cmd::LaunchKindArg::Process {
                        app,
                        args,
                        env: parsed_env,
                        cwd,
                    }
                }
                LaunchKindArg::Artifact => launch_cmd::LaunchKindArg::Artifact {
                    path: std::path::PathBuf::from(app),
                },
            };

            let geometry = match require_full_geometry(geom_x, geom_y, geom_w, geom_h) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("error: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            };
            let placement = if on_display.is_some() || geometry.is_some() || anchor.is_some() {
                Some(PlacementSpec { on_display, geometry, anchor: anchor.map(Anchor::from) })
            } else {
                None
            };

            launch_cmd::run(
                &client,
                LaunchArgs {
                    kind: kind_arg,
                    session,
                    timeout_ms,
                    require_confidence: require_confidence.into(),
                    require_fresh_surface,
                    placement,
                    auto_dismiss_after_ms: auto_dismiss_ms,
                    json,
                },
            )
            .await
        }
        Command::Replace {
            surface_id,
            kind,
            app,
            args,
            env,
            cwd,
            session,
            timeout_ms,
            require_confidence,
            require_fresh_surface,
            on_display,
            geom_x,
            geom_y,
            geom_w,
            geom_h,
            anchor,
            auto_dismiss_ms,
            inherit_placement,
            json,
        } => {
            let wire_kind = match kind {
                LaunchKindArg::Process => {
                    let parsed_env: std::collections::BTreeMap<String, String> = env
                        .into_iter()
                        .filter_map(|s| {
                            s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string()))
                        })
                        .collect();
                    porthole_protocol::launches::LaunchKind::Process(
                        porthole_protocol::launches::ProcessLaunch {
                            app,
                            args,
                            cwd,
                            env: parsed_env,
                        },
                    )
                }
                LaunchKindArg::Artifact => porthole_protocol::launches::LaunchKind::Artifact(
                    porthole_protocol::launches::ArtifactLaunch { path: app },
                ),
            };

            let geometry = match require_full_geometry(geom_x, geom_y, geom_w, geom_h) {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("error: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            };
            let placement = if inherit_placement {
                // Explicit inheritance: send placement: null so the daemon
                // inherits geometry from the old surface.
                None
            } else if on_display.is_some() || geometry.is_some() || anchor.is_some() {
                Some(PlacementSpec { on_display, geometry, anchor: anchor.map(Anchor::from) })
            } else {
                // No flags and no --inherit-placement: OS default (empty placement block).
                Some(PlacementSpec::default())
            };

            let req = porthole_protocol::launches::LaunchRequest {
                kind: wire_kind,
                session,
                require_confidence: require_confidence.into(),
                timeout_ms,
                placement,
                auto_dismiss_after_ms: auto_dismiss_ms,
                require_fresh_surface,
            };
            replace_cmd::run(&client, surface_id, req, json).await
        }
        Command::Screenshot { surface_id, out, session } => {
            porthole::commands::screenshot::run(
                &client,
                ScreenshotArgs { surface_id, output: out, session },
            )
            .await
        }
        Command::Key { surface_id, key, modifiers, session } => {
            let args = key_cmd::KeyArgs {
                surface_id,
                key,
                modifiers: modifiers.into_iter().map(Modifier::from).collect(),
                session,
            };
            key_cmd::run(&client, args).await
        }
        Command::Text { surface_id, text, session } => {
            text_cmd::run(&client, text_cmd::TextArgs { surface_id, text, session }).await
        }
        Command::Click { surface_id, x, y, button, count, modifiers, session } => {
            click_cmd::run(
                &client,
                click_cmd::ClickArgs {
                    surface_id,
                    x,
                    y,
                    button: button.into(),
                    count,
                    modifiers: modifiers.into_iter().map(Modifier::from).collect(),
                    session,
                },
            )
            .await
        }
        Command::Scroll { surface_id, x, y, delta_x, delta_y, session } => {
            scroll_cmd::run(
                &client,
                scroll_cmd::ScrollArgs { surface_id, x, y, delta_x, delta_y, session },
            )
            .await
        }
        Command::Wait { surface_id, condition, pattern, window_ms, threshold_pct, timeout_ms, session } => {
            let cond = match condition {
                ConditionArg::Stable => WaitCondition::Stable { window_ms, threshold_pct },
                ConditionArg::Dirty => WaitCondition::Dirty { threshold_pct },
                ConditionArg::Exists => WaitCondition::Exists,
                ConditionArg::Gone => WaitCondition::Gone,
                ConditionArg::TitleMatches => {
                    WaitCondition::TitleMatches { pattern: pattern.unwrap_or_default() }
                }
            };
            wait_cmd::run(&client, wait_cmd::WaitArgs { surface_id, condition: cond, timeout_ms, session })
                .await
        }
        Command::Close { surface_id, session } => {
            close_cmd::run(&client, surface_id, session).await
        }
        Command::Focus { surface_id, session } => {
            focus_cmd::run(&client, surface_id, session).await
        }
        Command::Attention => attention::run(&client).await,
        Command::Displays => displays::run(&client).await,
        Command::Search { app_name, title_pattern, pids, cg_window_ids, frontmost, session, json } => {
            use porthole::commands::search as search_cmd;
            search_cmd::run(
                &client,
                search_cmd::SearchArgs {
                    app_name,
                    title_pattern,
                    pids,
                    cg_window_ids,
                    frontmost: if frontmost { Some(true) } else { None },
                    session,
                    json,
                },
            )
            .await
        }
        Command::Track { ref_, session, json } => {
            use porthole::commands::track as track_cmd;
            track_cmd::run(&client, track_cmd::TrackArgs { ref_, session, json }).await
        }
        Command::Attach {
            app_name,
            title_pattern,
            pids,
            containing_pids,
            cg_window_ids,
            frontmost,
            session,
            json,
        } => {
            use porthole::commands::attach as attach_cmd;
            attach_cmd::run(
                &client,
                attach_cmd::AttachArgs {
                    app_name,
                    title_pattern,
                    pids,
                    containing_pids,
                    cg_window_ids,
                    frontmost: if frontmost { Some(true) } else { None },
                    session,
                    json,
                },
            )
            .await
        }
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
