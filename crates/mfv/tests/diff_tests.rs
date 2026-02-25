use std::fs;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use assert_fs::TempDir;
use predicates::prelude::*;

fn mfv() -> assert_cmd::Command {
    cargo_bin_cmd!("mfv").into()
}

fn write_schema(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

fn write_md(dir: &Path, name: &str, frontmatter: &str, body: &str) {
    let content = if frontmatter.is_empty() {
        body.to_string()
    } else {
        format!("---\n{frontmatter}---\n{body}")
    };
    fs::write(dir.join(name), content).unwrap();
}

#[test]
fn diff_no_changes() {
    let tmp = TempDir::new().unwrap();
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    // Init to create config + lock inside tmp dir
    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(tmp.path().join("mfv.toml"))
        .assert()
        .success();

    // Diff should show no changes
    mfv()
        .args(["diff", "--dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("No changes detected"));
}

#[test]
fn diff_detects_new_field() {
    let tmp = TempDir::new().unwrap();
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    // Init
    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(tmp.path().join("mfv.toml"))
        .assert()
        .success();

    // Add a field to the file
    write_md(tmp.path(), "note.md", "title: Hello\ntags: [a]\n", "Body");

    // Diff should detect the new field
    mfv()
        .args(["diff", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Fields added"))
        .stdout(predicate::str::contains("tags"));
}

#[test]
fn diff_detects_removed_field() {
    let tmp = TempDir::new().unwrap();
    // Two files so tags isn't inferred as required everywhere
    write_md(tmp.path(), "note.md", "title: Hello\ntags: [a]\n", "Body");
    write_md(tmp.path(), "other.md", "title: Hello\n", "Body");

    // Init
    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(tmp.path().join("mfv.toml"))
        .assert()
        .success();

    // Remove the tags field from note.md
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    // Diff should detect the removed field
    mfv()
        .args(["diff", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Fields removed"))
        .stdout(predicate::str::contains("tags"));
}

#[test]
fn diff_exits_1_on_changes() {
    let tmp = TempDir::new().unwrap();
    // Two files so title isn't inferred as required everywhere
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");
    write_md(tmp.path(), "second.md", "draft: true\n", "Body");

    // Init
    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(tmp.path().join("mfv.toml"))
        .assert()
        .success();

    // Add a new file with a new field
    write_md(tmp.path(), "other.md", "category: tech\n", "Body");

    // Diff should exit 1
    mfv()
        .args(["diff", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("Summary"));
}

#[test]
fn diff_no_lock_exit_2() {
    let tmp = TempDir::new().unwrap();
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );

    // No lock file exists
    mfv()
        .args(["diff", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no lock file found"));
}

#[test]
fn diff_fails_on_validation_error() {
    let tmp = TempDir::new().unwrap();
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    // Init
    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(tmp.path().join("mfv.toml"))
        .assert()
        .success();

    // Modify schema to require a field that doesn't exist
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"

[[fields.field]]
name = "author"
type = "string"
required = ["**"]
"#,
    );

    // Diff should fail due to validation
    mfv()
        .args(["diff", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("validation error"));
}

#[test]
fn diff_ignore_errors_continues() {
    let tmp = TempDir::new().unwrap();
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    // Init
    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(tmp.path().join("mfv.toml"))
        .assert()
        .success();

    // Modify schema to require a field that doesn't exist
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"

[[fields.field]]
name = "author"
type = "string"
required = ["**"]
"#,
    );

    // Diff with --ignore-errors should continue
    mfv()
        .args(["diff", "--dir"])
        .arg(tmp.path())
        .arg("--ignore-validation-errors")
        .assert()
        // Should not exit 2 (validation error), should complete the diff
        .stdout(predicate::str::contains("No changes").or(predicate::str::contains("Summary")));
}
