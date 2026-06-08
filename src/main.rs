use anyhow::Result;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use rep::cli::{CliCommand, parse_cli_args_from, run_interactive};
use rep::ui;

mod terminal_fallback;

const DEMO_MARKDOWN: &str = include_str!("../examples/demo-plan.md");

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
    let (source_path, debug) = match parse_cli_args_from(raw_args.iter().cloned())? {
        CliCommand::Help(text) | CliCommand::Version(text) => {
            return Ok(Some(text));
        }
        CliCommand::Run(cli) => (cli.source_path, cli.debug),
        CliCommand::Demo { debug } => (write_demo_source()?, debug),
    };
    if debug {
        let terminal_available = terminal_available();
        return Ok(Some(render_debug_diagnostics(
            &source_path,
            &terminal_fallback::diagnostics(terminal_available),
        )));
    }
    if !terminal_available() && try_fallback(&raw_args)? {
        return Ok(None);
    }

    run_interactive(source_path)
}

fn write_demo_source() -> Result<PathBuf> {
    let mut path = env::temp_dir();
    path.push(format!("rep-demo-{}.md", std::process::id()));
    fs::write(&path, DEMO_MARKDOWN)?;
    Ok(path)
}

fn render_debug_diagnostics(
    source_path: &Path,
    diagnostics: &terminal_fallback::FallbackDiagnostics,
) -> String {
    format!(
        "\
rep debug diagnostics
source_path: {}
terminal_available: {}
tmux_env_present: {}
tmux_pane_env_present: {}
rep_tmux_fallback_env_present: {}
rep_terminal_window_fallback_env_present: {}
ssh_session: {}
tmux_unavailable: {}
would_try_tmux_fallback: {}
would_try_terminal_window_fallback: {}",
        source_path.display(),
        diagnostics.terminal_available,
        diagnostics.tmux_env_present,
        diagnostics.tmux_pane_env_present,
        diagnostics.tmux_fallback_env_present,
        diagnostics.terminal_window_fallback_env_present,
        diagnostics.ssh_session,
        diagnostics.tmux_unavailable,
        diagnostics.would_try_tmux_fallback,
        diagnostics.would_try_terminal_window_fallback,
    )
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
                .contains("Usage: rep [OPTIONS] <markdown-file|--demo>")
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
    fn debug_returns_diagnostics_without_touching_tui_or_fallback() {
        let output = real_main_with(
            vec![OsString::from("--debug"), OsString::from("plan.md")],
            || true,
            |_| panic!("fallback should not run for debug"),
            |_| panic!("interactive TUI should not run for debug"),
        )
        .unwrap()
        .unwrap();

        assert!(output.contains("rep debug diagnostics"));
        assert!(output.contains("source_path: plan.md"));
        assert!(output.contains("terminal_available: true"));
    }

    #[test]
    fn demo_runs_interactive_with_generated_sample_file() {
        let output = real_main_with(
            vec![OsString::from("--demo")],
            || true,
            |_| panic!("fallback should not run when terminal is available"),
            |source_path| {
                let contents = std::fs::read_to_string(&source_path).unwrap();
                assert!(contents.contains("Checkout Cleanup Plan"));
                assert!(
                    source_path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .starts_with("rep-demo-")
                );
                Ok(Some("demo output".to_string()))
            },
        )
        .unwrap();

        assert_eq!(output.as_deref(), Some("demo output"));
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
