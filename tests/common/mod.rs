//! Shared transcript driver for integration tests.
//!
//! See `implementation.md` § "Scaffolding" and `modular_plan.md` §
//! "Fixture tooling and goldens" for the specification.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rep::app::App;
use std::path::{Path, PathBuf};

/// Parse one canonical key name (per `keys.txt` line) into a `KeyEvent`.
///
/// Supported tokens (case-sensitive on names, case-preserving on chars):
/// - single ASCII chars: `j`, `k`, `c`, `q`, `0`, `?`, …
/// - named keys: `Space`, `Backspace`, `Enter`, `Escape`, `Tab`,
///   `Up`, `Down`, `Left`, `Right`
/// - modifier-prefixed: `Shift+J`, `Ctrl+C`, `Alt+x`, optionally combined
///   (`Ctrl+Shift+K`)
///
/// `# comment` lines and blank lines are filtered out by `parse_keys`.
pub fn parse_key(line: &str) -> KeyEvent {
    let trimmed = line.trim();
    let (mods, name) = split_modifiers(trimmed);
    let code = match name {
        "Space" => KeyCode::Char(' '),
        "Backspace" => KeyCode::Backspace,
        "Enter" => KeyCode::Enter,
        "Escape" | "Esc" => KeyCode::Esc,
        "Tab" => KeyCode::Tab,
        "Up" => KeyCode::Up,
        "Down" => KeyCode::Down,
        "Left" => KeyCode::Left,
        "Right" => KeyCode::Right,
        s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap()),
        other => panic!("unrecognized key token: {other:?}"),
    };
    KeyEvent::new(code, mods)
}

fn split_modifiers(s: &str) -> (KeyModifiers, &str) {
    let mut mods = KeyModifiers::NONE;
    let mut rest = s;
    loop {
        if let Some(stripped) = rest.strip_prefix("Shift+") {
            mods.insert(KeyModifiers::SHIFT);
            rest = stripped;
        } else if let Some(stripped) = rest.strip_prefix("Ctrl+") {
            mods.insert(KeyModifiers::CONTROL);
            rest = stripped;
        } else if let Some(stripped) = rest.strip_prefix("Alt+") {
            mods.insert(KeyModifiers::ALT);
            rest = stripped;
        } else {
            break;
        }
    }
    (mods, rest)
}

/// Parse a `keys.txt` body into a sequence of `KeyEvent`s.
pub fn parse_keys(body: &str) -> Vec<KeyEvent> {
    body.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(parse_key)
        .collect()
}

/// Replay a transcript: load `input.md` into a fresh `App`, dispatch each
/// `KeyEvent` from `keys`, and return the captured emit + final anchor in the
/// canonical golden format.
///
/// The emit is normalized so the absolute `FILE:` path is replaced with the
/// stable token `<INPUT>` — goldens are otherwise machine-dependent.
pub fn replay(input_md: &Path, keys: &[KeyEvent]) -> (String, String) {
    let mut app = App::load(input_md.to_path_buf()).expect("App::load failed");
    for &k in keys {
        app.handle_key(k);
        if app.should_quit {
            break;
        }
    }
    let raw_emit = app.to_human_output();
    let emit = normalize_emit(&raw_emit, input_md);
    let (node, unit, unit_idx) = app.current_anchor();
    let anchor = format!("anchor: ({node}, {unit}, {unit_idx})\n");
    (emit, anchor)
}

fn normalize_emit(emit: &str, input_md: &Path) -> String {
    let actual = input_md.display().to_string();
    emit.replace(&actual, "<INPUT>")
}

/// Byte-exact comparison against a golden file. Set `UPDATE_GOLDENS=1` to
/// overwrite mismatches and exit success.
pub fn assert_golden(actual: &str, path: &Path) {
    let update = std::env::var("UPDATE_GOLDENS").as_deref() == Ok("1");
    if update {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        std::fs::write(path, actual).expect("UPDATE_GOLDENS write failed");
        return;
    }
    let expected = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("missing golden: {}", path.display()));
    if actual != expected {
        let actual_path = path.with_extension("actual.txt");
        let _ = std::fs::write(&actual_path, actual);
        panic!(
            "golden mismatch:\n  expected: {}\n  actual:   {}\n  diff: diff -u {} {}",
            path.display(),
            actual_path.display(),
            path.display(),
            actual_path.display()
        );
    }
}

/// Discover transcripts under `tests/fixtures/transcripts/`. Each directory
/// containing `input.md` and `keys.txt` is one transcript. Returns sorted
/// names so test order is deterministic.
pub fn discover_transcripts(root: &Path) -> Vec<TranscriptFixture> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let input = path.join("input.md");
        let keys = path.join("keys.txt");
        if input.exists() && keys.exists() {
            out.push(TranscriptFixture {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                dir: path,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[derive(Debug)]
pub struct TranscriptFixture {
    pub name: String,
    pub dir: PathBuf,
}

impl TranscriptFixture {
    pub fn input_path(&self) -> PathBuf {
        self.dir.join("input.md")
    }
    pub fn keys_path(&self) -> PathBuf {
        self.dir.join("keys.txt")
    }
    pub fn emit_golden(&self) -> PathBuf {
        self.dir.join("emit.golden.txt")
    }
    pub fn anchor_golden(&self) -> PathBuf {
        self.dir.join("anchor.golden.txt")
    }
}

/// Path to the transcripts root directory.
pub fn transcripts_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("transcripts")
}
