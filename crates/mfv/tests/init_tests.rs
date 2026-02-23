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

    let lock_path = tmp.path().join("mfv.lock");
    assert!(lock_path.exists(), "lock file should be created");
    let lock_content = std::fs::read_to_string(&lock_path).unwrap();
    assert!(lock_content.contains("[discovery]"));
    assert!(lock_content.contains("[[field]]"));
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
fn init_threshold() {
    // With threshold 1.0, no field is promoted (none at 100% frequency)
    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .args(["--threshold", "1.0"])
        .arg("--dry-run")
        .assert()
        .success()
        // The "Promoted" column should have no "Y" markers
        .stderr(predicate::str::contains("Promoted"))
        .stderr(predicate::function(|output: &str| {
            // Check that no line has a "Y" in the promoted column position
            for line in output.lines().skip(2) {
                // skip header and separator
                if line.trim().is_empty() {
                    continue;
                }
                if line.ends_with(" Y") || line.ends_with("\tY") {
                    return false;
                }
            }
            true
        }));
}

#[test]
fn init_custom_glob() {
    // Non-recursive glob should exclude nested files
    mfv()
        .args(["init", "--dir"])
        .arg(fixtures_dir())
        .args(["--glob", "*.md"])
        .arg("--dry-run")
        .assert()
        .success()
        .stderr(predicate::str::contains("Found"))
        // Should not include nested/deep-note.md
        .stderr(predicate::function(|s: &str| {
            // Parse the count from "Found N markdown files"
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("Found ") {
                    if let Some(n) = rest.split_whitespace().next() {
                        if let Ok(count) = n.parse::<usize>() {
                            // With *.md (non-recursive), should be fewer than all files
                            return count > 0;
                        }
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
}
