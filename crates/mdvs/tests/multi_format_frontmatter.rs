//! Integration tests for multi-format frontmatter (TODO-0162).
//!
//! Exercises `init` + `check` through the public command surface against
//! the TOML / JSON / mixed fixture vaults under `tests/fixtures/`. The
//! per-engine dispatch and `detect_engine` itself have lower-level
//! coverage in `crates/mdvs/src/discover/scan.rs::tests` — these tests
//! prove the command-level pipeline (scan → infer → write toml → reload
//! → validate) produces sensible results across formats.

use mdvs::cmd::{check, init};
use mdvs::outcome::Outcome;
use mdvs::schema::config::MdvsToml;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Copy a fixture vault into a fresh tempdir so tests can mutate state
/// (running `init` writes `mdvs.toml`) without dirtying the committed
/// fixture files.
fn copy_fixture(name: &str) -> TempDir {
    let src: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    let dest = tempfile::tempdir().expect("create tempdir");
    copy_dir(&src, dest.path());
    dest
}

fn copy_dir(src: &Path, dest: &Path) {
    fs::create_dir_all(dest).expect("mkdir dest");
    for entry in fs::read_dir(src).expect("read src dir") {
        let entry = entry.expect("read entry");
        let path = entry.path();
        let target = dest.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).expect("copy file");
        }
    }
}

/// Run `mdvs init` against `path` with sensible defaults, return the
/// reloaded `MdvsToml`.
fn run_init(path: &Path) -> MdvsToml {
    let result = init::run(
        path, "**",  // glob
        false, // force
        false, // dry_run
        false, // ignore_bare_files
        true,  // skip_gitignore
        false, // verbose
        None,  // schema override
    );
    assert!(
        result.result.is_ok(),
        "init failed: {:?}",
        result.result.err()
    );
    MdvsToml::read(&path.join("mdvs.toml")).expect("reload mdvs.toml after init")
}

/// Look up a field by dotted name in the freshly inferred toml. Panics
/// with a clear message if absent.
fn find_field<'a>(toml: &'a MdvsToml, name: &str) -> &'a mdvs::schema::config::TomlField {
    toml.fields
        .field
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| {
            let names: Vec<&str> = toml.fields.field.iter().map(|f| f.name.as_str()).collect();
            panic!("field '{name}' not found. fields present: {names:?}")
        })
}

// ============================================================================
// init — per-vault schema inference
// ============================================================================

/// Assert that a field's inferred type renders to the expected
/// function-style string (`Date`, `DateTime`, `Array(String)`, etc.).
fn assert_field_type(toml: &MdvsToml, field_name: &str, expected: &str) {
    let f = find_field(toml, field_name);
    let actual = f.field_type.to_string();
    assert_eq!(
        actual, expected,
        "field '{field_name}' type mismatch: expected {expected}, got {actual}"
    );
}

#[test]
fn init_infers_toml_vault() {
    let dir = copy_fixture("frontmatter-toml");
    let toml = run_init(dir.path());

    // Native TOML Date / DateTime literals promote to typed fields.
    assert_field_type(&toml, "released_on", "Date");
    assert_field_type(&toml, "built_at", "DateTime");

    // Basic scalars + array preserved.
    assert_field_type(&toml, "title", "String");
    assert_field_type(&toml, "year", "Integer");
    assert_field_type(&toml, "finished", "Boolean");
    assert_field_type(&toml, "tags", "Array(String)");

    // Nested `[calibration.baseline]` flattens to dotted-name leaves.
    assert_field_type(&toml, "calibration.baseline.wavelength_nm", "Float");
    assert_field_type(&toml, "calibration.baseline.intensity", "Float");
    assert_field_type(&toml, "calibration.operator", "String");
}

#[test]
fn init_infers_json_vault() {
    let dir = copy_fixture("frontmatter-json");
    let toml = run_init(dir.path());

    // RFC 3339 strings in JSON promote to typed Date / DateTime just
    // like YAML string dates do.
    assert_field_type(&toml, "released_on", "Date");
    assert_field_type(&toml, "built_at", "DateTime");

    // Nested JSON objects flatten to dotted-name leaves identically.
    assert_field_type(&toml, "calibration.baseline.wavelength_nm", "Float");
    assert_field_type(&toml, "calibration.operator", "String");
}

#[test]
fn init_infers_mixed_vault() {
    let dir = copy_fixture("frontmatter-mixed");
    let toml = run_init(dir.path());

    // All three formats produce identical JSON shapes downstream, so
    // inference unifies them into a single 4-field schema where each
    // field is present in all 3 files.
    let names: Vec<&str> = toml.fields.field.iter().map(|f| f.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["author", "tags", "title", "year"]);

    // Each field present in 3/3 files (so `required = ["**"]` glob-style).
    for f in &toml.fields.field {
        assert_eq!(
            f.required,
            vec!["**".to_string()],
            "field '{}' should be required across all files",
            f.name
        );
    }
}

// ============================================================================
// check — zero violations on each well-formed vault
// ============================================================================

fn check_violations(path: &Path) -> Vec<mdvs::output::FieldViolation> {
    let result = check::run(path, false, false, None);
    match result.result {
        Ok(Outcome::Check(outcome)) => outcome.violations.clone(),
        Ok(other) => panic!("expected Check outcome, got {other:?}"),
        Err(e) => panic!("check failed: {e:?}"),
    }
}

#[test]
fn check_passes_on_toml_vault() {
    let dir = copy_fixture("frontmatter-toml");
    run_init(dir.path());
    let violations = check_violations(dir.path());
    assert!(
        violations.is_empty(),
        "expected zero violations, got: {violations:?}"
    );
}

#[test]
fn check_passes_on_json_vault() {
    let dir = copy_fixture("frontmatter-json");
    run_init(dir.path());
    let violations = check_violations(dir.path());
    assert!(
        violations.is_empty(),
        "expected zero violations, got: {violations:?}"
    );
}

#[test]
fn check_passes_on_mixed_vault() {
    let dir = copy_fixture("frontmatter-mixed");
    run_init(dir.path());
    let violations = check_violations(dir.path());
    assert!(
        violations.is_empty(),
        "expected zero violations, got: {violations:?}"
    );
}

// ============================================================================
// Malformed frontmatter surfaces FrontmatterUnrepresentable
// ============================================================================

#[test]
fn malformed_toml_surfaces_violation() {
    let dir = tempfile::tempdir().unwrap();
    // Valid `+++` delimiters but broken TOML inside (unterminated string).
    fs::write(
        dir.path().join("broken.md"),
        "+++\ntitle = \"unterminated\n+++\n\nBody.\n",
    )
    .unwrap();
    // Init a working schema against a good file first so check has a
    // toml to validate against.
    fs::write(
        dir.path().join("good.md"),
        "+++\ntitle = \"ok\"\n+++\n\nBody.\n",
    )
    .unwrap();
    run_init(dir.path());

    let violations = check_violations(dir.path());
    assert!(
        violations.iter().any(|v| matches!(
            v.kind,
            mdvs::output::ViolationKind::FrontmatterUnrepresentable
        )),
        "expected FrontmatterUnrepresentable on broken.md, got: {violations:?}"
    );
}

#[test]
fn malformed_json_surfaces_violation() {
    let dir = tempfile::tempdir().unwrap();
    // Looks like JSON (starts with `{`) but the value is unquoted garbage.
    fs::write(
        dir.path().join("broken.md"),
        "{\n  \"title\": not-valid-json\n}\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("good.md"),
        "{\n  \"title\": \"ok\"\n}\n\nBody.\n",
    )
    .unwrap();
    run_init(dir.path());

    let violations = check_violations(dir.path());
    assert!(
        violations.iter().any(|v| matches!(
            v.kind,
            mdvs::output::ViolationKind::FrontmatterUnrepresentable
        )),
        "expected FrontmatterUnrepresentable on broken.md, got: {violations:?}"
    );
}
