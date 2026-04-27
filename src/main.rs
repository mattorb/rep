use anyhow::{Context, Result, bail};
use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rep::app::App;
use rep::cli::parse_cli_args;
use rep::ui;
use rep::ui::Tui;

const TMUX_FALLBACK_ENV: &str = "REP_TMUX_FALLBACK";
const TERMINAL_WINDOW_FALLBACK_ENV: &str = "REP_TERMINAL_WINDOW_FALLBACK";
const FALLBACK_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

fn main() {
    if let Err(err) = real_main() {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let raw_args: Vec<OsString> = env::args_os().skip(1).collect();
    let cli = parse_cli_args()?;
    if !ui::terminal_available() {
        let used_tmux_fallback = try_tmux_fallback(&raw_args)?;
        let should_try_local_terminal =
            !used_tmux_fallback && !is_ssh_session() && tmux_unavailable();
        if used_tmux_fallback
            || (should_try_local_terminal && try_terminal_window_fallback(&raw_args)?)
        {
            return Ok(());
        }
    }

    let mut app = App::load(cli.source_path)?;
    run_tui(&mut app)?;
    if !app.silent_quit {
        println!("{}", app.to_human_output());
    }
    Ok(())
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
                // Ignore mouse moves; only act on scroll and clicks.
                Event::Mouse(mouse) if !matches!(mouse.kind, MouseEventKind::Moved) => {
                    app.handle_mouse(mouse);
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn try_tmux_fallback(raw_args: &[OsString]) -> Result<bool> {
    if env::var_os(TMUX_FALLBACK_ENV).is_some() || env::var_os("TMUX").is_none() {
        return Ok(false);
    }
    let Some(pane) = env::var_os("TMUX_PANE") else {
        return Ok(false);
    };

    let exe = env::current_exe()?;
    let cwd = env::current_dir()?;
    let bridge = FallbackBridge::create("tmux")?;
    let cmd = build_bridge_command(&exe, raw_args, &bridge, TMUX_FALLBACK_ENV);

    let output = match Command::new("tmux")
        .arg("split-window")
        .arg("-P")
        .arg("-F")
        .arg("#{pane_id}")
        .arg("-v")
        .arg("-p")
        .arg("100")
        .arg("-t")
        .arg(pane)
        .arg("-c")
        .arg(cwd)
        .arg(cmd)
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let _ = fs::remove_dir_all(&bridge.dir);
            return Ok(false);
        }
        Err(err) => {
            let _ = fs::remove_dir_all(&bridge.dir);
            return Err(err).context("failed to run tmux split-window for rep");
        }
    };

    let split_result = if output.status.success() {
        Ok(())
    } else {
        Err(launch_failure(
            output.status.code(),
            &output.stderr,
            "tmux fallback failed while launching rep in a new pane",
        ))
    };
    if split_result.is_err() {
        let _ = fs::remove_dir_all(&bridge.dir);
    }
    split_result?;

    let new_pane = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !new_pane.is_empty() {
        let _ = Command::new("tmux")
            .arg("resize-pane")
            .arg("-Z")
            .arg("-t")
            .arg(&new_pane)
            .status();
        let _ = Command::new("tmux")
            .arg("select-pane")
            .arg("-t")
            .arg(&new_pane)
            .status();
    }

    complete_fallback(bridge, "tmux pane")
}

fn try_terminal_window_fallback(raw_args: &[OsString]) -> Result<bool> {
    if env::var_os(TERMINAL_WINDOW_FALLBACK_ENV).is_some() {
        return Ok(false);
    }

    #[cfg(target_os = "macos")]
    {
        let exe = env::current_exe()?;
        let cwd = env::current_dir()?;
        let bridge = FallbackBridge::create("terminal")?;
        let child_cmd = build_bridge_command(&exe, raw_args, &bridge, TERMINAL_WINDOW_FALLBACK_ENV);
        let launch_cmd = format!(
            "cd {} && {child_cmd}",
            shell_quote(cwd.to_string_lossy().as_ref()),
        );
        let script_cmd = apple_script_quote(&launch_cmd);

        let output = match Command::new("osascript")
            .arg("-e")
            .arg("tell application \"Terminal\"")
            .arg("-e")
            .arg("activate")
            .arg("-e")
            .arg(format!("do script \"{script_cmd}\""))
            .arg("-e")
            .arg("end tell")
            .output()
        {
            Ok(output) => output,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let _ = fs::remove_dir_all(&bridge.dir);
                return Ok(false);
            }
            Err(err) => {
                let _ = fs::remove_dir_all(&bridge.dir);
                return Err(err).context("failed to run osascript for rep terminal fallback");
            }
        };
        if !output.status.success() {
            let _ = fs::remove_dir_all(&bridge.dir);
            return Err(launch_failure(
                output.status.code(),
                &output.stderr,
                "terminal-window fallback failed while launching rep in Terminal.app",
            ));
        }

        complete_fallback(bridge, "terminal window")
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = raw_args;
        Ok(false)
    }
}

fn shell_quote(value: &str) -> String {
    let mut quoted = String::from("'");
    quoted.push_str(&value.replace('\'', "'\"'\"'"));
    quoted.push('\'');
    quoted
}

fn is_ssh_session() -> bool {
    env::var_os("SSH_CONNECTION").is_some()
        || env::var_os("SSH_CLIENT").is_some()
        || env::var_os("SSH_TTY").is_some()
}

fn tmux_unavailable() -> bool {
    if env::var_os("TMUX").is_none() || env::var_os("TMUX_PANE").is_none() {
        return true;
    }
    Command::new("tmux")
        .arg("-V")
        .status()
        .map_or(true, |status| !status.success())
}

#[cfg(target_os = "macos")]
fn apple_script_quote(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

struct FallbackBridge {
    dir: PathBuf,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    status_path: PathBuf,
}

impl FallbackBridge {
    fn create(kind: &str) -> Result<Self> {
        let mut dir = env::temp_dir();
        dir.push(format!(
            "rep-{kind}-bridge-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir(&dir).with_context(|| {
            format!("failed to create rep fallback bridge dir {}", dir.display())
        })?;

        Ok(Self {
            stdout_path: dir.join("stdout.log"),
            stderr_path: dir.join("stderr.log"),
            status_path: dir.join("status.code"),
            dir,
        })
    }
}

fn build_bridge_command(
    exe: &Path,
    raw_args: &[OsString],
    bridge: &FallbackBridge,
    fallback_env: &str,
) -> String {
    let mut child_cmd = shell_quote(exe.to_string_lossy().as_ref());
    for arg in raw_args {
        child_cmd.push(' ');
        child_cmd.push_str(&shell_quote(arg.to_string_lossy().as_ref()));
    }
    let status_file_cmd = format!(
        "printf '%s\\n' \"$rep_exit\" > {}",
        shell_quote(bridge.status_path.to_string_lossy().as_ref())
    );
    let trap_cmd = shell_quote(&status_file_cmd);
    format!(
        "rep_exit=1; \
trap {trap_cmd} EXIT; \
{fallback_env}=1 {child_cmd} > {stdout} 2> {stderr}; \
rep_exit=$?; \
exit \"$rep_exit\"",
        stdout = shell_quote(bridge.stdout_path.to_string_lossy().as_ref()),
        stderr = shell_quote(bridge.stderr_path.to_string_lossy().as_ref()),
    )
}

fn complete_fallback(bridge: FallbackBridge, context: &str) -> Result<bool> {
    if let Err(err) = wait_for_status_file(&bridge.status_path, FALLBACK_TIMEOUT, context) {
        let _ = fs::remove_dir_all(&bridge.dir);
        return Err(err);
    }

    let bridge_result = read_bridge_result(&bridge, context);
    let _ = fs::remove_dir_all(&bridge.dir);
    let bridge_result = bridge_result?;

    if bridge_result.status == 0 {
        if !bridge_result.stdout.is_empty() {
            io::stdout()
                .write_all(&bridge_result.stdout)
                .context("failed writing bridged final output")?;
            io::stdout().flush().ok();
        }
        return Ok(true);
    }

    let detail = String::from_utf8_lossy(&bridge_result.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        bail!("rep exited with status {}", bridge_result.status);
    }
    bail!("rep exited with status {}: {detail}", bridge_result.status);
}

fn read_bridge_result(bridge: &FallbackBridge, context: &str) -> Result<BridgeResult> {
    let status = fs::read_to_string(&bridge.status_path).with_context(|| {
        format!(
            "{context} fallback did not produce status file: {}",
            bridge.status_path.display()
        )
    })?;
    let code = status.trim().parse::<i32>().with_context(|| {
        format!(
            "invalid exit status in {context} bridge file {}: {:?}",
            bridge.status_path.display(),
            status.trim()
        )
    })?;
    let stdout = fs::read(&bridge.stdout_path).unwrap_or_default();
    let stderr = fs::read(&bridge.stderr_path).unwrap_or_default();
    Ok(BridgeResult {
        status: code,
        stdout,
        stderr,
    })
}

struct BridgeResult {
    status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn wait_for_status_file(path: &Path, timeout: Duration, context: &str) -> Result<()> {
    let start = Instant::now();
    loop {
        if path.exists() {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            bail!(
                "timed out waiting for rep to finish in {context} (missing status file: {})",
                path.display()
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn launch_failure(status_code: Option<i32>, stderr: &[u8], prefix: &str) -> anyhow::Error {
    let detail = String::from_utf8_lossy(stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        let status = status_code.map_or_else(|| "unknown".to_string(), |code| code.to_string());
        anyhow::anyhow!("{prefix} (status: {status})")
    } else {
        anyhow::anyhow!("{prefix}: {detail}")
    }
}
