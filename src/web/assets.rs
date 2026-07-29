use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::web::html_source::percent_decode;

const MAX_ASSET_BYTES: u64 = 20 * 1024 * 1024;

pub(crate) struct LocalAsset {
    pub(crate) content_type: &'static str,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn load(root: &Path, encoded_path: &str) -> Result<LocalAsset> {
    let decoded = percent_decode(encoded_path.as_bytes())
        .context("asset path contains invalid percent encoding")?;
    if decoded.contains(&0) {
        bail!("asset path contains NUL");
    }
    let decoded = std::str::from_utf8(&decoded).context("asset path is not valid UTF-8")?;
    if decoded.contains('%') {
        bail!("double-encoded asset paths are not allowed");
    }
    if decoded.starts_with('/') || decoded.starts_with('\\') || decoded.contains('\\') {
        bail!("absolute asset paths are not allowed");
    }
    if decoded.split('/').any(str::is_empty) {
        bail!("empty asset path components are not allowed");
    }
    let relative = PathBuf::from(decoded);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("asset traversal is not allowed");
    }
    let candidate = root.join(relative);
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("asset not found: {decoded}"))?;
    if !canonical.starts_with(root) {
        bail!("asset resolves outside the plan directory");
    }
    let metadata = fs::metadata(&canonical).context("failed to inspect local asset")?;
    if !metadata.is_file() {
        bail!("asset is not a regular file");
    }
    if metadata.len() > MAX_ASSET_BYTES {
        bail!("asset exceeds the 20 MiB limit");
    }
    let content_type =
        content_type(&canonical).context("asset type is not allowed for HTML review")?;
    let bytes = fs::read(&canonical).context("failed to read local asset")?;
    Ok(LocalAsset {
        content_type,
        bytes,
    })
}

fn content_type(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "css" => Some("text/css; charset=utf-8"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "avif" => Some("image/avif"),
        "svg" => Some("image/svg+xml"),
        "ico" => Some("image/x-icon"),
        "woff" => Some("font/woff"),
        "woff2" => Some("font/woff2"),
        "ttf" => Some("font/ttf"),
        "otf" => Some("font/otf"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/web")
            .canonicalize()
            .unwrap()
    }

    #[test]
    fn serves_allowed_css_svg_spaces_and_unicode_beneath_root() {
        let root = fixture_root();
        let css = load(&root, "assets/plan.css").unwrap();
        assert_eq!(css.content_type, "text/css; charset=utf-8");
        let svg = load(&root, "assets/diagram.svg").unwrap();
        assert_eq!(svg.content_type, "image/svg+xml");

        let scratch = root.join("assets/space \u{2603}.css");
        fs::write(&scratch, "body{}").unwrap();
        let encoded = "assets/space%20%E2%98%83.css";
        assert!(load(&root, encoded).is_ok());
        fs::remove_file(scratch).unwrap();

        let font = root.join("assets/test-font.woff2");
        fs::write(&font, b"wOF2 fixture bytes").unwrap();
        assert_eq!(
            load(&root, "assets/test-font.woff2").unwrap().content_type,
            "font/woff2"
        );
        fs::remove_file(font).unwrap();

        let raster = root.join("assets/test-image.png");
        fs::write(&raster, b"\x89PNG fixture bytes").unwrap();
        assert_eq!(
            load(&root, "assets/test-image.png").unwrap().content_type,
            "image/png"
        );
        fs::remove_file(raster).unwrap();
    }

    #[test]
    fn rejects_missing_wrong_type_traversal_double_encoding_and_root_paths() {
        let root = fixture_root();
        for path in [
            "missing.css",
            "layout.html",
            "../security.html",
            "%2e%2e/security.html",
            "%252e%252e/security.html",
            "/etc/passwd",
            "assets//plan.css",
        ] {
            assert!(load(&root, path).is_err(), "{path} should be rejected");
        }
    }

    #[test]
    #[cfg(unix)]
    fn symlink_escaping_root_is_rejected() {
        use std::os::unix::fs::symlink;

        let root = fixture_root();
        let link = root.join("assets/escape.css");
        let outside = std::env::temp_dir().join(format!("rep-outside-{}.css", std::process::id()));
        fs::write(&outside, "body{}").unwrap();
        let _ = fs::remove_file(&link);
        symlink(&outside, &link).unwrap();

        assert!(load(&root, "assets/escape.css").is_err());

        fs::remove_file(link).unwrap();
        fs::remove_file(outside).unwrap();
    }
}
