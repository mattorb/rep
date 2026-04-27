//! Phase 4/5 emit-matrix harness per modular_plan.md §F.
//!
//! Each fixture under `tests/fixtures/emit/<fixture>/` contributes a
//! Cartesian matrix of (unit × action) cells; each cell stores its
//! byte-exact `to_human_output` at `<fixture>/<unit>/<action>.golden.txt`.
//!
//! The runner is presence-driven: a cell exists iff its golden file
//! exists. Missing cells are silently skipped (e.g. `delete this` on a
//! word in a fixture with no words). Use `UPDATE_GOLDENS=1 cargo test
//! --test emit_matrix` to regenerate mismatching goldens.
//!
//! Coverage scope today is the **first anchor** of each unit on each
//! fixture; the synthetic action text is `X` for change / feedback /
//! insert (so emit shape is what's under test, not user input). Word /
//! Sentence / Line / Paragraph / Section units cover the five axes; the
//! five actions are change, feedback, insert-before, insert-after, and
//! strike (renders as `delete this`).

mod common;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::{Path, PathBuf};

use common::{assert_golden, replay};

#[derive(Debug, Clone, Copy)]
struct Unit {
    name: &'static str,
    /// Backspace count to cycle backward from the default Sentence
    /// anchor; 0 = stay on Sentence; negative would represent forward
    /// cycles (we use Space x1 for Word instead).
    cycle: CycleStep,
}

#[derive(Debug, Clone, Copy)]
enum CycleStep {
    /// Stay on Sentence (the App::load default unit).
    Sentence,
    /// `Backspace` N times: 1 = Line, 2 = Paragraph, 3 = Section.
    Backspace(u8),
    /// `Space` once = forward one cycle position from Sentence to Word.
    SpaceOnce,
}

const UNITS: &[Unit] = &[
    Unit {
        name: "section",
        cycle: CycleStep::Backspace(3),
    },
    Unit {
        name: "paragraph",
        cycle: CycleStep::Backspace(2),
    },
    Unit {
        name: "line",
        cycle: CycleStep::Backspace(1),
    },
    Unit {
        name: "sentence",
        cycle: CycleStep::Sentence,
    },
    Unit {
        name: "word",
        cycle: CycleStep::SpaceOnce,
    },
];

#[derive(Debug, Clone, Copy)]
struct Action {
    name: &'static str,
    /// The keystroke that opens the action's input mode (or commits the
    /// strike directly).
    enter_key: KeyCode,
    /// True if the action takes free-form text. False = strike (no text).
    needs_payload: bool,
}

const ACTIONS: &[Action] = &[
    Action {
        name: "change",
        enter_key: KeyCode::Char('c'),
        needs_payload: true,
    },
    Action {
        name: "feedback",
        enter_key: KeyCode::Char('f'),
        needs_payload: true,
    },
    Action {
        name: "insert-before",
        enter_key: KeyCode::Char('b'),
        needs_payload: true,
    },
    Action {
        name: "insert-after",
        enter_key: KeyCode::Char('a'),
        needs_payload: true,
    },
    Action {
        name: "strike",
        enter_key: KeyCode::Char('x'),
        needs_payload: false,
    },
];

fn cycle_keys(cycle: CycleStep) -> Vec<KeyEvent> {
    match cycle {
        CycleStep::Sentence => Vec::new(),
        CycleStep::Backspace(n) => (0..n)
            .map(|_| KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE))
            .collect(),
        CycleStep::SpaceOnce => vec![KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)],
    }
}

fn action_keys(action: Action) -> Vec<KeyEvent> {
    let mut keys = vec![KeyEvent::new(action.enter_key, KeyModifiers::NONE)];
    if action.needs_payload {
        keys.push(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE));
        keys.push(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    }
    keys.push(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
    keys
}

fn emit_matrix_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("emit")
}

#[derive(Debug)]
struct EmitFixture {
    name: String,
    input: PathBuf,
}

fn discover_fixtures(root: &Path) -> Vec<EmitFixture> {
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
        if input.exists() {
            out.push(EmitFixture {
                name: path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                input,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[test]
fn emit_matrix() {
    let root = emit_matrix_root();
    let fixtures = discover_fixtures(&root);
    if fixtures.is_empty() {
        // No emit fixtures yet — this is a compile gate until cells are
        // authored. Use UPDATE_GOLDENS=1 once cells are seeded.
        return;
    }

    let update = std::env::var("UPDATE_GOLDENS").as_deref() == Ok("1");
    let mut failures = Vec::new();

    for fixture in &fixtures {
        let fx_dir = root.join(&fixture.name);
        for unit in UNITS {
            for action in ACTIONS {
                let golden = fx_dir
                    .join(unit.name)
                    .join(format!("{}.golden.txt", action.name));
                if !golden.exists() && !update {
                    // Cell not authored — skip silently. Authors create
                    // the empty file (or `UPDATE_GOLDENS=1`) to opt in.
                    continue;
                }

                let mut keys = cycle_keys(unit.cycle);
                keys.extend(action_keys(*action));
                let (emit, _anchor) = replay(&fixture.input, &keys);

                if update && !golden.exists() {
                    if let Some(parent) = golden.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                }

                if let Err(msg) = compare_or_update(&emit, &golden, update) {
                    failures.push(format!(
                        "{}/{}/{}: {msg}",
                        fixture.name, unit.name, action.name
                    ));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "emit matrix failures:\n{}",
        failures.join("\n")
    );
}

fn compare_or_update(actual: &str, golden: &Path, update: bool) -> Result<(), String> {
    if update {
        std::fs::write(golden, actual).map_err(|e| format!("write {}: {e}", golden.display()))?;
        return Ok(());
    }
    let _ = assert_golden;
    let expected = std::fs::read_to_string(golden)
        .map_err(|_| format!("missing golden: {}", golden.display()))?;
    if actual != expected {
        let actual_path = golden.with_extension("actual.txt");
        let _ = std::fs::write(&actual_path, actual);
        return Err(format!(
            "golden mismatch:\n  diff -u {} {}",
            golden.display(),
            actual_path.display()
        ));
    }
    Ok(())
}
