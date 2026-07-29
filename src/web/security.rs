use std::net::SocketAddr;

use crate::web::protocol::{Request, Response};

pub(crate) fn validate_request(
    request: &Request,
    address: SocketAddr,
    token: &str,
) -> Result<(), Response> {
    let expected_host = address.to_string();
    if request.header("host") != Some(expected_host.as_str()) {
        return Err(Response::text(400, "invalid Host header"));
    }
    let prefix = format!("/session/{token}/");
    if !request.path.starts_with(&prefix) {
        return Err(Response::text(404, "not found"));
    }
    if request.method != "GET" && request.method != "HEAD" {
        let expected_origin = format!("http://{address}");
        if request.header("origin") != Some(expected_origin.as_str()) {
            return Err(Response::text(403, "invalid Origin header"));
        }
        if request.header("content-type") != Some("application/json") {
            return Err(Response::text(415, "expected application/json"));
        }
    }
    Ok(())
}

pub(crate) fn add_parent_headers(response: &mut Response) {
    response.headers.push((
        "Content-Security-Policy".to_string(),
        "default-src 'none'; style-src 'self'; script-src 'self'; frame-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
            .to_string(),
    ));
    add_common_headers(response);
}

pub(crate) fn add_api_headers(response: &mut Response) {
    response
        .headers
        .push(("Cache-Control".to_string(), "no-store".to_string()));
    add_common_headers(response);
}

pub(crate) fn add_document_headers(response: &mut Response) {
    response.headers.push((
        "Content-Security-Policy".to_string(),
        "default-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; sandbox allow-same-origin; base-uri 'none'; form-action 'none'; navigate-to 'none'"
            .to_string(),
    ));
    response
        .headers
        .push(("Cache-Control".to_string(), "no-store".to_string()));
    add_common_headers(response);
}

pub(crate) fn add_asset_headers(response: &mut Response) {
    response.headers.push((
        "Content-Security-Policy".to_string(),
        "default-src 'none'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; sandbox"
            .to_string(),
    ));
    response
        .headers
        .push(("Cache-Control".to_string(), "no-store".to_string()));
    add_common_headers(response);
}

fn add_common_headers(response: &mut Response) {
    response
        .headers
        .push(("X-Content-Type-Options".to_string(), "nosniff".to_string()));
    response
        .headers
        .push(("Referrer-Policy".to_string(), "no-referrer".to_string()));
    response.headers.push((
        "Cross-Origin-Resource-Policy".to_string(),
        "same-origin".to_string(),
    ));
}
