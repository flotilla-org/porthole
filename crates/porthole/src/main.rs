use std::path::PathBuf;

use clap::{Parser, Subcommand};
use porthole::client::DaemonClient;
use porthole::commands::launch::LaunchArgs;
use porthole::commands::screenshot::ScreenshotArgs;
use porthole::runtime::socket_path;
use porthole_core::input::{ClickButton, Modifier};
use porthole_core::wait::WaitCondition;
use porthole::commands::{attention, click as click_cmd, close as close_cmd, displays, focus as focus_cmd, key as key_cmd, scroll as scroll_cmd, text as text_cmd, wait as wait_cmd};
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
    /// Launch a process.
    Launch {
        /// App bundle path or executable.
        #[arg(long)]
        app: String,
        /// Extra arguments passed to the app.
        #[arg(long = "arg", value_name = "ARG")]
        args: Vec<String>,
        /// `KEY=VALUE` env vars.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Working directory.
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
enum ModifierArg { Cmd, Ctrl, Alt, Shift }

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
enum ButtonArg { Left, Right, Middle }

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
enum ConditionArg { Stable, Dirty, Exists, Gone, TitleMatches }

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let client = DaemonClient::new(socket_path());
    let result = match cli.command {
        Command::Info => porthole::commands::info::run(&client).await,
        Command::Launch { app, args, env, cwd, session, timeout_ms, require_confidence } => {
            let parsed_env: Vec<(String, String)> = env
                .into_iter()
                .filter_map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect();
            porthole::commands::launch::run(
                &client,
                LaunchArgs {
                    app,
                    args,
                    env: parsed_env,
                    cwd,
                    session,
                    timeout_ms,
                    require_confidence: require_confidence.into(),
                },
            )
            .await
        }
        Command::Screenshot { surface_id, out, session } => {
            porthole::commands::screenshot::run(&client, ScreenshotArgs { surface_id, output: out, session }).await
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
            click_cmd::run(&client, click_cmd::ClickArgs {
                surface_id, x, y,
                button: button.into(),
                count,
                modifiers: modifiers.into_iter().map(Modifier::from).collect(),
                session,
            }).await
        }
        Command::Scroll { surface_id, x, y, delta_x, delta_y, session } => {
            scroll_cmd::run(&client, scroll_cmd::ScrollArgs { surface_id, x, y, delta_x, delta_y, session }).await
        }
        Command::Wait { surface_id, condition, pattern, window_ms, threshold_pct, timeout_ms, session } => {
            let cond = match condition {
                ConditionArg::Stable => WaitCondition::Stable { window_ms, threshold_pct },
                ConditionArg::Dirty => WaitCondition::Dirty { threshold_pct },
                ConditionArg::Exists => WaitCondition::Exists,
                ConditionArg::Gone => WaitCondition::Gone,
                ConditionArg::TitleMatches => WaitCondition::TitleMatches {
                    pattern: pattern.unwrap_or_default(),
                },
            };
            wait_cmd::run(&client, wait_cmd::WaitArgs { surface_id, condition: cond, timeout_ms, session }).await
        }
        Command::Close { surface_id, session } => close_cmd::run(&client, surface_id, session).await,
        Command::Focus { surface_id, session } => focus_cmd::run(&client, surface_id, session).await,
        Command::Attention => attention::run(&client).await,
        Command::Displays => displays::run(&client).await,
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
