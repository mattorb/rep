use std::env;
use std::ffi::OsString;
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
    parse_cli_args_from(env::args_os().skip(1))
}

pub fn parse_cli_args_from<I, S>(args: I) -> Result<CliCommand>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut source_path: Option<PathBuf> = None;

    for arg in args {
        let arg = arg.into();
        let arg_text = arg.to_string_lossy();
        match arg_text.as_ref() {
            "-h" | "--help" => {
                return Ok(CliCommand::Help(help_text()));
            }
            "--version" | "-V" => {
                return Ok(CliCommand::Version(format!(
                    "rep {}",
                    env!("CARGO_PKG_VERSION")
                )));
            }
            _ if arg_text.starts_with('-') => {
                anyhow::bail!("unknown option: {arg_text}\nusage: rep <markdown-file-path>");
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<CliCommand> {
        parse_cli_args_from(args.iter().copied())
    }

    #[test]
    fn parses_single_source_path() {
        let command = parse(&["notes.md"]).unwrap();

        match command {
            CliCommand::Run(args) => assert_eq!(args.source_path, PathBuf::from("notes.md")),
            other => panic!("expected run command, got {other:?}"),
        }
    }

    #[test]
    fn returns_help_text_for_short_and_long_help_flags() {
        for flag in ["-h", "--help"] {
            let command = parse(&[flag]).unwrap();

            match command {
                CliCommand::Help(text) => {
                    assert!(text.contains("Usage: rep [OPTIONS] <markdown-file>"));
                    assert!(text.contains("-V, --version"));
                }
                other => panic!("expected help command for {flag}, got {other:?}"),
            }
        }
    }

    #[test]
    fn returns_version_text_for_short_and_long_version_flags() {
        for flag in ["-V", "--version"] {
            let command = parse(&[flag]).unwrap();

            match command {
                CliCommand::Version(text) => {
                    assert_eq!(text, format!("rep {}", env!("CARGO_PKG_VERSION")));
                }
                other => panic!("expected version command for {flag}, got {other:?}"),
            }
        }
    }

    #[test]
    fn rejects_unknown_options() {
        let err = parse(&["--bogus"]).unwrap_err();

        assert_eq!(
            err.to_string(),
            "unknown option: --bogus\nusage: rep <markdown-file-path>"
        );
    }

    #[test]
    fn rejects_missing_source_path() {
        let err = parse(&[]).unwrap_err();

        assert_eq!(err.to_string(), "usage: rep <markdown-file-path>");
    }

    #[test]
    fn rejects_multiple_source_paths() {
        let err = parse(&["one.md", "two.md"]).unwrap_err();

        assert_eq!(
            err.to_string(),
            "expected a single markdown file path\nusage: rep <markdown-file-path>"
        );
    }
}
