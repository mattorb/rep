use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use anyhow::{Context, Result, bail};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

impl Request {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

#[derive(Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    reason: &'static str,
    pub(crate) headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    pub(crate) fn new(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            reason: reason_phrase(status),
            headers: vec![("Content-Type".to_string(), content_type.to_string())],
            body,
        }
    }

    pub(crate) fn text(status: u16, body: &str) -> Self {
        Self::new(
            status,
            "text/plain; charset=utf-8",
            body.as_bytes().to_vec(),
        )
    }

    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self::new(
            status,
            "application/json; charset=utf-8",
            body.as_bytes().to_vec(),
        )
    }
}

pub(crate) fn read_request(stream: &mut TcpStream) -> Result<Request> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .context("failed to configure HTTP read timeout")?;
    let mut bytes = Vec::new();
    let header_end = loop {
        if bytes.len() >= MAX_HEADER_BYTES {
            bail!("HTTP request headers exceed 64 KiB");
        }
        let mut buffer = [0_u8; 4096];
        let count = stream
            .read(&mut buffer)
            .context("failed to read HTTP request")?;
        if count == 0 {
            bail!("connection closed before HTTP headers completed");
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = find_subslice(&bytes, b"\r\n\r\n") {
            break index + 4;
        }
    };

    let header_text =
        std::str::from_utf8(&bytes[..header_end]).context("HTTP headers must be UTF-8")?;
    let mut lines = header_text[..header_text.len() - 4].split("\r\n");
    let request_line = lines.next().context("missing HTTP request line")?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().context("missing HTTP method")?;
    let path = request_parts
        .next()
        .context("missing HTTP request target")?;
    let version = request_parts.next().context("missing HTTP version")?;
    if request_parts.next().is_some() || version != "HTTP/1.1" {
        bail!("unsupported HTTP request line");
    }
    if !matches!(method, "GET" | "HEAD" | "POST") {
        bail!("unsupported HTTP method");
    }
    if !path.starts_with('/') || path.contains('#') {
        bail!("invalid HTTP request target");
    }

    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').context("malformed HTTP header line")?;
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || headers.contains_key(&name) {
            bail!("empty or duplicate HTTP header");
        }
        headers.insert(name, value.trim().to_string());
    }
    let content_length = headers
        .get("content-length")
        .map_or(Ok(0), |value| value.parse::<usize>())
        .context("invalid Content-Length")?;
    if content_length > MAX_BODY_BYTES {
        bail!("HTTP request body exceeds 20 MiB");
    }
    let mut body = bytes[header_end..].to_vec();
    if body.len() > content_length {
        body.truncate(content_length);
    }
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let mut buffer = vec![0_u8; remaining.min(8192)];
        let count = stream
            .read(&mut buffer)
            .context("failed to read HTTP body")?;
        if count == 0 {
            bail!("connection closed before HTTP body completed");
        }
        body.extend_from_slice(&buffer[..count]);
    }
    Ok(Request {
        method: method.to_string(),
        path: path.split('?').next().unwrap_or(path).to_string(),
        headers,
        body,
    })
}

pub(crate) fn write_response(
    stream: &mut TcpStream,
    response: &Response,
    head_only: bool,
) -> Result<()> {
    let mut headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        response.status,
        response.reason,
        response.body.len()
    );
    for (name, value) in &response.headers {
        headers.push_str(name);
        headers.push_str(": ");
        headers.push_str(value);
        headers.push_str("\r\n");
    }
    headers.push_str("\r\n");
    stream
        .write_all(headers.as_bytes())
        .context("failed to write HTTP response headers")?;
    if !head_only {
        stream
            .write_all(&response.body)
            .context("failed to write HTTP response body")?;
    }
    stream.flush().context("failed to flush HTTP response")
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

const fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        405 => "Method Not Allowed",
        415 => "Unsupported Media Type",
        500 => "Internal Server Error",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_header_terminator() {
        assert_eq!(find_subslice(b"a\r\n\r\nb", b"\r\n\r\n"), Some(1));
        assert_eq!(find_subslice(b"abc", b"\r\n\r\n"), None);
    }

    #[test]
    fn response_constructors_pin_content_types() {
        assert_eq!(
            Response::text(404, "missing").headers[0].1,
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            Response::json(200, "{}").headers[0].1,
            "application/json; charset=utf-8"
        );
    }
}
