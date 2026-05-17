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
#[command(about = "Build a platform app bundle")]
struct BundleArgs {
    #[arg(long, value_enum, help = "Platform bundle to build")]
    platform: Platform,
    #[arg(long, help = "Build release profile instead of debug")]
    release: bool,
    #[arg(long, help = "Skip cargo build and rebuild/sign the app bundle from existing target binaries")]
    refresh: bool,
    #[arg(long, help = "Apple Development signing identity to use")]
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
