use anyhow::{Context, Result, bail};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TMUX_FALLBACK_ENV: &str = "REP_TMUX_FALLBACK";
const TERMINAL_WINDOW_FALLBACK_ENV: &str = "REP_TERMINAL_WINDOW_FALLBACK";
const FALLBACK_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FallbackDiagnostics {
    pub terminal_available: bool,
    pub tmux_env_present: bool,
    pub tmux_pane_env_present: bool,
    pub tmux_fallback_env_present: bool,
    pub terminal_window_fallback_env_present: bool,
    pub ssh_session: bool,
    pub tmux_unavailable: bool,
    pub would_try_tmux_fallback: bool,
    pub would_try_terminal_window_fallback: bool,
}

pub(crate) fn diagnostics(terminal_available: bool) -> FallbackDiagnostics {
    let tmux_env_present = env::var_os("TMUX").is_some();
    let tmux_pane_env_present = env::var_os("TMUX_PANE").is_some();
    let tmux_fallback_env_present = env::var_os(TMUX_FALLBACK_ENV).is_some();
    let terminal_window_fallback_env_present = env::var_os(TERMINAL_WINDOW_FALLBACK_ENV).is_some();
    let ssh_session = is_ssh_session();
    let tmux_unavailable = tmux_unavailable();
    let would_try_tmux_fallback = !terminal_available
        && !tmux_fallback_env_present
        && tmux_env_present
        && tmux_pane_env_present;
    let would_try_terminal_window_fallback = !terminal_available
        && !terminal_window_fallback_env_present
        && should_try_terminal_window(would_try_tmux_fallback, ssh_session, tmux_unavailable);

    FallbackDiagnostics {
        terminal_available,
        tmux_env_present,
        tmux_pane_env_present,
        tmux_fallback_env_present,
        terminal_window_fallback_env_present,
        ssh_session,
        tmux_unavailable,
        would_try_tmux_fallback,
        would_try_terminal_window_fallback,
    }
}

pub(crate) fn try_launch(raw_args: &[OsString]) -> Result<bool> {
    let used_tmux_fallback = try_tmux_fallback(raw_args)?;
    if used_tmux_fallback {
        return Ok(true);
    }

    if should_try_terminal_window(false, is_ssh_session(), tmux_unavailable()) {
        return try_terminal_window_fallback(raw_args);
    }

    Ok(false)
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

fn should_try_terminal_window(
    used_tmux_fallback: bool,
    is_ssh_session: bool,
    tmux_unavailable: bool,
) -> bool {
    !used_tmux_fallback && !is_ssh_session && tmux_unavailable
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
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_bridge(name: &str) -> FallbackBridge {
        let dir = env::temp_dir().join(format!("rep-{name}-test-{}", unique_suffix()));
        fs::create_dir(&dir).expect("test precondition: create bridge dir");
        FallbackBridge {
            stdout_path: dir.join("stdout.log"),
            stderr_path: dir.join("stderr.log"),
            status_path: dir.join("status.code"),
            dir,
        }
    }

    #[test]
    fn terminal_window_fallback_only_when_tmux_cannot_handle_session() {
        assert!(should_try_terminal_window(false, false, true));
        assert!(!should_try_terminal_window(true, false, true));
        assert!(!should_try_terminal_window(false, true, true));
        assert!(!should_try_terminal_window(false, false, false));
    }

    #[test]
    fn shell_quote_handles_spaces_and_single_quotes() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("two words"), "'two words'");
        assert_eq!(shell_quote("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn bridge_command_quotes_exe_args_and_bridge_paths() {
        let bridge = FallbackBridge {
            dir: PathBuf::from("/tmp/rep bridge"),
            stdout_path: PathBuf::from("/tmp/out log"),
            stderr_path: PathBuf::from("/tmp/err log"),
            status_path: PathBuf::from("/tmp/status code"),
        };
        let args = [
            OsString::from("plain.md"),
            OsString::from("two words.md"),
            OsString::from("it's.md"),
        ];

        let cmd = build_bridge_command(Path::new("/bin/rep"), &args, &bridge, "REP_TEST_FALLBACK");

        assert!(cmd.contains("REP_TEST_FALLBACK=1 '/bin/rep'"));
        assert!(cmd.contains("'plain.md'"));
        assert!(cmd.contains("'two words.md'"));
        assert!(cmd.contains("'it'\"'\"'s.md'"));
        assert!(cmd.contains("> '/tmp/out log'"));
        assert!(cmd.contains("2> '/tmp/err log'"));
        assert!(cmd.contains("trap "));
        assert!(cmd.contains("/tmp/status code"));
    }

    #[test]
    fn launch_failure_includes_status_when_stderr_is_empty() {
        let err = launch_failure(Some(17), b"", "tmux fallback failed");
        assert_eq!(err.to_string(), "tmux fallback failed (status: 17)");
    }

    #[test]
    fn launch_failure_prefers_stderr_detail() {
        let err = launch_failure(Some(17), b"bad pane\n", "tmux fallback failed");
        assert_eq!(err.to_string(), "tmux fallback failed: bad pane");
    }

    #[test]
    fn launch_failure_reports_unknown_status_without_code() {
        let err = launch_failure(None, b"", "terminal fallback failed");
        assert_eq!(
            err.to_string(),
            "terminal fallback failed (status: unknown)"
        );
    }

    #[test]
    fn wait_for_status_file_returns_when_file_exists() {
        let bridge = test_bridge("wait-ready");
        fs::write(&bridge.status_path, "0\n").expect("test precondition: write status");

        wait_for_status_file(&bridge.status_path, Duration::from_millis(0), "test bridge").unwrap();

        fs::remove_dir_all(&bridge.dir).ok();
    }

    #[test]
    fn wait_for_status_file_times_out_with_context() {
        let bridge = test_bridge("wait-timeout");

        let err =
            wait_for_status_file(&bridge.status_path, Duration::from_millis(0), "test bridge")
                .unwrap_err();

        let message = err.to_string();
        assert!(message.contains("timed out waiting for rep to finish in test bridge"));
        assert!(message.contains("status.code"));
        fs::remove_dir_all(&bridge.dir).ok();
    }

    #[test]
    fn read_bridge_result_reads_status_stdout_and_stderr() {
        let bridge = test_bridge("read-result");
        fs::write(&bridge.status_path, "7\n").expect("test precondition: write status");
        fs::write(&bridge.stdout_path, "final output").expect("test precondition: write stdout");
        fs::write(&bridge.stderr_path, "details").expect("test precondition: write stderr");

        let result = read_bridge_result(&bridge, "test bridge").unwrap();

        assert_eq!(result.status, 7);
        assert_eq!(result.stdout, b"final output");
        assert_eq!(result.stderr, b"details");
        fs::remove_dir_all(&bridge.dir).ok();
    }

    #[test]
    fn read_bridge_result_rejects_invalid_status() {
        let bridge = test_bridge("bad-status");
        fs::write(&bridge.status_path, "not a code\n")
            .expect("test precondition: write bad status");

        let err = match read_bridge_result(&bridge, "test bridge") {
            Ok(_) => panic!("expected invalid status error"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("invalid exit status"));
        fs::remove_dir_all(&bridge.dir).ok();
    }

    #[test]
    fn complete_fallback_removes_bridge_dir_after_child_failure() {
        let bridge = test_bridge("complete-failure");
        let dir = bridge.dir.clone();
        fs::write(&bridge.status_path, "2\n").expect("test precondition: write status");
        fs::write(&bridge.stderr_path, "child failed\n").expect("test precondition: write stderr");

        let err = complete_fallback(bridge, "test bridge").unwrap_err();

        assert_eq!(err.to_string(), "rep exited with status 2: child failed");
        assert!(!dir.exists());
    }

    #[test]
    fn complete_fallback_removes_bridge_dir_after_success() {
        let bridge = test_bridge("complete-success");
        let dir = bridge.dir.clone();
        fs::write(&bridge.status_path, "0\n").expect("test precondition: write status");

        assert!(complete_fallback(bridge, "test bridge").unwrap());
        assert!(!dir.exists());
    }
}
