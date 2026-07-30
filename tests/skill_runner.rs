#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn temp_dir(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("rep-skill-{name}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}

fn make_fake_rep(root: &Path) -> PathBuf {
    let path = root.join("fake rep");
    fs::write(
        &path,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$REP_ARGS_FILE\"\nprintf '%s' \"${REP_FAKE_OUTPUT:-}\"\nexit \"${REP_FAKE_RC:-0}\"\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn make_fake_review_rep(root: &Path) -> PathBuf {
    let path = root.join("fake review rep");
    fs::write(
        &path,
        r#"#!/bin/sh
printf '%s\n' "$@" >"$REP_ARGS_FILE"
printf '%s\n' 'Review URL: http://127.0.0.1:43117/session/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/' >&2
: >"$REP_FAKE_REP_READY"
while [ ! -e "$REP_FAKE_FINISH" ]; do sleep 0.05; done
printf '%s' "$REP_FAKE_OUTPUT"
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn make_fake_browser(root: &Path) -> PathBuf {
    let path = root.join("fake browser");
    fs::write(
        &path,
        r#"#!/bin/sh
printf '%s\n' "$@" >"$REP_FAKE_BROWSER_ARGS"
: >"$REP_FAKE_BROWSER_READY"
trap ': >"$REP_FAKE_BROWSER_STOPPED"; exit 0' TERM INT
while :; do sleep 0.05; done
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn wait_for_file(path: &Path) {
    for _ in 0..200 {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for {}", path.display());
}

fn script(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".agents/skills/rep/scripts")
        .join(relative)
}

#[test]
fn plan_mode_routes_html_markdown_and_rejects_ambiguous_extensions() {
    for (name, expected) in [
        ("PLAN.HTML", "html\n"),
        ("plan.htm", "html\n"),
        ("plan.md", "markdown\n"),
        ("roadmap", "markdown\n"),
    ] {
        let output = Command::new(script("plan_mode.sh"))
            .arg(name)
            .output()
            .unwrap();
        assert!(output.status.success(), "{name}");
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
    let output = Command::new(script("plan_mode.sh"))
        .arg("plan.txt")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn runner_forwards_post_path_web_flag_and_captures_fresh_output() {
    let root = temp_dir("forward");
    let plan = root.join("plan with spaces.html");
    fs::write(&plan, "<h1>Plan</h1>").unwrap();
    let args_file = root.join("args.txt");
    let fake = make_fake_rep(&root);
    let output = Command::new(script("run_rep_and_capture.sh"))
        .arg(&plan)
        .arg("--web")
        .env("REP_BIN", fake)
        .env("REP_ARGS_FILE", &args_file)
        .env("REP_CAPTURE_DIR", &root)
        .env("REP_FAKE_OUTPUT", "FORMAT: html\n\nNo actions.\n")
        .env("REP_BROWSER_MANAGED_EXTERNALLY", "1")
        .output()
        .unwrap();

    assert!(output.status.success());
    let forwarded = fs::read_to_string(args_file).unwrap();
    assert!(
        forwarded
            .lines()
            .next()
            .unwrap()
            .ends_with("plan with spaces.html")
    );
    assert_eq!(forwarded.lines().nth(1), Some("--web"));
    assert_eq!(forwarded.lines().nth(2), Some("--no-open"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    let capture = stderr
        .lines()
        .find_map(|line| line.strip_prefix("REP_CAPTURE_FILE="))
        .unwrap();
    assert_eq!(
        fs::read_to_string(capture).unwrap(),
        "FORMAT: html\n\nNo actions.\n"
    );
}

#[test]
fn html_runner_fails_before_rep_when_it_cannot_own_a_browser() {
    let root = temp_dir("missing-browser");
    let plan = root.join("plan.html");
    fs::write(&plan, "<h1>Plan</h1>").unwrap();
    let args_file = root.join("args.txt");
    let fake_rep = make_fake_rep(&root);
    let missing_browser = root.join("missing-browser");

    let output = Command::new(script("run_rep_and_capture.sh"))
        .arg(&plan)
        .arg("--web")
        .env("REP_BIN", fake_rep)
        .env("REP_ARGS_FILE", &args_file)
        .env("REP_CAPTURE_DIR", &root)
        .env("REP_BROWSER_BIN", missing_browser)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        !args_file.exists(),
        "Rep must not start after failed preflight"
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("REP_BROWSER_BIN is not executable")
    );
}

#[test]
fn html_runner_rejects_invalid_agent_window_bounds_before_rep() {
    let root = temp_dir("invalid-browser-bounds");
    let plan = root.join("plan.html");
    fs::write(&plan, "<h1>Plan</h1>").unwrap();
    let args_file = root.join("args.txt");
    let fake_rep = make_fake_rep(&root);
    let fake_browser = make_fake_browser(&root);

    let output = Command::new(script("run_rep_and_capture.sh"))
        .arg(&plan)
        .arg("--web")
        .env("REP_BIN", fake_rep)
        .env("REP_ARGS_FILE", &args_file)
        .env("REP_CAPTURE_DIR", &root)
        .env("REP_BROWSER_BIN", fake_browser)
        .env("REP_AGENT_WINDOW_BOUNDS", "not-bounds")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        !args_file.exists(),
        "Rep must not start with an invalid browser geometry override"
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("invalid REP_AGENT_WINDOW_BOUNDS")
    );
}

#[test]
fn html_runner_owns_and_stops_its_temporary_browser() {
    let root = temp_dir("browser-lifecycle");
    let plan = root.join("plan.html");
    fs::write(&plan, "<h1>Plan</h1>").unwrap();
    let args_file = root.join("args.txt");
    let browser_args = root.join("browser-args.txt");
    let browser_ready = root.join("browser-ready");
    let browser_stopped = root.join("browser-stopped");
    let rep_ready = root.join("rep-ready");
    let rep_finish = root.join("rep-finish");
    let fake_rep = make_fake_review_rep(&root);
    let fake_browser = make_fake_browser(&root);

    let child = Command::new(script("run_rep_and_capture.sh"))
        .arg(&plan)
        .arg("--web")
        .env("REP_BIN", fake_rep)
        .env("REP_ARGS_FILE", &args_file)
        .env("REP_CAPTURE_DIR", &root)
        .env("REP_FAKE_OUTPUT", "FORMAT: html\n\nNo actions.\n")
        .env("REP_FAKE_REP_READY", &rep_ready)
        .env("REP_FAKE_FINISH", &rep_finish)
        .env("REP_BROWSER_BIN", fake_browser)
        .env("REP_FAKE_BROWSER_ARGS", &browser_args)
        .env("REP_FAKE_BROWSER_READY", &browser_ready)
        .env("REP_FAKE_BROWSER_STOPPED", &browser_stopped)
        .env("REP_AGENT_WINDOW_BOUNDS", "41,53,1200,800")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    wait_for_file(&rep_ready);
    wait_for_file(&browser_ready);
    wait_for_file(&browser_args);
    fs::write(&rep_finish, "").unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert!(browser_stopped.exists());
    let launched = fs::read_to_string(browser_args).unwrap();
    assert!(launched.contains("--user-data-dir="));
    assert!(launched.contains("--new-window"));
    assert!(launched.contains("--window-position=41,53"));
    assert!(launched.contains("--window-size=1200,800"));
    assert!(launched.contains("http://127.0.0.1:43117/session/"));
    let forwarded = fs::read_to_string(args_file).unwrap();
    assert_eq!(forwarded.lines().nth(2), Some("--no-open"));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "FORMAT: html\n\nNo actions.\n"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("REP_CAPTURE_FILE="));
}

#[test]
fn runner_preserves_failure_status_marker_and_empty_silent_discard() {
    let root = temp_dir("status");
    let plan = root.join("plan.md");
    fs::write(&plan, "# Plan").unwrap();
    let args_file = root.join("args.txt");
    let fake = make_fake_rep(&root);

    let failed = Command::new(script("run_rep_and_capture.sh"))
        .arg(&plan)
        .env("REP_BIN", &fake)
        .env("REP_ARGS_FILE", &args_file)
        .env("REP_CAPTURE_DIR", &root)
        .env("REP_FAKE_RC", "7")
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(7));
    assert!(
        String::from_utf8(failed.stderr)
            .unwrap()
            .contains("REP_CAPTURE_FILE=")
    );

    let discarded = Command::new(script("run_rep_and_capture.sh"))
        .arg(&plan)
        .env("REP_BIN", fake)
        .env("REP_ARGS_FILE", args_file)
        .env("REP_CAPTURE_DIR", &root)
        .output()
        .unwrap();
    assert!(discarded.status.success());
    let marker = String::from_utf8(discarded.stderr)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("REP_CAPTURE_FILE=").map(str::to_string))
        .unwrap();
    assert_eq!(fs::metadata(marker).unwrap().len(), 0);
}
