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
    if io::stdout().is_terminal() {
        return true;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .is_ok()
}

impl Tui {
    pub fn new() -> Result<Self> {
        let tty_help = "rep requires an interactive terminal (TTY). \
Run it in a real terminal session, or launch it from your agent with PTY/interactive mode enabled.";

        // If stdout is being captured by a parent process, write the UI directly to /dev/tty.
        // This allows the TUI to take over the user's terminal while preserving stdout capture.
        let mut output: Box<dyn io::Write> = if io::stdout().is_terminal() {
            Box::new(io::stdout())
        } else {
            let tty = OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/tty")
                .context(tty_help)?;
            Box::new(tty)
        };

        enable_raw_mode().context(tty_help)?;
        execute!(output, EnterAlternateScreen, EnableMouseCapture).context(tty_help)?;
        let backend = CrosstermBackend::new(output);
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
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
