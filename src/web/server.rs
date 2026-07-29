use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::review::command::ReviewCommand;
use crate::review::document::ReviewDocument;
use crate::review::session::ReviewSession;
use crate::selection::model::{SelectionAnchor, SelectionUnit};
use crate::web::document::{HtmlReviewDocument, SelectionSlice, parse_manifest};
use crate::web::protocol::{Request, Response, read_request, write_response};
use crate::web::security;
use crate::web::session::{ReviewOutcome, generate_token};
use crate::web::{assets, html_source};

const APP_HTML: &str = include_str!("app.html");
const APP_CSS: &str = include_str!("app.css");
const APP_JS: &str = include_str!("app.js");
const DOCUMENT_JS: &str = include_str!("document.js");

struct WebContent {
    source_path: PathBuf,
    document: String,
    blocked_resources: usize,
    plan_root: PathBuf,
}

struct WebSession {
    document: Option<HtmlReviewDocument>,
    review: Option<ReviewSession>,
    revision: u64,
}

impl WebSession {
    const fn new() -> Self {
        Self {
            document: None,
            review: None,
            revision: 0,
        }
    }
}

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
        let plan_root = canonical_source
            .parent()
            .context("HTML source path has no parent directory")?
            .canonicalize()
            .context("failed to canonicalize HTML plan directory")?;
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).context("failed to bind loopback web server")?;
        listener
            .set_nonblocking(true)
            .context("failed to configure loopback listener")?;
        let address = listener
            .local_addr()
            .context("failed to read bind address")?;
        let token = generate_token()?;
        let transformed = html_source::transform(&source, &token)?;
        let content = WebContent {
            source_path: canonical_source.clone(),
            document: transformed.source,
            blocked_resources: transformed.blocked_resources,
            plan_root,
        };
        let url = format!("http://{address}/session/{token}/");
        let thread_url = url.clone();
        let join = thread::Builder::new()
            .name("rep-web-server".to_string())
            .spawn(move || {
                serve(
                    listener,
                    address,
                    &token,
                    &content,
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
    content: &WebContent,
    inactivity_timeout: Duration,
    stop: &AtomicBool,
) -> Result<ReviewOutcome> {
    let mut last_activity = Instant::now();
    let mut session = WebSession::new();
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
                if let Some(outcome) =
                    handle_connection(&mut stream, address, token, content, &mut session)?
                {
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
    content: &WebContent,
    session: &mut WebSession,
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
    let (mut response, outcome) = route(&request, address, token, content, session);
    if request.path.contains("/api/") {
        security::add_api_headers(&mut response);
    } else if request.path.ends_with("/assets/__rep_document__.html") {
        security::add_document_headers(&mut response);
    } else if request.path.contains("/assets/") {
        security::add_asset_headers(&mut response);
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
    content: &WebContent,
    session: &mut WebSession,
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
    let manifest = format!("{root}api/manifest");
    let command = format!("{root}api/command");
    let finish = format!("{root}api/finish");
    let discard = format!("{root}api/discard");
    let document = format!("{root}assets/__rep_document__.html");
    let app_css = format!("{root}app.css");
    let app_js = format!("{root}app.js");
    let document_js = format!("{root}document.js");
    let asset_prefix = format!("{root}assets/");

    match (request.method.as_str(), request.path.as_str()) {
        ("GET" | "HEAD", path) if path == root => (
            Response::new(
                200,
                "text/html; charset=utf-8",
                APP_HTML.as_bytes().to_vec(),
            ),
            None,
        ),
        ("GET" | "HEAD", path) if path == app_css => (
            Response::new(200, "text/css; charset=utf-8", APP_CSS.as_bytes().to_vec()),
            None,
        ),
        ("GET" | "HEAD", path) if path == app_js => (
            Response::new(
                200,
                "text/javascript; charset=utf-8",
                APP_JS.as_bytes().to_vec(),
            ),
            None,
        ),
        ("GET" | "HEAD", path) if path == document_js => (
            Response::new(
                200,
                "text/javascript; charset=utf-8",
                DOCUMENT_JS.as_bytes().to_vec(),
            ),
            None,
        ),
        ("GET" | "HEAD", path) if path == document => (
            Response::new(
                200,
                "text/html; charset=utf-8",
                content.document.as_bytes().to_vec(),
            ),
            None,
        ),
        ("GET" | "HEAD", path) if path.starts_with(&asset_prefix) => {
            let encoded_path = &path[asset_prefix.len()..];
            match assets::load(&content.plan_root, encoded_path) {
                Ok(asset) => (Response::new(200, asset.content_type, asset.bytes), None),
                Err(_) => (Response::text(404, "asset not found"), None),
            }
        }
        ("GET" | "HEAD", path) if path == health => {
            (Response::json(200, "{\"status\":\"ok\"}"), None)
        }
        ("GET" | "HEAD", path) if path == state => (state_response(content, session), None),
        ("POST", path) if path == manifest => {
            let incoming = match parse_manifest(&request.body) {
                Ok(manifest) => manifest,
                Err(error) => return (Response::text(400, &error.to_string()), None),
            };
            if let Some(document) = &session.document {
                if document.manifest() != &incoming {
                    return (
                        Response::text(409, "a different document manifest is already active"),
                        None,
                    );
                }
                return (state_response(content, session), None);
            }
            let document =
                match HtmlReviewDocument::from_manifest(content.source_path.clone(), incoming) {
                    Ok(document) => document,
                    Err(error) => return (Response::text(400, &error.to_string()), None),
                };
            let review = ReviewSession::new(document.initial_anchor());
            session.document = Some(document);
            session.review = Some(review);
            session.revision = 1;
            (state_response(content, session), None)
        }
        ("POST", path) if path == command => {
            let command: BrowserCommand = match serde_json::from_slice(&request.body) {
                Ok(command) => command,
                Err(error) => {
                    return (
                        Response::text(400, &format!("invalid command: {error}")),
                        None,
                    );
                }
            };
            let Some(document) = session.document.as_ref() else {
                return (
                    Response::text(409, "document manifest has not been initialized"),
                    None,
                );
            };
            let Some(review) = session.review.as_mut() else {
                return (Response::text(500, "review state is unavailable"), None);
            };
            if command.revision() != session.revision {
                return (
                    Response::text(409, "command revision does not match server state"),
                    None,
                );
            }
            review.clear_navigation_feedback();
            let status = match command {
                BrowserCommand::Move {
                    forward,
                    revision: _,
                } => {
                    review
                        .apply(document, ReviewCommand::MoveActiveUnit { forward })
                        .status
                }
                BrowserCommand::MoveNode { delta, revision: _ } => {
                    review
                        .apply(document, ReviewCommand::MoveNode { delta })
                        .status
                }
                BrowserCommand::Cycle {
                    forward,
                    revision: _,
                } => {
                    review
                        .apply(document, ReviewCommand::CycleUnit { forward })
                        .status
                }
                BrowserCommand::Adjust { finer, revision: _ } => {
                    review
                        .apply(document, ReviewCommand::AdjustUnit { finer })
                        .status
                }
                BrowserCommand::Select {
                    node,
                    unit,
                    scalar,
                    revision: _,
                } => {
                    let Some(unit) = parse_unit(&unit) else {
                        return (Response::text(400, "invalid selection unit"), None);
                    };
                    let Some(anchor) = document.anchor_at_scalar(node, unit, scalar) else {
                        return (
                            Response::text(400, "selection target is out of bounds"),
                            None,
                        );
                    };
                    review.set_anchor(document, anchor);
                    None
                }
            };
            session.revision += 1;
            (
                state_response_with_status(
                    content,
                    session,
                    status.or_else(|| {
                        session
                            .review
                            .as_ref()
                            .and_then(|review| review.nav_feedback.clone())
                    }),
                ),
                None,
            )
        }
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
enum BrowserCommand {
    Move {
        revision: u64,
        forward: bool,
    },
    MoveNode {
        revision: u64,
        delta: isize,
    },
    Cycle {
        revision: u64,
        forward: bool,
    },
    Adjust {
        revision: u64,
        finer: bool,
    },
    Select {
        revision: u64,
        node: usize,
        unit: String,
        scalar: usize,
    },
}

impl BrowserCommand {
    const fn revision(&self) -> u64 {
        match self {
            Self::Move { revision, .. }
            | Self::MoveNode { revision, .. }
            | Self::Cycle { revision, .. }
            | Self::Adjust { revision, .. }
            | Self::Select { revision, .. } => *revision,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StateSnapshot<'a> {
    status: &'static str,
    blocked_resources: usize,
    revision: u64,
    node_count: usize,
    mode: Option<&'static str>,
    anchor: Option<AnchorSnapshot>,
    selection: Vec<SelectionSlice>,
    message: Option<String>,
    outline: Vec<OutlineSnapshot<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnchorSnapshot {
    node: usize,
    unit: &'static str,
    unit_index: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OutlineSnapshot<'a> {
    node: usize,
    level: u8,
    text: &'a str,
}

fn state_response(content: &WebContent, session: &WebSession) -> Response {
    state_response_with_status(content, session, None)
}

fn state_response_with_status(
    content: &WebContent,
    session: &WebSession,
    message: Option<String>,
) -> Response {
    let (status, node_count, mode, anchor, selection, outline) =
        match (&session.document, &session.review) {
            (Some(document), Some(review)) if document.node_count() == 0 => {
                ("empty", 0, None, None, Vec::new(), Vec::new())
            }
            (Some(document), Some(review)) => {
                let anchor = review.anchor();
                let outline_rows = document.node_outline();
                let outline = outline_rows
                    .iter()
                    .map(|row| OutlineSnapshot {
                        node: row.node_idx,
                        level: row.level,
                        text: &row.text,
                    })
                    .collect::<Vec<_>>();
                // Serialize while the owned outline rows are still alive.
                let snapshot = StateSnapshot {
                    status: "ready",
                    blocked_resources: content.blocked_resources,
                    revision: session.revision,
                    node_count: document.node_count(),
                    mode: Some(review.mode_indicator()),
                    anchor: Some(anchor_snapshot(anchor)),
                    selection: document.selection_slices(anchor),
                    message,
                    outline,
                };
                return json_response(&snapshot);
            }
            _ => ("waiting", 0, None, None, Vec::new(), Vec::new()),
        };
    json_response(&StateSnapshot {
        status,
        blocked_resources: content.blocked_resources,
        revision: session.revision,
        node_count,
        mode,
        anchor,
        selection,
        message,
        outline,
    })
}

fn anchor_snapshot(anchor: SelectionAnchor) -> AnchorSnapshot {
    AnchorSnapshot {
        node: anchor.node_idx,
        unit: anchor.unit.mode_str(),
        unit_index: anchor.unit_idx,
    }
}

fn json_response(value: &impl Serialize) -> Response {
    match serde_json::to_string(value) {
        Ok(body) => Response::json(200, &body),
        Err(error) => Response::text(500, &format!("failed to encode state: {error}")),
    }
}

fn parse_unit(unit: &str) -> Option<SelectionUnit> {
    match unit {
        "section" => Some(SelectionUnit::Section),
        "paragraph" => Some(SelectionUnit::Paragraph),
        "line" => Some(SelectionUnit::Line),
        "sentence" => Some(SelectionUnit::Sentence),
        "word" => Some(SelectionUnit::Word),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::web::document::{HtmlManifest, HtmlManifestNode, ScalarRange};

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

    fn content() -> WebContent {
        WebContent {
            source_path: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/web/semantic.html")
                .canonicalize()
                .unwrap(),
            document: "<h1>Plan</h1>".to_string(),
            blocked_resources: 2,
            plan_root: Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/web")
                .canonicalize()
                .unwrap(),
        }
    }

    fn route_once(
        request: &Request,
        address: SocketAddr,
        token: &str,
        content: &WebContent,
    ) -> (Response, Option<ReviewOutcome>) {
        route(request, address, token, content, &mut WebSession::new())
    }

    fn manifest(nodes: &[&str]) -> HtmlManifest {
        HtmlManifest {
            version: 1,
            nodes: nodes
                .iter()
                .enumerate()
                .map(|(index, text)| HtmlManifestNode {
                    source_id: index as u64,
                    source_line: index + 1,
                    tag: if index == 0 { "h1" } else { "p" }.to_string(),
                    text: (*text).to_string(),
                    logical_lines: vec![ScalarRange {
                        start: 0,
                        end: text.chars().count(),
                    }],
                    selector: format!("body > :nth-child({})", index + 1),
                    text_fragment: None,
                    heading_level: (index == 0).then_some(1),
                    list_id: None,
                    top_level_ordered_list_item: false,
                    links: Vec::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn token_method_host_origin_and_content_type_are_required() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "a".repeat(64);
        let root = format!("/session/{token}/");
        let content = content();
        let (response, _) = route_once(&request("GET", &root, address), address, &token, &content);
        assert_eq!(response.status, 200);

        let (response, _) = route_once(
            &request("GET", "/session/wrong/", address),
            address,
            &token,
            &content,
        );
        assert_eq!(response.status, 404);

        let mut wrong_host = request("GET", &root, address);
        wrong_host
            .headers
            .insert("host".to_string(), "localhost".to_string());
        let (response, _) = route_once(&wrong_host, address, &token, &content);
        assert_eq!(response.status, 400);

        let finish = format!("{root}api/finish");
        let mut missing_origin = request("POST", &finish, address);
        missing_origin.headers.remove("origin");
        let (response, outcome) = route_once(&missing_origin, address, &token, &content);
        assert_eq!(response.status, 403);
        assert!(outcome.is_none());
    }

    #[test]
    fn embedded_shell_document_and_assets_are_routed() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "d".repeat(64);
        let content = content();
        let root = format!("/session/{token}/");

        for (path, expected_type) in [
            (root.clone(), "text/html; charset=utf-8"),
            (format!("{root}app.css"), "text/css; charset=utf-8"),
            (format!("{root}app.js"), "text/javascript; charset=utf-8"),
            (
                format!("{root}assets/__rep_document__.html"),
                "text/html; charset=utf-8",
            ),
            (
                format!("{root}assets/assets/plan.css"),
                "text/css; charset=utf-8",
            ),
        ] {
            let (response, _) =
                route_once(&request("GET", &path, address), address, &token, &content);
            assert_eq!(response.status, 200, "{path}");
            assert_eq!(response.headers[0].1, expected_type, "{path}");
        }
    }

    #[test]
    fn finish_and_discard_produce_terminal_outcomes() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "b".repeat(64);
        let content = content();
        let finish = format!("/session/{token}/api/finish");
        let (_, outcome) = route_once(
            &request("POST", &finish, address),
            address,
            &token,
            &content,
        );
        assert_eq!(
            outcome,
            Some(ReviewOutcome::Submitted("No actions.".to_string()))
        );

        let discard = format!("/session/{token}/api/discard");
        let (_, outcome) = route_once(
            &request("POST", &discard, address),
            address,
            &token,
            &content,
        );
        assert_eq!(outcome, Some(ReviewOutcome::Discarded));
    }

    #[test]
    fn manifest_is_idempotent_and_commands_require_current_revision() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "e".repeat(64);
        let content = content();
        let root = format!("/session/{token}/");
        let mut session = WebSession::new();
        let mut initialize = request("POST", &format!("{root}api/manifest"), address);
        initialize.body = serde_json::to_vec(&manifest(&["Plan", "First.", "Second."])).unwrap();

        let (response, _) = route(&initialize, address, &token, &content, &mut session);
        assert_eq!(response.status, 200);
        assert_eq!(session.revision, 1);
        assert_eq!(session.review.as_ref().unwrap().anchor().node_idx, 0);

        let (response, _) = route(&initialize, address, &token, &content, &mut session);
        assert_eq!(response.status, 200);
        assert_eq!(
            session.revision, 1,
            "reload must not reset or advance state"
        );

        let mut move_request = request("POST", &format!("{root}api/command"), address);
        move_request.body = br#"{"type":"move","revision":1,"forward":true}"#.to_vec();
        let (response, _) = route(&move_request, address, &token, &content, &mut session);
        assert_eq!(response.status, 200);
        assert_eq!(session.revision, 2);
        assert_eq!(session.review.as_ref().unwrap().anchor().node_idx, 1);

        let (response, _) = route(&move_request, address, &token, &content, &mut session);
        assert_eq!(response.status, 409);
        assert_eq!(session.revision, 2);

        initialize.body = serde_json::to_vec(&manifest(&["Different"])).unwrap();
        let (response, _) = route(&initialize, address, &token, &content, &mut session);
        assert_eq!(response.status, 409);
    }

    #[test]
    fn malformed_manifest_and_selection_commands_are_rejected() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "f".repeat(64);
        let content = content();
        let root = format!("/session/{token}/");
        let mut session = WebSession::new();
        let mut command = request("POST", &format!("{root}api/command"), address);
        command.body = br#"{"type":"move","revision":0,"forward":true}"#.to_vec();
        let (response, _) = route(&command, address, &token, &content, &mut session);
        assert_eq!(response.status, 409);

        let mut initialize = request("POST", &format!("{root}api/manifest"), address);
        initialize.body = br#"{"version":1,"nodes":[{"sourceId":0}]}"#.to_vec();
        let (response, _) = route(&initialize, address, &token, &content, &mut session);
        assert_eq!(response.status, 400);

        initialize.body = serde_json::to_vec(&manifest(&["Plan"])).unwrap();
        let (response, _) = route(&initialize, address, &token, &content, &mut session);
        assert_eq!(response.status, 200);
        command.body =
            br#"{"type":"select","revision":1,"node":99,"unit":"word","scalar":0}"#.to_vec();
        let (response, _) = route(&command, address, &token, &content, &mut session);
        assert_eq!(response.status, 400);
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
            &content(),
            Duration::ZERO,
            &stop,
        )
        .unwrap_err();

        assert!(error.to_string().contains("timed out"));
    }
}
