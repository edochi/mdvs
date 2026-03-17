# Validation

`mdvs check` validates every file's frontmatter against the schema in `mdvs.toml`. It's read-only, deterministic, and produces no side effects — it just tells you what's wrong.

## The four violations

| Violation | Meaning |
|---|---|
| `WrongType` | The value doesn't match the declared `type` |
| `Disallowed` | The field appears in a file outside its `allowed` paths |
| `MissingRequired` | A file matches a `required` glob but doesn't have the field |
| `NullNotAllowed` | The field is present but `null`, and `nullable` is `false` |

### WrongType

Fires when a value doesn't match the declared type. If `convergence_ms` is declared as `Boolean` but a file has `convergence_ms: 42`, the integer value fails the boolean check.

This violation has two important leniencies — see [Type checking rules](#type-checking-rules) below.

### Disallowed

Fires when a field appears in a file whose path doesn't match any of the field's `allowed` globs. For example, if `firmware_version` has `allowed = ["people/interns/**"]` but appears in `people/remo.md`, that file is outside the allowed paths.

### MissingRequired

Fires when a file's path matches one of the field's `required` globs, but the file doesn't contain that field at all.

For example, if `observation_notes` has `required = ["projects/alpha/notes/**"]`, then every file under `projects/alpha/notes/` must have it. Files that don't → `MissingRequired`.

### NullNotAllowed

Fires when a field is present with an explicit `null` value, but `nullable` is `false`. For example, if `drift_rate` has `nullable = false` and a file has `drift_rate: null`.

This is distinct from a missing field — see [Null vs absent](#null-vs-absent) below.

## Type checking rules

Two leniencies make validation practical for real-world YAML:

**String accepts any value.** Since String is the top type (see [Types & Widening](./types.md#string-is-the-top-type)), a String-typed field never triggers a `WrongType` violation. Booleans, integers, arrays — everything is accepted. This is by design: when types are widened to String during inference, the field should accept whatever values caused the widening.

**Float accepts integers.** An integer value like `5` passes validation for a Float field. YAML doesn't distinguish `5` from `5.0`, and many editors strip trailing `.0`. Rejecting integers from Float fields would cause constant false positives.

Arrays check element types recursively — an `Integer[]` field rejects `["a", "b"]` because the string elements fail the Integer check.

Objects just check that the value is an object — individual keys are not validated against the inferred structure.

## Null handling

Null interacts with validation in specific ways:

**All four checks are independent.** A null value is checked like any other value — each violation type is evaluated separately:

- **`WrongType`** — null is accepted by any type, so this never fires on null.
- **`Disallowed`** — the field is present (the key exists), so `Disallowed` fires if the path isn't in `allowed`.
- **`MissingRequired`** — null counts as "present", so this never fires on null.
- **`NullNotAllowed`** — fires when the value is null and `nullable = false`.

A single null field can trigger both `Disallowed` and `NullNotAllowed` at the same time.

**Null vs absent.** These are different situations with different outcomes:

| Situation | Example | Result |
|---|---|---|
| Field is **absent** | File has no `drift_rate` key at all | `MissingRequired` (if path matches `required`) |
| Field is **null**, `nullable = true` | `drift_rate: null` | Passes |
| Field is **null**, `nullable = false` | `drift_rate: null` | `NullNotAllowed` |

A null value counts as "present" — the field key exists in the frontmatter, it just has no value. So null never triggers `MissingRequired`. An absent field is genuinely missing — it can trigger `MissingRequired` but never `NullNotAllowed`.

> **Note:** In YAML, unquoted `null` is a null value, not the string `"null"`. To store the literal string, write `drift_rate: "null"` (with quotes).

## New fields

When `mdvs check` encounters a frontmatter field that isn't in `mdvs.toml` — neither constrained under `[[fields.field]]` nor listed in `ignore` — it reports it as a **new field**.

New fields are informational only. They don't count as violations and don't affect the exit code:

```
Checked 43 files — no violations, 1 new field(s)

╭──────────────────────────────┬─────────────────────┬─────────────────────────╮
│ "algorithm"                  │ new                 │ 2 files                 │
╰──────────────────────────────┴─────────────────────┴─────────────────────────╯
```

They're shown in the output so you know to either run `mdvs update` to add them to the schema, or add them to the `ignore` list.

## Bare files

When `include_bare_files = true` in `[scan]`, bare files (no frontmatter at all) are included in validation. Since they have no fields, they trigger `MissingRequired` for any `required` glob matching their path.

For example, if `title` has `required = ["**"]` and `scratch.md` is a bare file, it triggers `MissingRequired` for `title`. This is often why the inferred schema uses narrower required globs — bare files at the root prevent `required = ["**"]` from being inferred for fields that don't appear in them.

## Check and build

`mdvs build` runs the same validation internally before embedding. If any violations are found, build aborts — no dirty data reaches the index. The violations are the same ones `check` would report.

This means you can use `check` as a dry run before building, but you don't have to — build will catch the same problems.

## Exit codes

| Exit code | Meaning |
|---|---|
| 0 | No violations (new fields don't count) |
| 1 | One or more violations found |
| 2 | Scan or config error (couldn't run validation) |
