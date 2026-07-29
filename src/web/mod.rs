use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::cli::CliArgs;
use crate::web::browser::BrowserLauncher;

pub(crate) mod browser;
mod protocol;
mod security;
mod server;
mod session;

const MAX_HTML_BYTES: u64 = 10 * 1024 * 1024;
const DEFAULT_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) fn run(args: CliArgs) -> Result<Option<String>> {
    let source = load_html_source(&args.source_path)?;
    let stop = process_signal_flag()?;
    stop.store(false, Ordering::SeqCst);
    let running = server::RunningServer::start(
        args.source_path,
        source,
        DEFAULT_INACTIVITY_TIMEOUT,
        Arc::clone(&stop),
    )?;

    eprintln!("Rep HTML review: {}", running.source_path().display());
    eprintln!("Review URL: {}", running.url());

    if args.no_open {
        eprintln!("Browser launch disabled by --no-open; open the review URL manually.");
    } else if let Err(error) = browser::SystemBrowserLauncher.launch(running.url()) {
        eprintln!("Could not open a browser automatically: {error:#}");
        eprintln!("Open the review URL manually to continue.");
    }

    match running.wait()? {
        session::ReviewOutcome::Submitted(output) => Ok(Some(output)),
        session::ReviewOutcome::Discarded => Ok(None),
    }
}

pub(crate) fn debug_diagnostics(path: &Path, no_open: bool) -> Result<String> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect HTML file: {}", path.display()))?;
    if !metadata.is_file() {
        bail!("HTML plan is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_HTML_BYTES {
        bail!("HTML plan exceeds the 10 MiB limit: {}", path.display());
    }
    fs::read_to_string(path)
        .with_context(|| format!("HTML plan must be valid UTF-8: {}", path.display()))?;
    let canonical = path
        .canonicalize()
        .with_context(|| format!("failed to canonicalize HTML file: {}", path.display()))?;
    Ok(format!(
        "\
rep debug diagnostics
ui_mode: web
source_format: html
source_path: {}
source_size: {}
browser_launcher_candidate: {}
bind_address: 127.0.0.1 with an OS-assigned port
no_open: {}",
        canonical.display(),
        metadata.len(),
        browser::primary_launcher_candidate(),
        no_open,
    ))
}

fn load_html_source(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect HTML file: {}", path.display()))?;
    if !metadata.is_file() {
        bail!("HTML plan is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_HTML_BYTES {
        bail!("HTML plan exceeds the 10 MiB limit: {}", path.display());
    }
    fs::read_to_string(path)
        .with_context(|| format!("HTML plan must be valid UTF-8: {}", path.display()))
}

fn process_signal_flag() -> Result<Arc<AtomicBool>> {
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    if let Some(flag) = FLAG.get() {
        return Ok(Arc::clone(flag));
    }
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&flag))
        .context("failed to install SIGINT handler")?;
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&flag))
        .context("failed to install SIGTERM handler")?;
    let _ = FLAG.set(Arc::clone(&flag));
    Ok(flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_diagnostics_do_not_include_a_live_token() {
        let path = std::env::temp_dir().join(format!(
            "rep-web-debug-{}-{}.html",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(&path, "<h1>Plan</h1>").unwrap();

        let output = debug_diagnostics(&path, true).unwrap();

        assert!(output.contains("ui_mode: web"));
        assert!(output.contains("source_format: html"));
        assert!(output.contains("source_size: 13"));
        assert!(output.contains("no_open: true"));
        assert!(!output.contains("Review URL:"));
    }
}
