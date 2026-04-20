use std::path::PathBuf;

use clap::{Parser, Subcommand};
use porthole::client::DaemonClient;
use porthole::commands::launch::LaunchArgs;
use porthole::commands::screenshot::ScreenshotArgs;
use porthole::runtime::socket_path;
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
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
