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

#[test]
fn init_dry_run_prints_table() {
    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--dry-run")
        .assert()
        .success()
        .stderr(predicate::str::contains("Field"))
        .stderr(predicate::str::contains("Type"))
        .stderr(predicate::str::contains("title"))
        .stderr(predicate::str::contains("Scanning"));
}

#[test]
fn init_dry_run_discovers_fields() {
    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--dry-run")
        .assert()
        .success()
        .stderr(predicate::str::contains("tags"))
        .stderr(predicate::str::contains("date"))
        .stderr(predicate::str::contains("draft"));
}

#[test]
fn init_writes_config_and_lock() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("mfv.toml");

    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--config")
        .arg(&config_path)
        .assert()
        .success()
        .stderr(predicate::str::contains("Wrote"));

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("[[fields.field]]"));
    // Config should have allowed/required patterns
    assert!(content.contains("allowed = "));

    let lock_path = tmp.path().join("mfv.lock");
    assert!(lock_path.exists(), "lock file should be created");
    let lock_content = std::fs::read_to_string(&lock_path).unwrap();
    assert!(lock_content.contains("[discovery]"));
    assert!(lock_content.contains("[[field]]"));
    // Lock should have per-file observation lists
    assert!(lock_content.contains("files = ["));
}

#[test]
fn init_config_parseable() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("out.toml");

    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--config")
        .arg(&config_path)
        .assert()
        .success();

    let content = std::fs::read_to_string(&config_path).unwrap();
    let schema: Result<mdvs_schema::Schema, _> = content.parse();
    assert!(schema.is_ok(), "written file should parse as valid Schema");
}

#[test]
fn init_custom_glob() {
    // Non-recursive glob should exclude nested files
    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .args(["--glob", "*"])
        .arg("--dry-run")
        .assert()
        .success()
        .stderr(predicate::str::contains("markdown files considered"))
        // Should not include nested/deep-note.md
        .stderr(predicate::function(|s: &str| {
            // Parse the count from "N markdown files considered"
            for line in s.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_suffix(" markdown files considered") {
                    if let Ok(count) = rest.parse::<usize>() {
                        // With * (non-recursive), should be fewer than all files
                        return count > 0;
                    }
                }
            }
            false
        }));
}

#[test]
fn init_missing_dir_exit_2() {
    mfv()
        .args(["init", "--dir", "/nonexistent/path"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is not a directory"));
}

#[test]
fn init_empty_dir_exit_2() {
    let tmp = TempDir::new().unwrap();

    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(tmp.path().join("mfv.toml"))
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no markdown files found"));
}

#[test]
fn init_refuses_existing_config() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("mfv.toml");
    std::fs::write(&config_path, "# existing").unwrap();

    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--config")
        .arg(&config_path)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("already exists"))
        .stderr(predicate::str::contains("--force"));
}

#[test]
fn init_force_overwrites() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("mfv.toml");
    std::fs::write(&config_path, "# old content").unwrap();

    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--config")
        .arg(&config_path)
        .arg("--force")
        .assert()
        .success()
        .stderr(predicate::str::contains("Wrote"));

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("[[fields.field]]"));
}

#[test]
fn init_dry_run_no_files_written() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("mfv.toml");

    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--config")
        .arg(&config_path)
        .arg("--dry-run")
        .assert()
        .success();

    assert!(
        !config_path.exists(),
        "config should not be written in dry-run mode"
    );
    let lock_path = tmp.path().join("mfv.lock");
    assert!(
        !lock_path.exists(),
        "lock should not be written in dry-run mode"
    );
}

#[test]
fn init_minimal_omits_unconstrained() {
    let tmp = TempDir::new().unwrap();

    // "tags" and "title" are spread across directories but not in all files,
    // so inference gives allowed=["**"], required=[] → unconstrained.
    // "doi" is only in papers/ → scoped allowed=["papers/**"], constrained.
    let blog = tmp.path().join("blog");
    let papers = tmp.path().join("papers");
    std::fs::create_dir(&blog).unwrap();
    std::fs::create_dir(&papers).unwrap();

    std::fs::write(blog.join("post1.md"), "---\ntags: [a]\n---\nBody").unwrap();
    std::fs::write(blog.join("post2.md"), "---\ntitle: Hello\n---\nBody").unwrap();
    std::fs::write(papers.join("paper.md"), "---\ntags: [b]\ndoi: 10.1234/test\n---\nBody").unwrap();
    std::fs::write(papers.join("paper2.md"), "---\ntitle: X\n---\nBody").unwrap();

    let config_path = tmp.path().join("mfv.toml");

    // Without --minimal: all fields should appear
    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(&config_path)
        .assert()
        .success();

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("name = \"tags\""), "full mode should include tags");
    assert!(content.contains("name = \"doi\""), "full mode should include doi");

    // With --minimal: only constrained fields should appear
    mfv()
        .args(["init", "--dir"])
        .arg(tmp.path())
        .arg("--config")
        .arg(&config_path)
        .arg("--force")
        .arg("--minimal")
        .assert()
        .success();

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(!content.contains("name = \"tags\""), "minimal should omit unconstrained tags");
    assert!(!content.contains("name = \"title\""), "minimal should omit unconstrained title");
    assert!(content.contains("name = \"doi\""), "minimal should keep scoped doi");
}

#[test]
fn init_lock_contains_all_fields() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("mfv.toml");

    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .arg("--config")
        .arg(&config_path)
        .assert()
        .success();

    let lock_path = tmp.path().join("mfv.lock");
    let lock_content = std::fs::read_to_string(&lock_path).unwrap();

    // Lock should have discovery metadata
    assert!(lock_content.contains("total_files"));
    assert!(lock_content.contains("files_with_frontmatter"));
    assert!(lock_content.contains("generated_at"));

    // Lock should contain title (present in most fixtures)
    assert!(lock_content.contains("\"title\""));
    // Lock should have per-file observations
    assert!(lock_content.contains("files = ["));
}
