use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{ArgGroup, CommandFactory, Parser, error::ErrorKind};
use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};

use crate::app::App;
use crate::ui::Tui;

#[derive(Debug, Clone)]
pub struct CliArgs {
    pub source_path: PathBuf,
    pub debug: bool,
    pub show_keys: bool,
}

#[derive(Debug, Clone)]
pub enum CliCommand {
    Run(CliArgs),
    Demo { debug: bool },
    Help(String),
    Version(String),
}

#[derive(Debug, Parser)]
#[command(
    name = "rep",
    version,
    about = "Collaboratively Tag Text Tool",
    override_usage = "rep [OPTIONS] <markdown-file|--demo>",
    group(
        ArgGroup::new("input")
            .required(true)
            .args(["source_path", "demo"])
    )
)]
struct RawCliArgs {
    /// Print launch diagnostics and exit without opening the TUI
    #[arg(long)]
    debug: bool,

    /// Show a transient keypress HUD in the TUI.
    #[arg(long, hide = true)]
    show_keys: bool,

    /// Open a built-in sample Markdown file
    #[arg(long, conflicts_with = "source_path")]
    demo: bool,

    /// Path to the Markdown file to annotate
    #[arg(value_name = "markdown-file")]
    source_path: Option<PathBuf>,
}

pub fn parse_cli_args() -> Result<CliCommand> {
    parse_cli_args_from(env::args_os().skip(1))
}

pub fn parse_cli_args_from<I, S>(args: I) -> Result<CliCommand>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let raw_args = std::iter::once(OsString::from("rep")).chain(args.into_iter().map(Into::into));
    match RawCliArgs::try_parse_from(raw_args) {
        Ok(args) if args.demo => Ok(CliCommand::Demo { debug: args.debug }),
        Ok(args) => Ok(CliCommand::Run(CliArgs {
            source_path: args
                .source_path
                .expect("required input group guarantees a source path unless --demo was used"),
            debug: args.debug,
            show_keys: args.show_keys,
        })),
        Err(err) if err.kind() == ErrorKind::DisplayHelp => {
            let mut command = RawCliArgs::command();
            Ok(CliCommand::Help(command.render_help().to_string()))
        }
        Err(err) if err.kind() == ErrorKind::DisplayVersion => Ok(CliCommand::Version(
            RawCliArgs::command()
                .render_version()
                .to_string()
                .trim_end()
                .to_string(),
        )),
        Err(err) => Err(anyhow::anyhow!(strip_error_prefix(err.to_string()))),
    }
}

fn strip_error_prefix(message: String) -> String {
    message
        .strip_prefix("error: ")
        .unwrap_or(&message)
        .to_string()
}

/// Run the interactive TUI for a parsed CLI source path.
///
/// Returns `Some(output)` when the session should print the human-readable
/// action block after exit, or `None` for silent quit.
pub fn run_interactive(args: CliArgs) -> Result<Option<String>> {
    let mut app = App::load(args.source_path)?;
    if args.show_keys {
        app.enable_key_cues();
    }
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
            CliCommand::Run(args) => {
                assert_eq!(args.source_path, PathBuf::from("notes.md"));
                assert!(!args.debug);
            }
            other => panic!("expected run command, got {other:?}"),
        }
    }

    #[test]
    fn parses_debug_flag_with_source_path() {
        let command = parse(&["--debug", "notes.md"]).unwrap();

        match command {
            CliCommand::Run(args) => {
                assert_eq!(args.source_path, PathBuf::from("notes.md"));
                assert!(args.debug);
                assert!(!args.show_keys);
            }
            other => panic!("expected run command, got {other:?}"),
        }
    }

    #[test]
    fn parses_hidden_show_keys_flag() {
        let command = parse(&["--show-keys", "notes.md"]).unwrap();

        match command {
            CliCommand::Run(args) => {
                assert_eq!(args.source_path, PathBuf::from("notes.md"));
                assert!(!args.debug);
                assert!(args.show_keys);
            }
            other => panic!("expected run command, got {other:?}"),
        }
    }

    #[test]
    fn parses_demo_flag_without_source_path() {
        let command = parse(&["--demo"]).unwrap();

        match command {
            CliCommand::Demo { debug } => assert!(!debug),
            other => panic!("expected demo command, got {other:?}"),
        }
    }

    #[test]
    fn parses_debug_demo_flag() {
        let command = parse(&["--debug", "--demo"]).unwrap();

        match command {
            CliCommand::Demo { debug } => assert!(debug),
            other => panic!("expected demo command, got {other:?}"),
        }
    }

    #[test]
    fn rejects_demo_with_source_path() {
        let err = parse(&["--demo", "notes.md"]).unwrap_err();

        assert!(err.to_string().contains("cannot be used with"));
    }

    #[test]
    fn returns_help_text_for_short_and_long_help_flags() {
        for flag in ["-h", "--help"] {
            let command = parse(&[flag]).unwrap();

            match command {
                CliCommand::Help(text) => {
                    assert!(text.contains("Usage: rep [OPTIONS] <markdown-file|--demo>"));
                    assert!(text.contains("--debug"));
                    assert!(text.contains("--demo"));
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
                    assert_eq!(text.trim(), format!("rep {}", env!("CARGO_PKG_VERSION")));
                }
                other => panic!("expected version command for {flag}, got {other:?}"),
            }
        }
    }

    #[test]
    fn rejects_unknown_options() {
        let err = parse(&["--bogus"]).unwrap_err();

        assert!(err.to_string().contains("unexpected argument '--bogus'"));
        assert!(!err.to_string().starts_with("error: "));
    }

    #[test]
    fn rejects_missing_source_path() {
        let err = parse(&[]).unwrap_err();

        assert!(
            err.to_string()
                .contains("required arguments were not provided")
        );
        assert!(err.to_string().contains("<markdown-file>") || err.to_string().contains("--demo"));
    }

    #[test]
    fn rejects_multiple_source_paths() {
        let err = parse(&["one.md", "two.md"]).unwrap_err();

        assert!(err.to_string().contains("unexpected argument 'two.md'"));
    }
}
