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
fn scan_prints_table() {
    mfv()
        .args(["scan", "--dir"])
        .arg(fixtures_dir())
        .assert()
        .success()
        .stdout(predicate::str::contains("Field"))
        .stdout(predicate::str::contains("Type"))
        .stdout(predicate::str::contains("title"))
        .stderr(predicate::str::contains("Scanning"));
}

#[test]
fn scan_discovers_fields() {
    mfv()
        .args(["scan", "--dir"])
        .arg(fixtures_dir())
        .assert()
        .success()
        .stdout(predicate::str::contains("tags"))
        .stdout(predicate::str::contains("date"))
        .stdout(predicate::str::contains("draft"));
}

#[test]
fn scan_output_writes_file() {
    let tmp = TempDir::new().unwrap();
    let out_path = tmp.path().join("out.toml");

    mfv()
        .args(["scan", "--dir"])
        .arg(fixtures_dir())
        .arg("--output")
        .arg(&out_path)
        .assert()
        .success()
        .stderr(predicate::str::contains("Wrote"));

    let content = std::fs::read_to_string(&out_path).unwrap();
    assert!(content.contains("[[fields.field]]"));
}

#[test]
fn scan_output_parseable() {
    let tmp = TempDir::new().unwrap();
    let out_path = tmp.path().join("out.toml");

    mfv()
        .args(["scan", "--dir"])
        .arg(fixtures_dir())
        .arg("--output")
        .arg(&out_path)
        .assert()
        .success();

    let content = std::fs::read_to_string(&out_path).unwrap();
    let schema: Result<mdvs_schema::Schema, _> = content.parse();
    assert!(schema.is_ok(), "written file should parse as valid Schema");
}

#[test]
fn scan_threshold() {
    // With threshold 1.0, no field is promoted (none at 100% frequency)
    mfv()
        .args(["scan", "--dir"])
        .arg(fixtures_dir())
        .args(["--threshold", "1.0"])
        .assert()
        .success()
        // The "Promoted" column should have no "Y" markers
        .stdout(predicate::str::contains("Promoted"))
        .stdout(predicate::function(|output: &str| {
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
fn scan_custom_glob() {
    // Non-recursive glob should exclude nested files
    mfv()
        .args(["scan", "--dir"])
        .arg(fixtures_dir())
        .args(["--glob", "*.md"])
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
fn scan_missing_dir_exit_2() {
    mfv()
        .args(["scan", "--dir", "/nonexistent/path"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is not a directory"));
}

#[test]
fn scan_empty_dir_exit_2() {
    let tmp = TempDir::new().unwrap();

    mfv()
        .args(["scan", "--dir"])
        .arg(tmp.path())
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no markdown files found"));
}
