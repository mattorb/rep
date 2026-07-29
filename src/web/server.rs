use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::web::protocol::{Request, Response, read_request, write_response};
use crate::web::security;
use crate::web::session::{ReviewOutcome, generate_token};

pub(crate) struct RunningServer {
    source_path: PathBuf,
    url: String,
    join: JoinHandle<Result<ReviewOutcome>>,
}

impl RunningServer {
    pub(crate) fn start(
        source_path: PathBuf,
        source: String,
        inactivity_timeout: Duration,
        stop: Arc<AtomicBool>,
    ) -> Result<Self> {
        let canonical_source = source_path.canonicalize().with_context(|| {
            format!(
                "failed to canonicalize HTML file: {}",
                source_path.display()
            )
        })?;
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).context("failed to bind loopback web server")?;
        listener
            .set_nonblocking(true)
            .context("failed to configure loopback listener")?;
        let address = listener
            .local_addr()
            .context("failed to read bind address")?;
        let token = generate_token()?;
        let url = format!("http://{address}/session/{token}/");
        let thread_url = url.clone();
        let join = thread::Builder::new()
            .name("rep-web-server".to_string())
            .spawn(move || {
                serve(
                    listener,
                    address,
                    &token,
                    &source,
                    inactivity_timeout,
                    &stop,
                )
            })
            .context("failed to start web server thread")?;
        debug_assert!(thread_url.starts_with("http://127.0.0.1:"));
        Ok(Self {
            source_path: canonical_source,
            url,
            join,
        })
    }

    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn wait(self) -> Result<ReviewOutcome> {
        self.join
            .join()
            .map_err(|_| anyhow!("web server thread panicked"))?
    }
}

fn serve(
    listener: TcpListener,
    address: SocketAddr,
    token: &str,
    source: &str,
    inactivity_timeout: Duration,
    stop: &AtomicBool,
) -> Result<ReviewOutcome> {
    let mut last_activity = Instant::now();
    loop {
        if stop.load(Ordering::SeqCst) {
            bail!("web review interrupted");
        }
        if last_activity.elapsed() >= inactivity_timeout {
            bail!("web review timed out after inactivity");
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                last_activity = Instant::now();
                if let Some(outcome) = handle_connection(&mut stream, address, token, source)? {
                    return Ok(outcome);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) => return Err(error).context("loopback listener failed"),
        }
    }
}

fn handle_connection(
    stream: &mut TcpStream,
    address: SocketAddr,
    token: &str,
    source: &str,
) -> Result<Option<ReviewOutcome>> {
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(error) => {
            let response = Response::text(400, &format!("bad request: {error}"));
            write_response(stream, &response, false)?;
            return Ok(None);
        }
    };
    let head_only = request.method == "HEAD";
    let (mut response, outcome) = route(&request, address, token, source);
    if request.path.ends_with("/api/health")
        || request.path.ends_with("/api/state")
        || request.path.contains("/api/")
    {
        security::add_api_headers(&mut response);
    } else {
        security::add_parent_headers(&mut response);
    }
    write_response(stream, &response, head_only)?;
    Ok(outcome)
}

fn route(
    request: &Request,
    address: SocketAddr,
    token: &str,
    source: &str,
) -> (Response, Option<ReviewOutcome>) {
    if let Err(response) = security::validate_request(request, address, token) {
        return (response, None);
    }
    if request.method == "POST"
        && serde_json::from_slice::<serde_json::Value>(&request.body).is_err()
    {
        return (Response::text(400, "request body must be valid JSON"), None);
    }
    let root = format!("/session/{token}/");
    let health = format!("{root}api/health");
    let state = format!("{root}api/state");
    let finish = format!("{root}api/finish");
    let discard = format!("{root}api/discard");

    match (request.method.as_str(), request.path.as_str()) {
        ("GET" | "HEAD", path) if path == root => (
            Response::new(
                200,
                "text/html; charset=utf-8",
                placeholder_shell(token, source.len()).into_bytes(),
            ),
            None,
        ),
        ("GET" | "HEAD", path) if path == health || path == state => (
            Response::json(
                200,
                &format!(
                    "{{\"status\":\"waiting\",\"sourceBytes\":{}}}",
                    source.len()
                ),
            ),
            None,
        ),
        ("POST", path) if path == finish => (
            Response::json(200, "{\"status\":\"finished\"}"),
            Some(ReviewOutcome::Submitted("No actions.".to_string())),
        ),
        ("POST", path) if path == discard => (
            Response::json(200, "{\"status\":\"discarded\"}"),
            Some(ReviewOutcome::Discarded),
        ),
        ("GET" | "HEAD", _) => (Response::text(404, "not found"), None),
        _ => (Response::text(405, "method not allowed"), None),
    }
}

fn placeholder_shell(token: &str, source_bytes: usize) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>Rep HTML review</title></head>\
         <body><main><h1>Rep HTML review</h1><p>Preparing {source_bytes} bytes of HTML.</p>\
         <p>Session {token}</p></main></body></html>"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn request(method: &str, path: &str, address: SocketAddr) -> Request {
        let mut headers = BTreeMap::from([("host".to_string(), address.to_string())]);
        if method == "POST" {
            headers.insert("origin".to_string(), format!("http://{address}"));
            headers.insert("content-type".to_string(), "application/json".to_string());
        }
        Request {
            method: method.to_string(),
            path: path.to_string(),
            headers,
            body: b"{}".to_vec(),
        }
    }

    #[test]
    fn token_method_host_origin_and_content_type_are_required() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "a".repeat(64);
        let root = format!("/session/{token}/");
        let (response, _) = route(&request("GET", &root, address), address, &token, "");
        assert_eq!(response.status, 200);

        let (response, _) = route(
            &request("GET", "/session/wrong/", address),
            address,
            &token,
            "",
        );
        assert_eq!(response.status, 404);

        let mut wrong_host = request("GET", &root, address);
        wrong_host
            .headers
            .insert("host".to_string(), "localhost".to_string());
        let (response, _) = route(&wrong_host, address, &token, "");
        assert_eq!(response.status, 400);

        let finish = format!("{root}api/finish");
        let mut missing_origin = request("POST", &finish, address);
        missing_origin.headers.remove("origin");
        let (response, outcome) = route(&missing_origin, address, &token, "");
        assert_eq!(response.status, 403);
        assert!(outcome.is_none());
    }

    #[test]
    fn finish_and_discard_produce_terminal_outcomes() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "b".repeat(64);
        let finish = format!("/session/{token}/api/finish");
        let (_, outcome) = route(&request("POST", &finish, address), address, &token, "");
        assert_eq!(
            outcome,
            Some(ReviewOutcome::Submitted("No actions.".to_string()))
        );

        let discard = format!("/session/{token}/api/discard");
        let (_, outcome) = route(&request("POST", &discard, address), address, &token, "");
        assert_eq!(outcome, Some(ReviewOutcome::Discarded));
    }

    #[test]
    fn zero_timeout_exits_without_waiting_for_wall_clock_time() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = AtomicBool::new(false);

        let error = serve(
            listener,
            address,
            &"c".repeat(64),
            "",
            Duration::ZERO,
            &stop,
        )
        .unwrap_err();

        assert!(error.to_string().contains("timed out"));
    }
}
