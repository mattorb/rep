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
    response
        .headers
        .push(("Cache-Control".to_string(), "no-store".to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn header<'a>(response: &'a Response, name: &str) -> Option<&'a str> {
        response
            .headers
            .iter()
            .find_map(|(candidate, value)| (candidate == name).then_some(value.as_str()))
    }

    #[test]
    fn every_response_class_is_no_store_nosniff_and_no_referrer() {
        for add_headers in [
            add_parent_headers as fn(&mut Response),
            add_api_headers,
            add_document_headers,
            add_asset_headers,
        ] {
            let mut response = Response::text(200, "ok");
            add_headers(&mut response);
            assert_eq!(header(&response, "Cache-Control"), Some("no-store"));
            assert_eq!(header(&response, "X-Content-Type-Options"), Some("nosniff"));
            assert_eq!(header(&response, "Referrer-Policy"), Some("no-referrer"));
            assert_eq!(
                header(&response, "Cross-Origin-Resource-Policy"),
                Some("same-origin")
            );
        }
    }

    #[test]
    fn parent_and_document_csp_keep_plan_code_inert() {
        let mut parent = Response::text(200, "ok");
        add_parent_headers(&mut parent);
        let parent_csp = header(&parent, "Content-Security-Policy").unwrap();
        assert!(parent_csp.contains("script-src 'self'"));
        assert!(parent_csp.contains("frame-src 'self'"));

        let mut document = Response::text(200, "ok");
        add_document_headers(&mut document);
        let document_csp = header(&document, "Content-Security-Policy").unwrap();
        assert!(document_csp.contains("default-src 'none'"));
        assert!(document_csp.contains("sandbox allow-same-origin"));
        assert!(!document_csp.contains("script-src"));
        assert!(document_csp.contains("form-action 'none'"));
    }
}
