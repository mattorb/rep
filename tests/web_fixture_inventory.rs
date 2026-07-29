use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/web")
}

#[test]
fn every_html_fixture_has_nonempty_expected_metadata() {
    let root = fixture_root();
    let mut html_stems = BTreeSet::new();
    let mut expected_stems = BTreeSet::new();

    for entry in fs::read_dir(&root).expect("read web fixture directory") {
        let entry = entry.expect("read fixture entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("read fixture file type");
        assert!(
            !file_type.is_symlink(),
            "committed fixture must not be a symlink: {}",
            path.display()
        );
        if !file_type.is_file() {
            continue;
        }
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("html") => {
                html_stems.insert(
                    path.file_stem()
                        .expect("HTML fixture stem")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            Some("json") if path.to_string_lossy().ends_with(".expected.json") => {
                let name = path.file_name().expect("expected fixture file name");
                let name = name.to_string_lossy();
                let stem = name
                    .strip_suffix(".expected.json")
                    .expect("expected metadata suffix");
                expected_stems.insert(stem.to_string());
                let contents = fs::read_to_string(&path).expect("read expected metadata");
                assert!(
                    contents.trim().len() > 2,
                    "expected metadata must be nonempty: {}",
                    path.display()
                );
            }
            _ => {}
        }
    }

    assert!(
        !html_stems.is_empty(),
        "web fixture corpus must not be empty"
    );
    assert_eq!(
        html_stems, expected_stems,
        "every HTML fixture must have one .expected.json description"
    );
}

#[test]
fn fixture_assets_are_regular_files_beneath_fixture_root() {
    let root = fixture_root()
        .canonicalize()
        .expect("canonical fixture root");
    let assets = root.join("assets");
    for entry in fs::read_dir(&assets).expect("read fixture assets") {
        let entry = entry.expect("read asset entry");
        let path = entry.path();
        let file_type = entry.file_type().expect("read asset type");
        assert!(file_type.is_file(), "asset must be a regular file");
        assert!(!file_type.is_symlink(), "asset must not be a symlink");
        assert!(
            path.canonicalize()
                .expect("canonical asset")
                .starts_with(&root),
            "asset escaped fixture root: {}",
            path.display()
        );
    }
}
