//! Integration tests for the stderr warning about integers that cannot widen
//! exactly to Float.
//!
//! The warning is printed only for fields a command is about to write to
//! `mdvs.toml`. Once the user declares the field another way or ignores it,
//! later runs must stay silent. These tests run the real binary so they can
//! observe stderr.

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

/// Substring every inexact-widening warning line carries.
const WARNING_MARKER: &str = "widens integers to Float";

/// 2^53 + 1, the smallest positive integer with no exact f64 equivalent.
const BEYOND_F64_EXACT: &str = "9007199254740993";

fn mdvs(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mdvs"))
        .args(args)
        .arg(dir)
        .output()
        .expect("run mdvs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn write_note(dir: &Path, name: &str, frontmatter: &str) {
    fs::write(dir.join(name), format!("---\n{frontmatter}\n---\nBody.\n")).expect("write note");
}

/// Writes the two notes that make `score` infer as Float with
/// `widen-int-to-float` while holding an integer beyond 2^53.
fn write_score_notes(dir: &Path) {
    write_note(dir, "a.md", "title: a\nscore: 0.5");
    write_note(dir, "b.md", &format!("title: b\nscore: {BEYOND_F64_EXACT}"));
}

fn edit_config(dir: &Path, from: &str, to: &str) {
    let path = dir.join("mdvs.toml");
    let text = fs::read_to_string(&path).expect("read mdvs.toml");
    assert!(text.contains(from), "mdvs.toml lacks {from:?}:\n{text}");
    fs::write(&path, text.replacen(from, to, 1)).expect("write mdvs.toml");
}

#[test]
fn init_warns_for_inferred_field() {
    let tmp = tempfile::tempdir().unwrap();
    write_score_notes(tmp.path());
    let out = mdvs(&["init"], tmp.path());
    assert!(out.status.success(), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(err.contains(WARNING_MARKER), "{err}");
    assert!(err.contains("'score'"), "{err}");
    assert!(err.contains(BEYOND_F64_EXACT), "{err}");
}

#[test]
fn check_warns_for_newly_added_field() {
    let tmp = tempfile::tempdir().unwrap();
    write_note(tmp.path(), "a.md", "title: a");
    assert!(mdvs(&["init"], tmp.path()).status.success());
    write_score_notes(tmp.path());
    let err = stderr(&mdvs(&["check"], tmp.path()));
    assert!(err.contains(WARNING_MARKER), "{err}");
}

#[test]
fn update_warns_for_newly_added_field() {
    let tmp = tempfile::tempdir().unwrap();
    write_note(tmp.path(), "a.md", "title: a");
    assert!(mdvs(&["init"], tmp.path()).status.success());
    write_score_notes(tmp.path());
    let err = stderr(&mdvs(&["update"], tmp.path()));
    assert!(err.contains(WARNING_MARKER), "{err}");
}

#[test]
fn declared_field_is_silent_on_check_and_update() {
    let tmp = tempfile::tempdir().unwrap();
    write_score_notes(tmp.path());
    assert!(mdvs(&["init"], tmp.path()).status.success());
    edit_config(tmp.path(), "type = \"Float\"", "type = \"String\"");
    edit_config(
        tmp.path(),
        "preprocess = [\"widen-int-to-float\"]",
        "preprocess = [\"coerce-to-string\"]",
    );

    let check = mdvs(&["check"], tmp.path());
    assert!(check.status.success(), "{}", stderr(&check));
    assert!(
        !stderr(&check).contains(WARNING_MARKER),
        "{}",
        stderr(&check)
    );

    let update = mdvs(&["update"], tmp.path());
    assert!(update.status.success(), "{}", stderr(&update));
    assert!(
        !stderr(&update).contains(WARNING_MARKER),
        "{}",
        stderr(&update)
    );
}

#[test]
fn ignored_field_is_silent_on_check_and_update() {
    let tmp = tempfile::tempdir().unwrap();
    write_note(tmp.path(), "a.md", "title: a");
    assert!(mdvs(&["init"], tmp.path()).status.success());
    edit_config(tmp.path(), "ignore = []", "ignore = [\"score\"]");
    write_score_notes(tmp.path());

    let check = mdvs(&["check"], tmp.path());
    assert!(
        !stderr(&check).contains(WARNING_MARKER),
        "{}",
        stderr(&check)
    );

    let update = mdvs(&["update"], tmp.path());
    assert!(
        !stderr(&update).contains(WARNING_MARKER),
        "{}",
        stderr(&update)
    );
}
