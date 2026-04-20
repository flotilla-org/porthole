use clap::{Parser, Subcommand};
use porthole::client::DaemonClient;
use porthole::runtime::socket_path;

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
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let client = DaemonClient::new(socket_path());
    let result = match cli.command {
        Command::Info => porthole::commands::info::run(&client).await,
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
