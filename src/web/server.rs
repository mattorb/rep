use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::output::render_human_output;
use crate::review::annotation::EditableAnnotation;
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
    terminal: Option<ReviewOutcome>,
}

impl WebSession {
    const fn new() -> Self {
        Self {
            document: None,
            review: None,
            revision: 0,
            terminal: None,
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
                stream
                    .set_nonblocking(false)
                    .context("failed to configure accepted loopback connection")?;
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
            // Browsers may open speculative loopback connections and abandon
            // them without sending a request. A reset client connection is
            // isolated to that connection and must not terminate the review.
            let _ = write_response(stream, &response, false);
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
    // A browser can cancel a fetch or close a tab while a response is being
    // written. The request has already been validated and applied, so preserve
    // any terminal outcome and keep non-terminal sessions available.
    let _ = write_response(stream, &response, head_only);
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
    let heartbeat = format!("{root}api/heartbeat");
    let output = format!("{root}api/output");
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
        ("GET" | "HEAD", path) if path == output => {
            let output = current_output(session);
            (
                Response::json(
                    200,
                    &serde_json::to_string(&serde_json::json!({ "output": output }))
                        .unwrap_or_else(|_| "{\"output\":\"\"}".to_string()),
                ),
                None,
            )
        }
        ("POST", path) if path == heartbeat => (Response::json(200, "{\"status\":\"ok\"}"), None),
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
                BrowserCommand::Search {
                    query,
                    forward,
                    revision: _,
                } => {
                    if query.len() > 1024 || query.trim().is_empty() {
                        return (Response::text(400, "search query is invalid"), None);
                    }
                    review
                        .apply(document, ReviewCommand::Search { query, forward })
                        .status
                }
                BrowserCommand::JumpSearch {
                    forward,
                    revision: _,
                } => {
                    review
                        .apply(document, ReviewCommand::JumpSearch { forward })
                        .status
                }
                BrowserCommand::JumpAnnotation {
                    forward,
                    revision: _,
                } => {
                    review
                        .apply(document, ReviewCommand::JumpAnnotation { forward })
                        .status
                }
                BrowserCommand::Annotate {
                    kind,
                    text,
                    revision: _,
                } => {
                    if text.len() > 1024 * 1024 || text.trim().is_empty() {
                        return (Response::text(400, "annotation text is invalid"), None);
                    }
                    let created_at = Utc::now().to_rfc3339();
                    match kind.as_str() {
                        "change" => Some(review.add_change(document, created_at, text)),
                        "feedback" => Some(review.add_feedback(document, created_at, text)),
                        "insertBefore" => Some(review.add_insert(document, created_at, text, true)),
                        "insertAfter" => Some(review.add_insert(document, created_at, text, false)),
                        _ => return (Response::text(400, "invalid annotation kind"), None),
                    }
                }
                BrowserCommand::Edit { text, revision: _ } => {
                    if text.len() > 1024 * 1024 || text.trim().is_empty() {
                        return (Response::text(400, "annotation text is invalid"), None);
                    }
                    let node_idx = review.anchor().node_idx;
                    match review.editable_annotation_at_cursor(document) {
                        Some(EditableAnnotation::Change(index)) => {
                            review.update_change(node_idx, index, text)
                        }
                        Some(EditableAnnotation::Feedback(index)) => {
                            review.update_feedback(node_idx, index, text)
                        }
                        None => {
                            return (
                                Response::text(409, "no editable annotation at selection"),
                                None,
                            );
                        }
                    }
                }
                BrowserCommand::Strike { revision: _ } => {
                    Some(review.toggle_strike(document, Utc::now().to_rfc3339()))
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
        ("POST", path) if path == finish => {
            terminal_action(session, ReviewOutcome::Submitted(current_output(session)))
        }
        ("POST", path) if path == discard => terminal_action(session, ReviewOutcome::Discarded),
        ("GET" | "HEAD", _) => (Response::text(404, "not found"), None),
        _ => (Response::text(405, "method not allowed"), None),
    }
}

fn terminal_action(
    session: &mut WebSession,
    requested: ReviewOutcome,
) -> (Response, Option<ReviewOutcome>) {
    if session.terminal.is_some() {
        return (
            Response::json(200, "{\"status\":\"already-complete\"}"),
            None,
        );
    }
    let response = match requested {
        ReviewOutcome::Submitted(_) => Response::json(200, "{\"status\":\"finished\"}"),
        ReviewOutcome::Discarded => Response::json(200, "{\"status\":\"discarded\"}"),
    };
    session.terminal = Some(requested.clone());
    (response, Some(requested))
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
    Search {
        revision: u64,
        query: String,
        forward: bool,
    },
    JumpSearch {
        revision: u64,
        forward: bool,
    },
    JumpAnnotation {
        revision: u64,
        forward: bool,
    },
    Annotate {
        revision: u64,
        kind: String,
        text: String,
    },
    Edit {
        revision: u64,
        text: String,
    },
    Strike {
        revision: u64,
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
            | Self::Search { revision, .. }
            | Self::JumpSearch { revision, .. }
            | Self::JumpAnnotation { revision, .. }
            | Self::Annotate { revision, .. }
            | Self::Edit { revision, .. }
            | Self::Strike { revision, .. }
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
    links: Vec<String>,
    annotations: Vec<AnnotationSlice>,
    annotation_count: usize,
    editable: Option<EditableSnapshot>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AnnotationSlice {
    kind: &'static str,
    node: usize,
    start: usize,
    end: usize,
    first: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EditableSnapshot {
    kind: &'static str,
    text: String,
}

fn state_response(content: &WebContent, session: &WebSession) -> Response {
    state_response_with_status(content, session, None)
}

fn state_response_with_status(
    content: &WebContent,
    session: &WebSession,
    message: Option<String>,
) -> Response {
    let (status, node_count, mode, anchor, selection, outline, links, annotations, editable) =
        match (&session.document, &session.review) {
            (Some(document), Some(review)) if document.node_count() == 0 => (
                "empty",
                0,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
            ),
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
                let links = document
                    .links_for(anchor)
                    .into_iter()
                    .map(|link| link.url)
                    .collect();
                let annotations = annotation_slices(document, review);
                let editable = editable_snapshot(document, review);
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
                    links,
                    annotation_count: annotation_count(review),
                    annotations,
                    editable,
                };
                return json_response(&snapshot);
            }
            _ => (
                "waiting",
                0,
                None,
                None,
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
            ),
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
        links,
        annotation_count: annotations.len(),
        annotations,
        editable,
    })
}

fn annotation_slices(
    document: &HtmlReviewDocument,
    review: &ReviewSession,
) -> Vec<AnnotationSlice> {
    let mut slices = Vec::new();
    let mut add = |kind: &'static str, anchor: SelectionAnchor| {
        slices.extend(
            document
                .selection_slices(anchor)
                .into_iter()
                .enumerate()
                .map(|(index, slice)| AnnotationSlice {
                    kind,
                    node: slice.node,
                    start: slice.start,
                    end: slice.end,
                    first: index == 0,
                }),
        );
    };
    for (&node, annotations) in &review.annotations.changes {
        for annotation in annotations {
            add(
                "change",
                SelectionAnchor::new(
                    node,
                    annotation.target_unit,
                    annotation.sentence_index.unwrap_or(0),
                ),
            );
        }
    }
    for (&node, annotations) in &review.annotations.feedbacks {
        for annotation in annotations {
            add(
                "feedback",
                SelectionAnchor::new(
                    node,
                    annotation.target_unit,
                    annotation.sentence_index.unwrap_or(0),
                ),
            );
        }
    }
    for (&node, annotations) in &review.annotations.inserts_before {
        for annotation in annotations {
            add(
                "insertBefore",
                SelectionAnchor::new(
                    node,
                    annotation.target_unit,
                    annotation.sentence_index.unwrap_or(0),
                ),
            );
        }
    }
    for (&node, annotations) in &review.annotations.inserts_after {
        for annotation in annotations {
            add(
                "insertAfter",
                SelectionAnchor::new(
                    node,
                    annotation.target_unit,
                    annotation.sentence_index.unwrap_or(0),
                ),
            );
        }
    }
    for (&node, strikes) in &review.annotations.strikes {
        for &(unit, index) in strikes {
            add("strike", SelectionAnchor::new(node, unit, index));
        }
    }
    slices
}

fn annotation_count(review: &ReviewSession) -> usize {
    review
        .annotations
        .changes
        .values()
        .map(Vec::len)
        .sum::<usize>()
        + review
            .annotations
            .feedbacks
            .values()
            .map(Vec::len)
            .sum::<usize>()
        + review
            .annotations
            .inserts_before
            .values()
            .map(Vec::len)
            .sum::<usize>()
        + review
            .annotations
            .inserts_after
            .values()
            .map(Vec::len)
            .sum::<usize>()
        + review
            .annotations
            .strikes
            .values()
            .map(std::collections::BTreeSet::len)
            .sum::<usize>()
}

fn editable_snapshot(
    document: &HtmlReviewDocument,
    review: &ReviewSession,
) -> Option<EditableSnapshot> {
    let node = review.anchor().node_idx;
    match review.editable_annotation_at_cursor(document)? {
        EditableAnnotation::Change(index) => Some(EditableSnapshot {
            kind: "change",
            text: review
                .annotations
                .changes
                .get(&node)?
                .get(index)?
                .change
                .clone(),
        }),
        EditableAnnotation::Feedback(index) => Some(EditableSnapshot {
            kind: "feedback",
            text: review
                .annotations
                .feedbacks
                .get(&node)?
                .get(index)?
                .feedback
                .clone(),
        }),
    }
}

fn current_output(session: &WebSession) -> String {
    match (&session.document, &session.review) {
        (Some(document), Some(review)) => {
            render_human_output(&review.emit_model(document, Utc::now().to_rfc3339()))
                .trim_end_matches('\n')
                .to_string()
        }
        _ => "No actions.".to_string(),
    }
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
    use std::io::{Read, Write};
    use std::net::Shutdown;

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
                    element_summary: if index == 0 {
                        "h1#plan".to_string()
                    } else {
                        "p.review-node".to_string()
                    },
                    text: (*text).to_string(),
                    logical_lines: vec![ScalarRange {
                        start: 0,
                        end: text.chars().count(),
                    }],
                    selector: format!("body > :nth-child({})", index + 1),
                    text_fragment: None,
                    heading_level: (index == 0).then_some(1),
                    section_start: (index == 0).then_some(0),
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
    fn heartbeat_keeps_the_session_alive_without_mutating_review_state() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "8".repeat(64);
        let content = content();
        let root = format!("/session/{token}/");
        let mut session = WebSession::new();
        let mut initialize = request("POST", &format!("{root}api/manifest"), address);
        initialize.body = serde_json::to_vec(&manifest(&["Plan"])).unwrap();
        let (response, _) = route(&initialize, address, &token, &content, &mut session);
        assert_eq!(response.status, 200);

        let revision = session.revision;
        let heartbeat = request("POST", &format!("{root}api/heartbeat"), address);
        let (response, outcome) = route(&heartbeat, address, &token, &content, &mut session);

        assert_eq!(response.status, 200);
        assert!(outcome.is_none());
        assert_eq!(session.revision, revision);
        assert!(session.terminal.is_none());
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
    fn first_terminal_action_wins_and_freezes_the_outcome() {
        let address: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let token = "9".repeat(64);
        let content = content();
        let root = format!("/session/{token}/");
        let mut session = WebSession::new();
        let finish = request("POST", &format!("{root}api/finish"), address);
        let discard = request("POST", &format!("{root}api/discard"), address);

        let (_, first) = route(&finish, address, &token, &content, &mut session);
        let (_, second) = route(&discard, address, &token, &content, &mut session);

        assert_eq!(
            first,
            Some(ReviewOutcome::Submitted("No actions.".to_string()))
        );
        assert!(second.is_none());
        assert_eq!(
            session.terminal,
            Some(ReviewOutcome::Submitted("No actions.".to_string()))
        );
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
        assert_eq!(
            session.review.as_ref().unwrap().anchor().unit,
            SelectionUnit::Section
        );

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
        assert_eq!(
            session.review.as_ref().unwrap().anchor(),
            SelectionAnchor::new(0, SelectionUnit::Section, 0),
            "the single section reports a boundary without changing anchors"
        );

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

    #[test]
    fn injected_interrupt_exits_without_a_terminal_outcome() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = AtomicBool::new(true);

        let error = serve(
            listener,
            address,
            &"6".repeat(64),
            &content(),
            Duration::from_secs(60),
            &stop,
        )
        .unwrap_err();

        assert!(error.to_string().contains("interrupted"));
    }

    #[test]
    fn abandoned_client_connection_does_not_end_the_session() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let token = "7".repeat(64);
        let content = content();
        let mut session = WebSession::new();

        let mut abandoned = TcpStream::connect(address).unwrap();
        abandoned
            .write_all(
                format!("GET /session/{token}/ HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes(),
            )
            .unwrap();
        abandoned.shutdown(Shutdown::Both).unwrap();
        let (mut abandoned_server, _) = listener.accept().unwrap();
        assert!(
            handle_connection(
                &mut abandoned_server,
                address,
                &token,
                &content,
                &mut session,
            )
            .unwrap()
            .is_none()
        );

        let mut healthy = TcpStream::connect(address).unwrap();
        healthy
            .write_all(
                format!("GET /session/{token}/api/health HTTP/1.1\r\nHost: {address}\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        let (mut healthy_server, _) = listener.accept().unwrap();
        assert!(
            handle_connection(&mut healthy_server, address, &token, &content, &mut session,)
                .unwrap()
                .is_none()
        );
        drop(healthy_server);
        let mut response = String::new();
        healthy.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    }
}
