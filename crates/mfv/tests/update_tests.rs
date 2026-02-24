use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::cargo::cargo_bin_cmd;
use assert_fs::TempDir;
use predicates::prelude::*;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures")
}

fn mfv() -> assert_cmd::Command {
    cargo_bin_cmd!("mfv").into()
}

/// Create a schema file with the given TOML content.
fn write_schema(dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
}

/// Create a markdown file with YAML frontmatter.
fn write_md(dir: &Path, name: &str, frontmatter: &str, body: &str) {
    let content = if frontmatter.is_empty() {
        body.to_string()
    } else {
        format!("---\n{frontmatter}---\n{body}")
    };
    fs::write(dir.join(name), content).unwrap();
}

#[test]
fn update_refreshes_lock() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("mfv.toml");

    // Init first to create config + lock
    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--config")
        .arg(&config_path)
        .assert()
        .success();

    let lock_path = tmp.path().join("mfv.lock");
    assert!(lock_path.exists());
    let lock_before = fs::read_to_string(&lock_path).unwrap();

    // Small delay so timestamp differs
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Update to refresh lock
    mfv()
        .args(["update", "--dir"])
        .arg(fixtures_dir())
        .arg("--config")
        .arg(&config_path)
        .assert()
        .success()
        .stderr(predicate::str::contains("Wrote"));

    let lock_after = fs::read_to_string(&lock_path).unwrap();
    assert!(lock_after.contains("[discovery]"));
    assert!(lock_after.contains("[[field]]"));
    // Timestamp should differ
    assert_ne!(lock_before, lock_after);
}

#[test]
fn update_reads_glob_from_config() {
    let tmp = TempDir::new().unwrap();

    // Create files in different directories
    fs::create_dir(tmp.path().join("blog")).unwrap();
    write_md(
        tmp.path().join("blog").as_path(),
        "post.md",
        "title: Post\n",
        "Body",
    );
    fs::create_dir(tmp.path().join("notes")).unwrap();
    write_md(
        tmp.path().join("notes").as_path(),
        "idea.md",
        "title: Idea\n",
        "Body",
    );

    // Config with directory-scoped glob — only blog/
    write_schema(
        tmp.path(),
        "mfv.toml",
        r#"
[directory]
glob = "blog/*"

[[fields.field]]
name = "title"
type = "string"
"#,
    );

    // Update should only find blog files
    mfv()
        .args(["update", "--dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("1 markdown files considered"));

    let lock_path = tmp.path().join("mfv.lock");
    let lock_content = fs::read_to_string(&lock_path).unwrap();
    assert!(lock_content.contains("total_files = 1"));
}

#[test]
fn update_missing_config_exit_2() {
    let tmp = TempDir::new().unwrap();
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    mfv()
        .args(["update", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no config found"));
}

#[test]
fn update_missing_dir_exit_2() {
    mfv()
        .args(["update", "--dir", "/nonexistent/path"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is not a directory"));
}

#[test]
fn update_auto_discovers_mdvs_toml() {
    let tmp = TempDir::new().unwrap();
    write_md(tmp.path(), "note.md", "title: Hello\n", "Body");

    // Only mdvs.toml, no mfv.toml
    write_schema(
        tmp.path(),
        "mdvs.toml",
        r#"
[[fields.field]]
name = "title"
type = "string"
"#,
    );

    mfv()
        .args(["update", "--dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .stderr(predicate::str::contains("Wrote"));

    // Lock should be written next to mdvs.toml
    let lock_path = tmp.path().join("mdvs.lock");
    assert!(lock_path.exists());
}
