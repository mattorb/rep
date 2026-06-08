use anyhow::Result;
use std::env;
use std::ffi::OsString;

use rep::cli::{CliCommand, parse_cli_args, run_interactive};
use rep::ui;

mod terminal_fallback;

fn main() {
    if let Err(err) = real_main() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let raw_args: Vec<OsString> = env::args_os().skip(1).collect();
    let cli = match parse_cli_args()? {
        CliCommand::Run(cli) => cli,
        CliCommand::Help(text) | CliCommand::Version(text) => {
            println!("{text}");
            return Ok(());
        }
    };
    if !ui::terminal_available() && terminal_fallback::try_launch(&raw_args)? {
        return Ok(());
    }

    if let Some(output) = run_interactive(cli.source_path)? {
        println!("{output}");
    }
    Ok(())
}
