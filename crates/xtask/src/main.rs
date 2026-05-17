use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "xtask")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Bundle(BundleArgs),
}

#[derive(Debug, Parser)]
struct BundleArgs {
    #[arg(long, value_enum)]
    platform: Platform,
    #[arg(long)]
    release: bool,
    #[arg(long)]
    refresh: bool,
    #[arg(long)]
    sign: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum Platform {
    Macos,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Bundle(args) => {
            println!(
                "bundle platform={:?} profile={}",
                args.platform,
                if args.release { "release" } else { "debug" }
            );
        }
    }
}
