use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use voicebridge::{bind, list, run};

/// Speech to EDCoPilot commands.
#[derive(Parser)]
#[command(name = "voicebridge", version)]
struct Cli {
    /// The configuration file.
    #[arg(long, global = true, default_value = "voicebridge.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the configuration problems and the load report.
    Check {
        /// Print one line for each command that the program left out of
        /// the commands of the EDCoPilot profile.
        #[arg(long)]
        left_out: bool,
    },
    /// Match a phrase. Without --send it is a dry run.
    Text {
        phrase: String,
        /// Write the command token to the request file.
        #[arg(long)]
        send: bool,
    },
    /// Print the input devices.
    Devices,
    /// Print the pressed keys and joystick buttons until Ctrl+C.
    Inputs,
    /// Set the push-to-talk key or joystick button: press it, Esc cancels.
    Bind,
    /// List the commands that work. FILTER words narrow the list.
    Commands {
        /// Words that the phrase, the token or the category must have.
        filter: Vec<String>,
        /// Also list the commands that cannot fire, with the reason.
        #[arg(long)]
        all: bool,
    },
    /// Run WAV files through the engine and the matcher as a dry run.
    Replay {
        /// A WAV file (16 kHz mono, 32-bit float or 16-bit) or a directory of WAV files.
        path: PathBuf,
    },
    /// Start the voice session.
    Run {
        /// Match, but do not write the request file.
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Check { left_out } => run::check(&cli.config, left_out),
        Command::Text { phrase, send } => run::text(&cli.config, &phrase, send),
        Command::Devices => Ok(run::devices()),
        Command::Inputs => run::inputs(),
        Command::Bind => bind::bind(&cli.config),
        Command::Commands { filter, all } => list::commands(&cli.config, &filter.join(" "), all),
        Command::Replay { path } => run::replay(&cli.config, &path),
        Command::Run { dry_run } => run::run(&cli.config, dry_run),
    };
    result.unwrap_or_else(|e| {
        eprintln!("error: {e:#}");
        ExitCode::FAILURE
    })
}
