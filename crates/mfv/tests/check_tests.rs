use std::fs;

use assert_cmd::cargo::cargo_bin_cmd;
use assert_fs::TempDir;
use predicates::prelude::*;

fn mfv() -> assert_cmd::Command {
    cargo_bin_cmd!("mfv").into()
}

/// Create a markdown file with YAML frontmatter.
fn write_md(dir: &std::path::Path, name: &str, frontmatter: &str, body: &str) {
    let content = if frontmatter.is_empty() {
        body.to_string()
    } else {
        format!("---\n{frontmatter}---\n{body}")
    };
    fs::write(dir.join(name), content).unwrap();
}

/// Create a schema file with the given TOML content.
fn write_schema(dir: &std::path::Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

#[test]
fn valid_files_exit_0() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("All files valid."));
}

#[test]
fn invalid_files_exit_1() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[directory]
include_bare_files = true

[[fields.field]]
name = "title"
type = "string"
required = ["**"]
"#,
    );
    write_md(tmp.path(), "note.md", "", "No frontmatter here.");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("required field missing"));
}

#[test]
fn wrong_type_exit_1() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );
    write_md(tmp.path(), "note.md", "title: 42\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("expected type"));
}

#[test]
fn no_config_exit_2() {
    let tmp = TempDir::new().unwrap();
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no config found"));
}

#[test]
fn missing_dir_exit_2() {
    mfv()
        .args(["check", "--dir", "/nonexistent/path"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is not a directory"));
}

#[test]
fn auto_discover_mfv_toml() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .success();
}

#[test]
fn auto_discover_mdvs_toml() {
    let tmp = TempDir::new().unwrap();
    // No mfv.toml, only mdvs.toml — should be picked up as fallback
    write_schema(
        tmp.path(),
        "mdvs.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .success();
}

#[test]
fn mfv_toml_precedence() {
    let tmp = TempDir::new().unwrap();

    // mfv.toml: only requires title (string) — note is valid
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );

    // mdvs.toml: requires "author" (required) — note would be invalid
    write_schema(
        tmp.path(),
        "mdvs.toml",
        r#"
[[fields.field]]
name = "author"
type = "string"
required = ["**"]
"#,
    );

    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    // Should succeed because mfv.toml takes precedence
    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .success();
}

#[test]
fn explicit_schema_overrides() {
    let tmp = TempDir::new().unwrap();

    // mfv.toml: lenient, just title
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );

    // strict.toml: requires "author"
    write_schema(
        tmp.path(),
        "strict.toml",
        r#"
[[fields.field]]
name = "author"
type = "string"
required = ["**"]
"#,
    );

    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    // Explicit --schema points to stricter file — should fail
    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .arg("--schema")
        .arg(tmp.path().join("strict.toml"))
        .assert()
        .code(1)
        .stdout(predicate::str::contains("required field missing"));
}

#[test]
fn format_json() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[directory]
include_bare_files = true

[[fields.field]]
name = "title"
type = "string"
required = ["**"]
"#,
    );
    write_md(tmp.path(), "note.md", "", "No frontmatter.");

    let output = mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .args(["--format", "json"])
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();

    let json: Vec<serde_json::Value> =
        serde_json::from_slice(&output).expect("stdout should be valid JSON");
    assert!(!json.is_empty());
    assert!(json[0]["file"].is_string());
    assert!(json[0]["field"].is_string());
    assert!(json[0]["message"].is_string());
}

#[test]
fn format_json_valid() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .args(["--format", "json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[]"));
}

#[test]
fn format_github() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[directory]
include_bare_files = true

[[fields.field]]
name = "title"
type = "string"
required = ["**"]
"#,
    );
    write_md(tmp.path(), "note.md", "", "No frontmatter.");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .args(["--format", "github"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("::error file="));
}

#[test]
fn not_allowed_field_exit_1() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "doi"
type = "string"
allowed = ["papers/**"]
"#,
    );
    // File outside papers/ has doi — should be not allowed
    fs::create_dir(tmp.path().join("blog")).unwrap();
    write_md(tmp.path().join("blog").as_path(), "post.md", "doi: 10.1234/test\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(1)
        .stdout(predicate::str::contains("not allowed"));
}

#[test]
fn unlisted_field_passes() {
    // Fields not in schema have no constraints — should pass validation
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );
    write_md(tmp.path(), "note.md", "title: Hello\nextra: oops\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("All files valid."));
}

#[test]
fn invalid_schema_exit_2() {
    let tmp = TempDir::new().unwrap();
    write_schema(tmp.path(), "mfv.toml", "this is not [valid toml = = =");
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(2);
}

#[test]
fn toml_frontmatter_validates() {
    let tmp = TempDir::new().unwrap();
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
required = ["**"]
"#,
    );

    // Write a file with TOML frontmatter (+++ delimiters)
    let content = "+++\ntitle = \"Hello from TOML\"\n+++\nBody text.";
    fs::write(tmp.path().join("note.md"), content).unwrap();

    mfv()
        .args(["check", "--dir"])
        .arg(tmp.path())
        .assert()
        .success();
}
