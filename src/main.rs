use anyhow::Result;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

use rep::cli::{CliCommand, parse_cli_args_from, run_interactive};
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
    if let Some(output) = real_main_with(
        raw_args,
        ui::terminal_available,
        terminal_fallback::try_launch,
        run_interactive,
    )? {
        println!("{output}");
    }
    Ok(())
}

fn real_main_with<TerminalAvailable, TryFallback, RunInteractive>(
    raw_args: Vec<OsString>,
    terminal_available: TerminalAvailable,
    mut try_fallback: TryFallback,
    mut run_interactive: RunInteractive,
) -> Result<Option<String>>
where
    TerminalAvailable: FnOnce() -> bool,
    TryFallback: FnMut(&[OsString]) -> Result<bool>,
    RunInteractive: FnMut(PathBuf) -> Result<Option<String>>,
{
    let cli = match parse_cli_args_from(raw_args.iter().cloned())? {
        CliCommand::Help(text) | CliCommand::Version(text) => {
            return Ok(Some(text));
        }
        CliCommand::Run(cli) => cli,
    };
    if !terminal_available() && try_fallback(&raw_args)? {
        return Ok(None);
    }

    run_interactive(cli.source_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;

    #[test]
    fn help_returns_text_without_touching_terminal_or_tui() {
        let output = real_main_with(
            vec![OsString::from("--help")],
            || panic!("terminal should not be queried for help"),
            |_| panic!("fallback should not run for help"),
            |_| panic!("interactive TUI should not run for help"),
        )
        .unwrap();

        assert!(
            output
                .unwrap()
                .contains("Usage: rep [OPTIONS] <markdown-file>")
        );
    }

    #[test]
    fn version_returns_text_without_touching_terminal_or_tui() {
        let output = real_main_with(
            vec![OsString::from("--version")],
            || panic!("terminal should not be queried for version"),
            |_| panic!("fallback should not run for version"),
            |_| panic!("interactive TUI should not run for version"),
        )
        .unwrap();

        assert_eq!(
            output.unwrap(),
            format!("rep {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn available_terminal_runs_interactive_session() {
        let output = real_main_with(
            vec![OsString::from("plan.md")],
            || true,
            |_| panic!("fallback should not run when terminal is available"),
            |source_path| {
                assert_eq!(source_path, PathBuf::from("plan.md"));
                Ok(Some("final output".to_string()))
            },
        )
        .unwrap();

        assert_eq!(output.as_deref(), Some("final output"));
    }

    #[test]
    fn unavailable_terminal_returns_when_fallback_launches() {
        let output = real_main_with(
            vec![OsString::from("plan.md")],
            || false,
            |raw_args| {
                assert_eq!(raw_args, [OsString::from("plan.md")]);
                Ok(true)
            },
            |_| panic!("interactive TUI should not run after fallback launches"),
        )
        .unwrap();

        assert_eq!(output, None);
    }

    #[test]
    fn unavailable_terminal_runs_interactive_when_fallback_declines() {
        let output = real_main_with(
            vec![OsString::from("plan.md")],
            || false,
            |_| Ok(false),
            |source_path| {
                assert_eq!(source_path, PathBuf::from("plan.md"));
                Ok(None)
            },
        )
        .unwrap();

        assert_eq!(output, None);
    }

    #[test]
    fn propagates_fallback_errors() {
        let err = real_main_with(
            vec![OsString::from("plan.md")],
            || false,
            |_| bail!("fallback exploded"),
            |_| panic!("interactive TUI should not run after fallback error"),
        )
        .unwrap_err();

        assert_eq!(err.to_string(), "fallback exploded");
    }
}
