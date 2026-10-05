//! Integration tests pinning the process exit codes and stdout contract of
//! the real binary.
//!
//! Scripts and agent hooks branch on the exit code: 0 means clean, 1 means
//! the command ran and found violations, 2 means the command itself failed.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// Exit code for a clean run.
const EXIT_OK: i32 = 0;

/// Exit code for a run that found validation violations.
const EXIT_VIOLATIONS: i32 = 1;

/// Exit code for a command that failed to run.
const EXIT_FAILURE: i32 = 2;

fn mdvs(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mdvs"))
        .args(args)
        .arg(dir)
        .output()
        .expect("run mdvs")
}

fn exit_code(output: &Output) -> i32 {
    output.status.code().expect("mdvs exited without a code")
}

fn describe(output: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Writes three notes sharing `title` (String) and `count` (Integer), then
/// runs `mdvs init` so the directory holds a matching `mdvs.toml`.
fn init_vault(dir: &Path) {
    for i in 1..=3 {
        fs::write(
            dir.join(format!("n{i}.md")),
            format!("---\ntitle: note {i}\ncount: {i}\n---\nBody.\n"),
        )
        .expect("write note");
    }
    let out = mdvs(&["init"], dir);
    assert_eq!(exit_code(&out), EXIT_OK, "{}", describe(&out));
}

#[test]
fn check_on_clean_vault_exits_zero() {
    let tmp = tempfile::tempdir().unwrap();
    init_vault(tmp.path());
    let out = mdvs(&["check"], tmp.path());
    assert_eq!(exit_code(&out), EXIT_OK, "{}", describe(&out));
}

#[test]
fn check_with_violation_exits_one() {
    let tmp = tempfile::tempdir().unwrap();
    init_vault(tmp.path());
    fs::write(
        tmp.path().join("bad.md"),
        "---\ntitle: bad\ncount: not a number\n---\nBody.\n",
    )
    .unwrap();
    let out = mdvs(&["check", "--no-update"], tmp.path());
    assert_eq!(exit_code(&out), EXIT_VIOLATIONS, "{}", describe(&out));
}

#[test]
fn check_with_invalid_config_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("mdvs.toml"), "not valid toml [[[").unwrap();
    let out = mdvs(&["check"], tmp.path());
    assert_eq!(exit_code(&out), EXIT_FAILURE, "{}", describe(&out));
}

#[test]
fn check_on_missing_directory_exits_two() {
    let tmp = tempfile::tempdir().unwrap();
    let out = mdvs(&["check"], &tmp.path().join("missing"));
    assert_eq!(exit_code(&out), EXIT_FAILURE, "{}", describe(&out));
}

#[test]
fn export_jsonschema_to_stdout_prints_only_the_schema() {
    let tmp = tempfile::tempdir().unwrap();
    init_vault(tmp.path());
    let out = mdvs(&["export-jsonschema"], tmp.path());
    assert_eq!(exit_code(&out), EXIT_OK, "{}", describe(&out));
    let schema: serde_json::Value =
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}\n{}", describe(&out)));
    assert!(
        schema["properties"]["count"].is_object(),
        "{}",
        describe(&out)
    );
}
