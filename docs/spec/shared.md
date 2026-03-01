# Shared Types

**Status: DRAFT**

Output structs shared across multiple commands. Each command collects its results
into a struct before display — these are the types that appear in more than one command.

---

## CommandOutput trait

Every command's output struct implements this trait to support multiple output formats.
JSON output comes for free via `Serialize`. Human output is implemented per struct.

### Trait definition

```rust
trait CommandOutput: Serialize {
    /// Render this result as human-readable text (tables, summaries).
    fn format_human(&self) -> String;

    /// Print to stdout in the requested format.
    /// Default implementation handles dispatch — commands don't need to override this.
    fn print(&self, format: OutputFormat) {
        match format {
            OutputFormat::Human => print!("{}", self.format_human()),
            OutputFormat::Json => print!("{}", serde_json::to_string_pretty(self).unwrap()),
        }
    }
}
```

### OutputFormat

```rust
#[derive(Clone, clap::ValueEnum)]
enum OutputFormat {
    Human,
    Json,
}
```

| Variant | Flag value | Description                    |
|---------|------------|--------------------------------|
| Human   | `human`    | Readable tables (default)      |
| Json    | `json`     | Structured JSON for piping     |

### Global flag via clap

The `--output` flag is defined once on the root `Cli` struct and propagated to all
subcommands via `#[arg(global = true)]`.

```rust
#[derive(Parser)]
struct Cli {
    /// Output format
    #[arg(short, long, global = true, default_value = "human")]
    output: OutputFormat,

    #[command(subcommand)]
    command: Command,
}
```

### Implementing for a command

Each command's result struct derives `Serialize` and implements `CommandOutput`.
The `format_human` method returns the full human-readable output as a string.

```rust
#[derive(Serialize)]
pub struct CheckResult {
    pub files_checked: usize,
    pub field_violations: Vec<FieldViolation>,
    pub new_fields: Vec<NewField>,
}

impl CommandOutput for CheckResult {
    fn format_human(&self) -> String {
        if self.field_violations.is_empty() {
            return format!("Checked {} files — no violations\n", self.files_checked);
        }
        let mut out = String::new();
        for v in &self.field_violations {
            out.push_str(&format!("{}: {}\n", v.field, v.rule));
            for f in &v.files {
                out.push_str(&format!("  {}\n", f.path.display()));
            }
            out.push('\n');
        }
        out.push_str(&format!(
            "Checked {} files — {} field violations\n",
            self.files_checked, self.field_violations.len()
        ));
        out
    }
}
```

### Usage in main.rs

Every subcommand follows the same pattern: run the command, print the result.

```rust
fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Init { .. } => {
            let result = init::run(..)?;
            result.print(cli.output);
        }
        Command::Check { .. } => {
            let result = check::run(..)?;
            result.print(cli.output);
        }
        Command::Search { .. } => {
            let result = search::run(..)?;
            result.print(cli.output);
        }
        // ... same for all commands
    }
}
```

### Design notes

- **JSON is automatic**: `#[derive(Serialize)]` is all that's needed. No manual JSON formatting.
- **Human is explicit**: each command controls its own human-readable output via `format_human`.
- **Progress to stderr**: progress messages ("Loading model...") go to stderr.
  Only the final result goes through `CommandOutput::print` to stdout.
- **Extensible**: adding a new format (csv, yaml) means adding a variant to `OutputFormat`
  and a method to the trait with a default (e.g. `fn format_csv(&self) -> Option<String> { None }`).
- **No dynamic dispatch needed**: each match arm in main.rs knows the concrete type. No `Box<dyn CommandOutput>`.

---

## DiscoveredField

Represents a single frontmatter field found during scanning.

**Used by:** init, update, info

| Field       | Type   | Description                              |
|-------------|--------|------------------------------------------|
| name        | String | Field name (e.g. "title", "tags")        |
| field_type  | String | Inferred type (e.g. "String", "Boolean") |
| files_found | usize  | Number of files containing this field    |
| total_files | usize  | Total files scanned (for "N/M" display)  |

---

## FieldViolation

A single rule violation for a field, grouped with all offending files.

**Used by:** check, update

| Field  | Type              | Description                                              |
|--------|-------------------|----------------------------------------------------------|
| field  | String            | Field name                                               |
| kind   | ViolationKind     | Type of violation                                        |
| rule   | String            | The toml rule (e.g. `required in ["blog/**"]`)           |
| files  | Vec\<ViolatingFile\> | Files that violate this rule                          |

A single field can appear in multiple `FieldViolation` entries if it violates different rules.

### ViolationKind

| Variant          | Meaning                                                        |
|------------------|----------------------------------------------------------------|
| MissingRequired  | File matches a `required` glob but doesn't have the field      |
| WrongType        | Field value doesn't match declared type (int-in-float lenient) |
| Disallowed       | File has the field but doesn't match any `allowed` glob        |

### ViolatingFile

| Field  | Type            | Description                                          |
|--------|-----------------|------------------------------------------------------|
| path   | PathBuf         | File path                                            |
| detail | Option\<String\> | Extra info (e.g. "got Integer" for WrongType)       |

---

## NewField

A frontmatter field found in files but not present in `mdvs.toml` (neither in `[[fields.field]]` nor in `[fields].ignore`).

**Used by:** check, update

| Field       | Type   | Description                           |
|-------------|--------|---------------------------------------|
| name        | String | Field name                            |
| files_found | usize  | Number of files containing this field |
