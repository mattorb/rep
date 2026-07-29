use std::io;
use std::process::Command;

use anyhow::{Result, bail};

pub(crate) trait BrowserLauncher {
    fn launch(&self, url: &str) -> Result<()>;
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SystemBrowserLauncher;

impl BrowserLauncher for SystemBrowserLauncher {
    fn launch(&self, url: &str) -> Result<()> {
        launch_for_platform(url, std::env::consts::OS, |program, args| {
            Command::new(program).args(args).status()
        })
    }
}

pub(crate) const fn primary_launcher_candidate() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open (fallback: gio open)"
    } else {
        "unsupported"
    }
}

fn launch_for_platform<F>(url: &str, platform: &str, mut run: F) -> Result<()>
where
    F: FnMut(&str, &[&str]) -> io::Result<std::process::ExitStatus>,
{
    let candidates: &[(&str, &[&str])] = match platform {
        "macos" => &[("open", &[url])],
        "linux" => &[("xdg-open", &[url]), ("gio", &["open", url])],
        other => bail!("automatic browser launch is unsupported on {other}"),
    };

    let mut failures = Vec::new();
    for (program, args) in candidates {
        match run(program, args) {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => failures.push(format!("{program} exited with {status}")),
            Err(error) => failures.push(format!("{program}: {error}")),
        }
    }
    bail!("all browser launchers failed: {}", failures.join("; "))
}

#[cfg(test)]
mod tests {
    use std::process::ExitStatus;

    use super::*;

    #[cfg(unix)]
    fn status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    #[test]
    #[cfg(unix)]
    fn macos_uses_open_with_the_url() {
        let mut calls = Vec::new();
        launch_for_platform("http://127.0.0.1/", "macos", |program, args| {
            calls.push((program.to_string(), args.join(" ")));
            Ok(status(0))
        })
        .unwrap();
        assert_eq!(
            calls,
            [("open".to_string(), "http://127.0.0.1/".to_string())]
        );
    }

    #[test]
    #[cfg(unix)]
    fn linux_falls_back_to_gio() {
        let mut calls = Vec::new();
        launch_for_platform("http://127.0.0.1/", "linux", |program, args| {
            calls.push((program.to_string(), args.join(" ")));
            Ok(status(i32::from(program == "xdg-open")))
        })
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].0, "gio");
        assert_eq!(calls[1].1, "open http://127.0.0.1/");
    }

    #[test]
    fn unsupported_platform_fails_without_running_a_command() {
        let error = launch_for_platform("http://127.0.0.1/", "windows", |_, _| {
            panic!("launcher must not run")
        })
        .unwrap_err();
        assert!(error.to_string().contains("unsupported"));
    }
}
