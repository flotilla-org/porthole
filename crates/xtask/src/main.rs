use clap::{Parser, Subcommand, ValueEnum};
use xtask::macos_bundle::{self, BundleOptions};

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
    let result = match cli.command {
        Command::Bundle(args) => match args.platform {
            Platform::Macos => macos_bundle::run(BundleOptions {
                release: args.release,
                refresh: args.refresh,
                sign: args.sign,
            }),
        },
    };

    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
