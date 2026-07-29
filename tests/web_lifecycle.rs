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
    assert!(shell.contains("Rep HTML review"), "{shell}");

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
