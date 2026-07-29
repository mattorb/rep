use std::ops::Range;

use anyhow::{Result, bail};

#[derive(Debug, Clone)]
pub(crate) struct TransformedHtml {
    pub(crate) source: String,
    pub(crate) blocked_resources: usize,
}

#[derive(Debug)]
struct Edit {
    range: Range<usize>,
    replacement: String,
}

#[derive(Debug)]
struct Attribute {
    name: String,
    name_range: Range<usize>,
    full_range: Range<usize>,
    value_range: Option<Range<usize>>,
}

pub(crate) fn transform(source: &str, token: &str) -> Result<TransformedHtml> {
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("invalid web session token");
    }
    let bytes = source.as_bytes();
    let mut edits = Vec::new();
    let mut cursor = 0;
    let mut source_id = 0_u64;
    let mut blocked_resources = 0_usize;

    while let Some(relative) = bytes[cursor..].iter().position(|byte| *byte == b'<') {
        let start = cursor + relative;
        if bytes[start..].starts_with(b"<!--") {
            cursor = find_bytes(bytes, start + 4, b"-->").map_or(bytes.len(), |end| end + 3);
            continue;
        }
        let Some(end) = find_tag_end(bytes, start + 1) else {
            break;
        };
        let Some((closing, name, name_end)) = parse_tag_name(source, start, end) else {
            cursor = end;
            continue;
        };
        if closing {
            cursor = end;
            continue;
        }

        let attributes = parse_attributes(source, name_end, end.saturating_sub(1));
        if is_removed_whole_element(&name) {
            let removal_end = find_closing_element(bytes, end, &name).unwrap_or(end);
            edits.push(Edit {
                range: start..removal_end,
                replacement: String::new(),
            });
            blocked_resources += 1;
            cursor = removal_end;
            continue;
        }
        if name == "base" || is_forbidden_meta(&name, &attributes, source) {
            edits.push(Edit {
                range: start..end,
                replacement: String::new(),
            });
            blocked_resources += 1;
            cursor = end;
            continue;
        }

        for attribute in &attributes {
            if attribute.name.starts_with("on")
                || attribute.name == "srcdoc"
                || attribute.name.starts_with("data-rep-")
                || attribute.name == "srcset"
                || is_form_navigation_attribute(&name, &attribute.name)
            {
                edits.push(Edit {
                    range: attribute.full_range.clone(),
                    replacement: String::new(),
                });
                blocked_resources += 1;
                continue;
            }

            if matches!(name.as_str(), "a" | "area") && attribute.name == "href" {
                let unsafe_url = attribute
                    .value_range
                    .as_ref()
                    .map(|range| &source[range.clone()])
                    .is_some_and(is_executable_navigation_url);
                edits.push(if unsafe_url {
                    blocked_resources += 1;
                    Edit {
                        range: attribute.full_range.clone(),
                        replacement: String::new(),
                    }
                } else {
                    Edit {
                        range: attribute.name_range.clone(),
                        replacement: "data-rep-original-href".to_string(),
                    }
                });
                continue;
            }
            if matches!(name.as_str(), "a" | "area")
                && matches!(attribute.name.as_str(), "target" | "download")
            {
                edits.push(Edit {
                    range: attribute.full_range.clone(),
                    replacement: String::new(),
                });
                continue;
            }

            if is_resource_attribute(&name, &attribute.name, &attributes, source)
                && let Some(value_range) = &attribute.value_range
            {
                let value = &source[value_range.clone()];
                match safe_local_resource_url(value, token, &name) {
                    Some(replacement) => edits.push(Edit {
                        range: value_range.clone(),
                        replacement,
                    }),
                    None => {
                        edits.push(Edit {
                            range: attribute.full_range.clone(),
                            replacement: String::new(),
                        });
                        blocked_resources += 1;
                    }
                }
            }
        }

        let insertion = if end >= 2 && bytes[end - 2] == b'/' {
            end - 2
        } else {
            end - 1
        };
        let source_line = bytes[..start].iter().filter(|byte| **byte == b'\n').count() + 1;
        edits.push(Edit {
            range: insertion..insertion,
            replacement: format!(
                " data-rep-source-id=\"{source_id}\" data-rep-source-line=\"{source_line}\""
            ),
        });
        source_id += 1;
        cursor = end;
    }

    edits.sort_by(|left, right| {
        right
            .range
            .start
            .cmp(&left.range.start)
            .then_with(|| right.range.end.cmp(&left.range.end))
    });
    let mut transformed = source.to_string();
    let mut previous_start = source.len() + 1;
    for edit in edits {
        if edit.range.end > source.len() || edit.range.start > edit.range.end {
            bail!("HTML source transformer produced an invalid edit");
        }
        if edit.range.end > previous_start {
            bail!("HTML source transformer produced overlapping edits");
        }
        transformed.replace_range(edit.range.clone(), &edit.replacement);
        previous_start = edit.range.start;
    }

    Ok(TransformedHtml {
        source: transformed,
        blocked_resources,
    })
}

fn find_tag_end(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    let mut quote = None;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        match quote {
            Some(active) if byte == active => quote = None,
            Some(_) => {}
            None if matches!(byte, b'\'' | b'"') => quote = Some(byte),
            None if byte == b'>' => return Some(cursor + 1),
            None => {}
        }
        cursor += 1;
    }
    None
}

fn parse_tag_name(source: &str, start: usize, end: usize) -> Option<(bool, String, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = start + 1;
    while cursor < end && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    if matches!(bytes.get(cursor), Some(b'!') | Some(b'?')) {
        return None;
    }
    let closing = bytes.get(cursor) == Some(&b'/');
    if closing {
        cursor += 1;
    }
    while cursor < end && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    let name_start = cursor;
    while cursor < end
        && (bytes[cursor].is_ascii_alphanumeric() || matches!(bytes[cursor], b':' | b'-'))
    {
        cursor += 1;
    }
    (cursor > name_start).then(|| {
        (
            closing,
            source[name_start..cursor].to_ascii_lowercase(),
            cursor,
        )
    })
}

fn parse_attributes(source: &str, mut cursor: usize, tag_close: usize) -> Vec<Attribute> {
    let bytes = source.as_bytes();
    let mut attributes = Vec::new();
    while cursor < tag_close {
        let whitespace_start = cursor;
        while cursor < tag_close && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag_close || bytes[cursor] == b'/' {
            break;
        }
        let name_start = cursor;
        while cursor < tag_close
            && !bytes[cursor].is_ascii_whitespace()
            && !matches!(bytes[cursor], b'=' | b'/' | b'>')
        {
            cursor += 1;
        }
        if cursor == name_start {
            cursor += 1;
            continue;
        }
        let name_end = cursor;
        while cursor < tag_close && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let mut value_range = None;
        if cursor < tag_close && bytes[cursor] == b'=' {
            cursor += 1;
            while cursor < tag_close && bytes[cursor].is_ascii_whitespace() {
                cursor += 1;
            }
            if cursor < tag_close && matches!(bytes[cursor], b'\'' | b'"') {
                let quote = bytes[cursor];
                cursor += 1;
                let value_start = cursor;
                while cursor < tag_close && bytes[cursor] != quote {
                    cursor += 1;
                }
                value_range = Some(value_start..cursor);
                if cursor < tag_close {
                    cursor += 1;
                }
            } else {
                let value_start = cursor;
                while cursor < tag_close
                    && !bytes[cursor].is_ascii_whitespace()
                    && !matches!(bytes[cursor], b'/' | b'>')
                {
                    cursor += 1;
                }
                value_range = Some(value_start..cursor);
            }
        }
        attributes.push(Attribute {
            name: source[name_start..name_end].to_ascii_lowercase(),
            name_range: name_start..name_end,
            full_range: whitespace_start..cursor,
            value_range,
        });
    }
    attributes
}

fn is_removed_whole_element(name: &str) -> bool {
    matches!(
        name,
        "script" | "noscript" | "iframe" | "frame" | "frameset" | "object" | "embed" | "applet"
    )
}

fn is_forbidden_meta(name: &str, attributes: &[Attribute], source: &str) -> bool {
    if name != "meta" {
        return false;
    }
    attribute_value(attributes, source, "http-equiv").is_some_and(|value| {
        value.eq_ignore_ascii_case("refresh")
            || value.eq_ignore_ascii_case("content-security-policy")
    })
}

fn is_form_navigation_attribute(element: &str, attribute: &str) -> bool {
    matches!(
        (element, attribute),
        ("form", "action" | "method" | "target")
            | ("button" | "input", "formaction" | "formtarget")
    )
}

fn is_executable_navigation_url(value: &str) -> bool {
    let trimmed = value.trim_start();
    ["javascript:", "vbscript:", "data:"].iter().any(|prefix| {
        trimmed
            .get(..prefix.len())
            .is_some_and(|value_prefix| value_prefix.eq_ignore_ascii_case(prefix))
    })
}

fn is_resource_attribute(
    element: &str,
    attribute: &str,
    attributes: &[Attribute],
    source: &str,
) -> bool {
    match (element, attribute) {
        ("img" | "input" | "source", "src") => true,
        ("image" | "use", "href" | "xlink:href") => true,
        ("link", "href") => attribute_value(attributes, source, "rel").is_some_and(|value| {
            value
                .split_ascii_whitespace()
                .any(|rel| rel.eq_ignore_ascii_case("stylesheet"))
        }),
        _ => false,
    }
}

fn attribute_value<'a>(
    attributes: &'a [Attribute],
    source: &'a str,
    name: &str,
) -> Option<&'a str> {
    attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .and_then(|attribute| attribute.value_range.clone())
        .map(|range| &source[range])
}

fn safe_local_resource_url(value: &str, token: &str, element: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("data:") {
        return (element == "img"
            && trimmed
                .get(..11)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:image/")))
        .then(|| trimmed.to_string());
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') || trimmed.starts_with("//") {
        return None;
    }
    let path = trimmed.split(['?', '#']).next().unwrap_or(trimmed);
    let decoded = percent_decode(path.as_bytes())?;
    let decoded = std::str::from_utf8(&decoded).ok()?;
    if decoded.contains('\0')
        || decoded.contains('\\')
        || decoded
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        || decoded
            .split('/')
            .next()
            .is_some_and(|first| first.contains(':'))
    {
        return None;
    }
    Some(format!(
        "/session/{token}/assets/{}",
        percent_encode_path(decoded.as_bytes())
    ))
}

pub(crate) fn percent_decode(input: &[u8]) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        if input[cursor] == b'%' {
            let high = *input.get(cursor + 1)?;
            let low = *input.get(cursor + 2)?;
            output.push(hex_value(high)? << 4 | hex_value(low)?);
            cursor += 3;
        } else {
            output.push(input[cursor]);
            cursor += 1;
        }
    }
    Some(output)
}

fn percent_encode_path(input: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::new();
    for &byte in input {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            output.push(byte as char);
        } else {
            output.push('%');
            output.push(HEX[(byte >> 4) as usize] as char);
            output.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    output
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn find_closing_element(bytes: &[u8], from: usize, name: &str) -> Option<usize> {
    let needle = format!("</{name}").into_bytes();
    let start = find_ascii_case_insensitive(bytes, from, &needle)?;
    find_tag_end(bytes, start + needle.len())
}

fn find_ascii_case_insensitive(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes[from..]
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
        .map(|position| from + position)
}

fn find_bytes(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    bytes[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| from + position)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn security_fixture_removes_active_content_and_blocks_unsafe_urls() {
        let source = include_str!("../../tests/fixtures/web/security.html");
        let transformed = transform(source, TOKEN).unwrap();
        let lower = transformed.source.to_ascii_lowercase();

        for forbidden in [
            "<script",
            "<base",
            "http-equiv=\"refresh\"",
            "content-security-policy",
            "onload=",
            "onclick=",
            "<iframe",
            "<object",
            "javascript:",
            "https://example.com/remote.css",
            "https://example.com/tracker.png",
            "../../outside.png",
            "/root-relative.png",
        ] {
            assert!(!lower.contains(forbidden), "still contains {forbidden}");
        }
        assert!(transformed.source.contains("<form"));
        assert!(!transformed.source.contains(" action="));
        assert!(transformed.source.contains(">Submit</button>"));
        assert!(transformed.blocked_resources >= 10);
    }

    #[test]
    fn layout_fixture_keeps_markup_css_and_rewrites_local_assets() {
        let source = include_str!("../../tests/fixtures/web/layout.html");
        let transformed = transform(source, TOKEN).unwrap();

        assert!(transformed.source.contains("class=\"board\""));
        assert!(
            transformed
                .source
                .contains(&format!("/session/{TOKEN}/assets/assets/plan.css"))
        );
        assert!(
            transformed
                .source
                .contains(&format!("/session/{TOKEN}/assets/assets/diagram.svg"))
        );
        assert!(transformed.source.contains("data-rep-source-line=\""));
        assert_eq!(transformed.blocked_resources, 0);
    }

    #[test]
    fn source_line_metadata_uses_original_lines() {
        let transformed = transform("<h1>One</h1>\n\n<p>Two</p>", TOKEN).unwrap();
        assert!(
            transformed
                .source
                .contains("<h1 data-rep-source-id=\"0\" data-rep-source-line=\"1\">")
        );
        assert!(
            transformed
                .source
                .contains("<p data-rep-source-id=\"1\" data-rep-source-line=\"3\">")
        );
    }

    #[test]
    fn malformed_html_remains_textually_intact_for_browser_repair() {
        let source = include_str!("../../tests/fixtures/web/malformed.html");
        let transformed = transform(source, TOKEN).unwrap();
        for visible in [
            "Browser repair behavior",
            "A paragraph closed by the next block.",
            "First item",
            "Second cell",
        ] {
            assert!(transformed.source.contains(visible));
        }
    }

    #[test]
    fn percent_decoding_is_exact_and_rejects_malformed_sequences() {
        assert_eq!(percent_decode(b"a%20b").unwrap(), b"a b");
        assert!(percent_decode(b"%").is_none());
        assert!(percent_decode(b"%GG").is_none());
    }
}
