//! `mdvs export-jsonschema` — emit the canonical JSON Schema of the local
//! `mdvs.toml` as JSON or TOML.
//!
//! Round-trip target: the output of this command, fed back through
//! `mdvs init --from-jsonschema`, reproduces the original `mdvs.toml`'s
//! `[[fields.field]]` and `[fields].ignore` entries (preserving constraints,
//! path-scoping, and `preprocess` arrays via `x-mdvs`).

use crate::outcome::Outcome;
use crate::outcome::commands::export_jsonschema::{ExportFormat, ExportJsonschemaOutcome};
use crate::schema::config::MdvsToml;
use crate::schema::json_schema::dsl_to_canonical;
use crate::step::{CommandResult, ErrorKind, StepEntry};
use std::fs;
use std::path::Path;
use std::time::Instant;
use tracing::{info, instrument};

/// Read `mdvs.toml`, translate to canonical JSON Schema, write to stdout or
/// a file in the requested format.
#[instrument(name = "export-jsonschema", skip_all)]
pub fn run(path: &Path, format: ExportFormat, output_file: Option<&Path>) -> CommandResult {
    let start = Instant::now();
    let mut steps: Vec<StepEntry> = Vec::new();

    let config_path = path.join("mdvs.toml");
    let toml = match MdvsToml::read(&config_path) {
        Ok(t) => t,
        Err(e) => {
            steps.push(StepEntry::err(ErrorKind::User, e.to_string(), 0));
            return CommandResult::failed_from_steps(steps, start);
        }
    };
    if let Err(e) = toml.validate() {
        steps.push(StepEntry::err(
            ErrorKind::User,
            format!("mdvs.toml is invalid: {e}"),
            0,
        ));
        return CommandResult::failed_from_steps(steps, start);
    }

    let fields_exported = toml.fields.field.len();
    let ignore_exported = toml.fields.ignore.len();

    let canonical = dsl_to_canonical(&toml);
    let rendered = match format {
        ExportFormat::Json => match serde_json::to_string_pretty(&canonical) {
            Ok(s) => s,
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::Application,
                    format!("failed to serialize as JSON: {e}"),
                    0,
                ));
                return CommandResult::failed_from_steps(steps, start);
            }
        },
        ExportFormat::Toml => match tomljson::to_string(&canonical) {
            Ok(s) => s,
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::Application,
                    format!("failed to serialize as TOML: {e}"),
                    0,
                ));
                return CommandResult::failed_from_steps(steps, start);
            }
        },
    };

    match output_file {
        Some(out) => {
            if let Err(e) = fs::write(out, &rendered) {
                steps.push(StepEntry::err(
                    ErrorKind::Application,
                    format!("failed to write '{}': {e}", out.display()),
                    0,
                ));
                return CommandResult::failed_from_steps(steps, start);
            }
            info!(path = %out.display(), "schema written");
        }
        None => {
            // Stdout — emit verbatim, no trailing summary noise.
            println!("{rendered}");
        }
    }

    CommandResult {
        steps,
        result: Ok(Outcome::ExportJsonschema(Box::new(
            ExportJsonschemaOutcome {
                source: config_path,
                destination: output_file.map(|p| p.to_path_buf()),
                format,
                fields_exported,
                ignore_exported,
            },
        ))),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::Outcome;
    use crate::schema::json_schema::canonical_to_dsl;
    use crate::schema::load::load_schema;
    use std::fs;

    fn unwrap_outcome(result: &CommandResult) -> &ExportJsonschemaOutcome {
        match &result.result {
            Ok(Outcome::ExportJsonschema(o)) => o,
            other => panic!("expected Ok(ExportJsonschema), got: {other:?}"),
        }
    }

    fn init_sample_toml(dir: &Path) {
        // Build a representative mdvs.toml exercising every reverse-translatable
        // shape: constraints, nullable, preprocess, path-scoping, ignore.
        let toml_content = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]
ignore = ["internal_id"]

[[fields.field]]
name = "title"
type = "String"
allowed = ["**"]
required = ["**"]
nullable = false

[fields.field.constraints]
min_length = 3
max_length = 100

[[fields.field]]
name = "rating"
type = "Integer"
allowed = ["**"]
required = []
nullable = false

[fields.field.constraints]
min = 0
max = 5

[[fields.field]]
name = "status"
type = "String"
allowed = ["**"]
required = []
nullable = true

[fields.field.constraints]
categories = ["draft", "published"]

[[fields.field]]
name = "funding"
type = "String"
allowed = ["**"]
required = []
nullable = false
preprocess = ["coerce-to-string"]
"#;
        fs::write(dir.join("mdvs.toml"), toml_content).unwrap();
    }

    #[test]
    fn export_to_stdout_json_format() {
        let tmp = tempfile::tempdir().unwrap();
        init_sample_toml(tmp.path());
        let step = run(tmp.path(), ExportFormat::Json, None);
        assert!(!crate::step::has_failed(&step));
        let outcome = unwrap_outcome(&step);
        assert_eq!(outcome.fields_exported, 4);
        assert_eq!(outcome.ignore_exported, 1);
        assert!(outcome.destination.is_none());
    }

    #[test]
    fn export_to_file_json_format() {
        let tmp = tempfile::tempdir().unwrap();
        init_sample_toml(tmp.path());
        let out = tmp.path().join("schema.json");
        let step = run(tmp.path(), ExportFormat::Json, Some(&out));
        assert!(!crate::step::has_failed(&step));
        let content = fs::read_to_string(&out).unwrap();
        assert!(content.starts_with('{'));
        assert!(content.contains("\"title\""));
        assert!(content.contains("\"x-mdvs\""));
        // Should be the JSON Schema 2020-12 draft identifier
        assert!(content.contains("json-schema.org/draft/2020-12"));
    }

    #[test]
    fn export_to_file_toml_format() {
        let tmp = tempfile::tempdir().unwrap();
        init_sample_toml(tmp.path());
        let out = tmp.path().join("schema.toml");
        let step = run(tmp.path(), ExportFormat::Toml, Some(&out));
        assert!(!crate::step::has_failed(&step));
        let content = fs::read_to_string(&out).unwrap();
        // TOML output should be parseable by tomljson back to the original Value.
        let parsed = tomljson::from_str(&content).unwrap();
        let import = canonical_to_dsl(&parsed).expect("round-trip parses");
        // Sanity: same field count.
        assert_eq!(import.fields.len(), 4);
        assert_eq!(import.ignore.len(), 1);
    }

    #[test]
    fn export_then_init_roundtrip_preserves_fields() {
        // The end-to-end round-trip: export → init --from-jsonschema → compare.
        let tmp = tempfile::tempdir().unwrap();
        init_sample_toml(tmp.path());

        let schema_path = tmp.path().join("schema.json");
        let step = run(tmp.path(), ExportFormat::Json, Some(&schema_path));
        assert!(!crate::step::has_failed(&step));

        // Reload schema via load_schema, run canonical_to_dsl, compare to original toml.
        let canonical = load_schema(&schema_path).unwrap();
        let import = canonical_to_dsl(&canonical).expect("reverse translation");

        let original = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let mut original_fields = original.fields.field.clone();
        original_fields.sort_by(|a, b| a.name.cmp(&b.name));
        let mut roundtrip_fields = import.fields;
        roundtrip_fields.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(roundtrip_fields, original_fields);
        assert_eq!(import.ignore, original.fields.ignore);
    }

    #[test]
    fn export_missing_toml_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let step = run(tmp.path(), ExportFormat::Json, None);
        assert!(crate::step::has_failed(&step));
    }
}
