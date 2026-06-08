use std::process::Command;

fn rep_bin() -> Command {
    let mut bin = std::env::current_exe().unwrap();
    // Walk up from deps/ to the build root, then find the rep binary.
    bin.pop(); // remove test binary name
    if bin.ends_with("deps") {
        bin.pop();
    }
    bin.push("rep");
    Command::new(bin)
}

#[test]
fn version_flag_prints_version() {
    let out = rep_bin()
        .arg("--version")
        .output()
        .expect("failed to run rep");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("rep "), "got: {stdout}");
    // Version should match what Cargo embeds.
    let version = env!("CARGO_PKG_VERSION");
    assert!(stdout.contains(version), "got: {stdout}");
}

#[test]
fn short_version_flag() {
    let out = rep_bin().arg("-V").output().expect("failed to run rep");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("rep "), "got: {stdout}");
}

#[test]
fn help_flag_exits_zero() {
    let out = rep_bin().arg("--help").output().expect("failed to run rep");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Usage:"), "got: {stdout}");
    assert!(!stdout.contains("Navigation:"), "got: {stdout}");
    assert!(!stdout.contains("Annotations:"), "got: {stdout}");
}

#[test]
fn short_help_flag() {
    let out = rep_bin().arg("-h").output().expect("failed to run rep");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Usage:"), "got: {stdout}");
    assert!(!stdout.contains("Navigation:"), "got: {stdout}");
    assert!(!stdout.contains("Annotations:"), "got: {stdout}");
}

#[test]
fn debug_flag_prints_diagnostics_without_opening_tui() {
    let out = rep_bin()
        .args(["--debug", "plan.md"])
        .output()
        .expect("failed to run rep");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rep debug diagnostics"), "got: {stdout}");
    assert!(stdout.contains("source_path: plan.md"), "got: {stdout}");
    assert!(stdout.contains("terminal_available:"), "got: {stdout}");
    assert!(stdout.contains("would_try_tmux_fallback:"), "got: {stdout}");
}

#[test]
fn unknown_flag_exits_nonzero() {
    let out = rep_bin()
        .arg("--bogus-flag")
        .output()
        .expect("failed to run rep");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown option") || stderr.contains("bogus"),
        "got: {stderr}"
    );
}

#[test]
fn missing_file_argument_exits_nonzero() {
    let out = rep_bin().output().expect("failed to run rep");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("usage") || stderr.contains("markdown"),
        "got: {stderr}"
    );
}

#[test]
fn nonexistent_file_exits_nonzero() {
    let out = rep_bin()
        .arg("/tmp/rep-test-nonexistent-file-abc123.md")
        .output()
        .expect("failed to run rep");
    assert!(!out.status.success());
}

#[test]
fn json_flag_exits_nonzero() {
    let out = rep_bin().arg("--json").output().expect("failed to run rep");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unexpected argument '--json'"),
        "got: {stderr}"
    );
}

#[test]
fn multiple_file_arguments_exits_nonzero() {
    let out = rep_bin()
        .args(["file1.md", "file2.md"])
        .output()
        .expect("failed to run rep");
    assert!(!out.status.success());
}
