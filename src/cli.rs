use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::{ArgGroup, CommandFactory, Parser, error::ErrorKind};
use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};

use crate::app::App;
use crate::ui::Tui;
use crate::web;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    MarkdownTui,
    HtmlWeb,
}

#[derive(Debug, Clone)]
pub struct CliArgs {
    pub source_path: PathBuf,
    pub debug: bool,
    pub show_keys: bool,
    pub launch_mode: LaunchMode,
    pub no_open: bool,
}

#[derive(Debug, Clone)]
pub enum CliCommand {
    Run(CliArgs),
    Demo { debug: bool, show_keys: bool },
    Help(String),
    Version(String),
}

#[derive(Debug, Parser)]
#[command(
    name = "rep",
    version,
    about = "Collaboratively Tag Text Tool",
    override_usage = "rep [OPTIONS] <plan-file|--demo>",
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
    #[arg(long, conflicts_with_all = ["source_path", "web", "no_open"])]
    demo: bool,

    /// Review an HTML plan in a local browser
    #[arg(long)]
    web: bool,

    /// Print the web review URL without opening a browser
    #[arg(long, requires = "web")]
    no_open: bool,

    /// Path to the Markdown or HTML plan to annotate
    #[arg(value_name = "plan-file")]
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
        Ok(args) if args.demo => Ok(CliCommand::Demo {
            debug: args.debug,
            show_keys: args.show_keys,
        }),
        Ok(args) => {
            let source_path = args
                .source_path
                .expect("required input group guarantees a source path unless --demo was used");
            let launch_mode = validate_launch_mode(&source_path, args.web)?;
            Ok(CliCommand::Run(CliArgs {
                source_path,
                debug: args.debug,
                show_keys: args.show_keys,
                launch_mode,
                no_open: args.no_open,
            }))
        }
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

fn validate_launch_mode(source_path: &std::path::Path, web: bool) -> Result<LaunchMode> {
    let is_html = source_path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
        });
    match (web, is_html) {
        (true, true) => Ok(LaunchMode::HtmlWeb),
        (true, false) => Err(anyhow::anyhow!(
            "--web accepts only .html or .htm plan files"
        )),
        (false, true) => Err(anyhow::anyhow!(
            "HTML plans require --web (for example: rep --web {})",
            source_path.display()
        )),
        (false, false) => Ok(LaunchMode::MarkdownTui),
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

/// Run the local browser frontend for an HTML plan.
pub fn run_web(args: CliArgs) -> Result<Option<String>> {
    web::run(args)
}

pub fn web_debug_diagnostics(path: &std::path::Path, no_open: bool) -> Result<String> {
    web::debug_diagnostics(path, no_open)
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
                assert_eq!(args.launch_mode, LaunchMode::MarkdownTui);
                assert!(!args.no_open);
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
            CliCommand::Demo { debug, show_keys } => {
                assert!(!debug);
                assert!(!show_keys);
            }
            other => panic!("expected demo command, got {other:?}"),
        }
    }

    #[test]
    fn parses_hidden_show_keys_flag_with_demo() {
        let command = parse(&["--show-keys", "--demo"]).unwrap();

        match command {
            CliCommand::Demo { debug, show_keys } => {
                assert!(!debug);
                assert!(show_keys);
            }
            other => panic!("expected demo command, got {other:?}"),
        }
    }

    #[test]
    fn parses_debug_demo_flag() {
        let command = parse(&["--debug", "--demo"]).unwrap();

        match command {
            CliCommand::Demo { debug, show_keys } => {
                assert!(debug);
                assert!(!show_keys);
            }
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
                    assert!(text.contains("Usage: rep [OPTIONS] <plan-file|--demo>"));
                    assert!(text.contains("--debug"));
                    assert!(text.contains("--demo"));
                    assert!(text.contains("--web"));
                    assert!(text.contains("--no-open"));
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
        assert!(err.to_string().contains("<plan-file>") || err.to_string().contains("--demo"));
    }

    #[test]
    fn rejects_multiple_source_paths() {
        let err = parse(&["one.md", "two.md"]).unwrap_err();

        assert!(err.to_string().contains("unexpected argument 'two.md'"));
    }

    #[test]
    fn parses_html_web_flags_before_or_after_the_path_case_insensitively() {
        for args in [
            ["--web", "--no-open", "plan.html"],
            ["plan.HTM", "--web", "--no-open"],
        ] {
            let CliCommand::Run(parsed) = parse(&args).unwrap() else {
                panic!("expected run command");
            };
            assert_eq!(parsed.launch_mode, LaunchMode::HtmlWeb);
            assert!(parsed.no_open);
        }
    }

    #[test]
    fn html_without_web_explains_the_required_mode() {
        for path in ["plan.html", "plan.HTM"] {
            let error = parse(&[path]).unwrap_err();
            assert!(error.to_string().contains("require --web"), "{error}");
        }
    }

    #[test]
    fn web_rejects_markdown_extensionless_and_other_inputs() {
        for path in ["plan.md", "PLAN", "plan.txt"] {
            let error = parse(&["--web", path]).unwrap_err();
            assert!(error.to_string().contains("only .html or .htm"), "{error}");
        }
    }

    #[test]
    fn no_open_requires_web() {
        let error = parse(&["plan.md", "--no-open"]).unwrap_err();
        assert!(error.to_string().contains("required arguments"), "{error}");
        assert!(error.to_string().contains("--web"), "{error}");
    }

    #[test]
    fn web_conflicts_with_demo() {
        let error = parse(&["--demo", "--web"]).unwrap_err();
        assert!(error.to_string().contains("cannot be used with"), "{error}");
    }

    #[test]
    fn debug_web_retains_web_routing_without_starting_it() {
        let CliCommand::Run(args) = parse(&["plan.html", "--debug", "--web", "--no-open"]).unwrap()
        else {
            panic!("expected run command");
        };
        assert!(args.debug);
        assert_eq!(args.launch_mode, LaunchMode::HtmlWeb);
        assert!(args.no_open);
    }
}
