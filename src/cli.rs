use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct CliArgs {
    pub source_path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum CliCommand {
    Run(CliArgs),
    Help(String),
    Version(String),
}

pub fn parse_cli_args() -> Result<CliCommand> {
    let mut source_path: Option<PathBuf> = None;

    for arg in env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                return Ok(CliCommand::Help(help_text()));
            }
            "--version" | "-V" => {
                return Ok(CliCommand::Version(format!(
                    "rep {}",
                    env!("CARGO_PKG_VERSION")
                )));
            }
            _ if arg.starts_with('-') => {
                anyhow::bail!("unknown option: {arg}\nusage: rep <markdown-file-path>");
            }
            _ => {
                if source_path.is_some() {
                    anyhow::bail!(
                        "expected a single markdown file path\nusage: rep <markdown-file-path>"
                    );
                }
                source_path = Some(PathBuf::from(arg));
            }
        }
    }

    let source_path = source_path.context("usage: rep <markdown-file-path>")?;
    Ok(CliCommand::Run(CliArgs { source_path }))
}

fn help_text() -> String {
    format!(
        "rep {} - Collaboratively Tag Text Tool

Usage: rep [OPTIONS] <markdown-file>

Arguments:
  <markdown-file>   Path to the Markdown file to annotate

Options:
  -h, --help        Print this help message
  -V, --version     Print version",
        env!("CARGO_PKG_VERSION")
    )
}
