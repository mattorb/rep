use std::fs::OpenOptions;
use std::io;
use std::io::IsTerminal;

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;

pub struct Tui {
    pub terminal: Terminal<CrosstermBackend<Box<dyn io::Write>>>,
}

pub fn terminal_available() -> bool {
    terminal_available_from(io::stdout().is_terminal(), || {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .is_ok()
    })
}

fn terminal_available_from(stdout_is_terminal: bool, tty_available: impl FnOnce() -> bool) -> bool {
    stdout_is_terminal || tty_available()
}

impl Tui {
    pub fn new() -> Result<Self> {
        let tty_help = "rep requires an interactive terminal (TTY). \
Run it in a real terminal session, or launch it from your agent with PTY/interactive mode enabled.";

        // If stdout is being captured by a parent process, write the UI directly to /dev/tty.
        // This allows the TUI to take over the user's terminal while preserving stdout capture.
        let mut output = open_tui_output(
            io::stdout().is_terminal(),
            || Box::new(io::stdout()),
            || {
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open("/dev/tty")
                    .map(|tty| Box::new(tty) as Box<dyn io::Write>)
            },
            tty_help,
        )?;

        enable_raw_mode().context(tty_help)?;
        execute!(output, EnterAlternateScreen, EnableMouseCapture).context(tty_help)?;
        let backend = CrosstermBackend::new(output);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }
}

fn open_tui_output<OpenStdout, OpenTty>(
    stdout_is_terminal: bool,
    open_stdout: OpenStdout,
    open_tty: OpenTty,
    tty_help: &str,
) -> Result<Box<dyn io::Write>>
where
    OpenStdout: FnOnce() -> Box<dyn io::Write>,
    OpenTty: FnOnce() -> io::Result<Box<dyn io::Write>>,
{
    if stdout_is_terminal {
        Ok(open_stdout())
    } else {
        open_tty().with_context(|| tty_help.to_string())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        );
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct SharedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedWriter {
        fn contents(&self) -> Vec<u8> {
            self.bytes.lock().unwrap().clone()
        }
    }

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn terminal_available_when_stdout_is_terminal() {
        assert!(terminal_available_from(true, || {
            panic!("tty should not be probed when stdout is a terminal")
        }));
    }

    #[test]
    fn terminal_available_when_tty_can_be_opened() {
        assert!(terminal_available_from(false, || true));
    }

    #[test]
    fn terminal_unavailable_when_stdout_and_tty_are_unavailable() {
        assert!(!terminal_available_from(false, || false));
    }

    #[test]
    fn tui_output_prefers_stdout_when_it_is_terminal() {
        let tty_opened = AtomicBool::new(false);
        let stdout = SharedWriter::default();
        let stdout_check = stdout.clone();

        let mut output = open_tui_output(
            true,
            || Box::new(stdout),
            || {
                tty_opened.store(true, Ordering::SeqCst);
                Ok(Box::new(Vec::<u8>::new()))
            },
            "tty help",
        )
        .unwrap();

        output.write_all(b"stdout").unwrap();
        assert!(!tty_opened.load(Ordering::SeqCst));
        assert_eq!(stdout_check.contents(), b"stdout");
        drop(output);
    }

    #[test]
    fn tui_output_uses_tty_when_stdout_is_captured() {
        let stdout_opened = AtomicBool::new(false);
        let tty = SharedWriter::default();
        let tty_check = tty.clone();

        let mut output = open_tui_output(
            false,
            || {
                stdout_opened.store(true, Ordering::SeqCst);
                Box::new(Vec::<u8>::new())
            },
            || Ok(Box::new(tty)),
            "tty help",
        )
        .unwrap();

        output.write_all(b"tty").unwrap();
        assert!(!stdout_opened.load(Ordering::SeqCst));
        assert_eq!(tty_check.contents(), b"tty");
        drop(output);
    }

    #[test]
    fn tui_output_adds_tty_help_to_open_errors() {
        let err = match open_tui_output(
            false,
            || Box::new(Vec::<u8>::new()),
            || {
                Err(io::Error::new(
                    ErrorKind::NotFound,
                    "no controlling terminal",
                ))
            },
            "tty help",
        ) {
            Ok(_) => panic!("expected tty open error"),
            Err(err) => err,
        };

        assert_eq!(err.to_string(), "tty help");
    }

    #[test]
    fn tui_output_error_retains_tty_open_failure_as_source() {
        let err = match open_tui_output(
            false,
            || Box::new(Vec::<u8>::new()),
            || {
                Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "permission denied",
                ))
            },
            "tty help",
        ) {
            Ok(_) => panic!("expected tty open error"),
            Err(err) => err,
        };

        let source = err
            .source()
            .expect("tty open context should retain source error");
        assert_eq!(source.to_string(), "permission denied");
    }
}
