#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
