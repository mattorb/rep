use std::env;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};

use crate::app::App;
use crate::ui::Tui;

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

/// Run the interactive TUI for a parsed CLI source path.
///
/// Returns `Some(output)` when the session should print the human-readable
/// action block after exit, or `None` for silent quit.
pub fn run_interactive(source_path: PathBuf) -> Result<Option<String>> {
    let mut app = App::load(source_path)?;
    run_tui(&mut app)?;
    let output = if app.silent_quit {
        None
    } else {
        Some(app.to_human_output())
    };
    Ok(output)
}

fn run_tui(app: &mut App) -> Result<()> {
    let mut tui = Tui::new()?;
    while !app.should_quit {
        tui.terminal.draw(|frame| app.draw(frame))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    app.handle_key(key);
                }
                Event::Mouse(mouse) if !matches!(mouse.kind, MouseEventKind::Moved) => {
                    app.handle_mouse(mouse);
                }
                _ => {}
            }
        }
    }
    Ok(())
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
