//! Phase-0 transcript regression harness. Each fixture under
//! `tests/fixtures/transcripts/<name>/` is a self-contained transcript:
//!
//!   input.md         -- markdown source
//!   keys.txt         -- canonical keypress sequence (one key per line)
//!   emit.golden.txt  -- byte-exact `to_human_output` after replay
//!   anchor.golden.txt -- final `(node_idx, unit, unit_idx)` of selection
//!
//! Run with `UPDATE_GOLDENS=1 cargo test all_transcripts` to regenerate.

use crate::test_support::{
    TranscriptFixture, assert_golden, discover_transcripts, parse_keys, replay, transcripts_root,
};

#[test]
fn all_transcripts() {
    let root = transcripts_root();
    let fixtures = discover_transcripts(&root);
    if fixtures.is_empty() {
        // No fixtures yet -- the suite is still useful as a compile gate.
        return;
    }

    let mut failures = Vec::new();
    for fixture in fixtures {
        if let Err(msg) = run_one(&fixture) {
            failures.push(format!("{}: {msg}", fixture.name));
        }
    }
    assert!(
        failures.is_empty(),
        "transcript failures:\n{}",
        failures.join("\n")
    );
}

fn run_one(fixture: &TranscriptFixture) -> Result<(), String> {
    let keys_body =
        std::fs::read_to_string(fixture.keys_path()).map_err(|e| format!("read keys.txt: {e}"))?;
    let keys = parse_keys(&keys_body);
    let (emit, anchor) = replay(&fixture.input_path(), &keys);
    assert_golden(&emit, &fixture.emit_golden());
    assert_golden(&anchor, &fixture.anchor_golden());
    Ok(())
}
