use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::Duration;

fn rep_bin() -> Command {
    let mut binary = std::env::current_exe().unwrap();
    binary.pop();
    if binary.ends_with("deps") {
        binary.pop();
    }
    binary.push("rep");
    Command::new(binary)
}

#[test]
fn no_open_server_finishes_cleanly_and_closes_its_listener() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/web/semantic.html");
    let mut child = rep_bin()
        .args(["--web", "--no-open"])
        .arg(fixture)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start rep web process");

    let stderr = child.stderr.take().unwrap();
    let mut stderr = BufReader::new(stderr);
    let mut url = None;
    for _ in 0..10 {
        let mut line = String::new();
        let count = stderr
            .read_line(&mut line)
            .expect("read startup diagnostics");
        assert_ne!(count, 0, "rep exited before printing the review URL");
        if let Some(value) = line.strip_prefix("Review URL: ") {
            url = Some(value.trim().to_string());
            break;
        }
    }
    let url = url.expect("review URL in stderr");
    let (address, path) = split_local_url(&url);

    let shell = request(
        address,
        &format!("GET {path} HTTP/1.1\r\nHost: {address}\r\n\r\n"),
    );
    assert!(shell.starts_with("HTTP/1.1 200 OK"), "{shell}");
    assert!(shell.contains("<title>Rep HTML Review</title>"), "{shell}");

    let document = request(
        address,
        &format!("GET {path}assets/__rep_document__.html HTTP/1.1\r\nHost: {address}\r\n\r\n"),
    );
    assert!(document.starts_with("HTTP/1.1 200 OK"), "{document}");
    assert!(document.contains("data-rep-source-line="), "{document}");
    assert!(document.contains("sandbox allow-same-origin"), "{document}");

    let finish_path = format!("{path}api/finish");
    let finish = request(
        address,
        &format!(
            "POST {finish_path} HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
        ),
    );
    assert!(finish.starts_with("HTTP/1.1 200 OK"), "{finish}");

    let status = child.wait().expect("wait for rep web process");
    assert!(status.success(), "{status}");
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert_eq!(stdout, "No actions.\n");

    let address = address.parse().unwrap();
    assert!(
        TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err(),
        "listener remained reachable after process exit"
    );
}

#[test]
#[cfg(unix)]
fn sigterm_stops_the_server_nonzero_without_partial_output() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/web/semantic.html");
    let mut child = rep_bin()
        .args(["--web", "--no-open"])
        .arg(fixture)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start rep web process");

    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut url = None;
    for _ in 0..10 {
        let mut line = String::new();
        let count = stderr
            .read_line(&mut line)
            .expect("read startup diagnostics");
        assert_ne!(count, 0, "rep exited before printing the review URL");
        if let Some(value) = line.strip_prefix("Review URL: ") {
            url = Some(value.trim().to_string());
            break;
        }
    }
    let url = url.expect("review URL in stderr");
    let (address, _) = split_local_url(&url);
    let socket: std::net::SocketAddr = address.parse().unwrap();

    let signal = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(signal.success(), "{signal}");

    let status = child.wait().expect("wait for interrupted web process");
    assert!(!status.success(), "{status}");
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert_eq!(stdout, "", "interruption emitted a partial action list");
    let mut remaining_stderr = String::new();
    stderr.read_to_string(&mut remaining_stderr).unwrap();
    assert!(
        remaining_stderr.contains("web review interrupted"),
        "{remaining_stderr}"
    );
    assert!(
        TcpStream::connect_timeout(&socket, Duration::from_millis(100)).is_err(),
        "listener remained reachable after interruption"
    );
}

#[test]
#[cfg(unix)]
fn missing_browser_opener_keeps_manual_review_url_usable() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/web/semantic.html");
    let empty_path = std::env::temp_dir().join(format!("rep-empty-path-{}", std::process::id()));
    std::fs::create_dir_all(&empty_path).unwrap();
    let mut child = rep_bin()
        .arg("--web")
        .arg(fixture)
        .env("PATH", &empty_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start rep web process");

    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut url = None;
    for _ in 0..10 {
        let mut line = String::new();
        let count = stderr.read_line(&mut line).expect("read startup output");
        assert_ne!(count, 0, "rep exited before printing the review URL");
        if let Some(value) = line.strip_prefix("Review URL: ") {
            url = Some(value.trim().to_string());
            break;
        }
    }
    let url = url.expect("review URL in stderr");
    let (address, path) = split_local_url(&url);
    let finish = request(
        address,
        &format!(
            "POST {path}api/finish HTTP/1.1\r\nHost: {address}\r\nOrigin: http://{address}\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{{}}"
        ),
    );
    assert!(finish.starts_with("HTTP/1.1 200 OK"), "{finish}");

    let status = child.wait().expect("wait for manual-open web process");
    assert!(status.success(), "{status}");
    let mut remaining_stderr = String::new();
    stderr.read_to_string(&mut remaining_stderr).unwrap();
    assert!(
        remaining_stderr.contains("Could not open a browser automatically"),
        "{remaining_stderr}"
    );
    assert!(
        remaining_stderr.contains("Open the review URL manually to continue"),
        "{remaining_stderr}"
    );
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert_eq!(stdout, "No actions.\n");
    std::fs::remove_dir(empty_path).unwrap();
}

fn split_local_url(url: &str) -> (&str, &str) {
    let without_scheme = url.strip_prefix("http://").expect("http loopback URL");
    let slash = without_scheme.find('/').expect("URL path");
    (&without_scheme[..slash], &without_scheme[slash..])
}

fn request(address: &str, request: &str) -> String {
    let mut stream = TcpStream::connect(address).expect("connect to rep web server");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}
