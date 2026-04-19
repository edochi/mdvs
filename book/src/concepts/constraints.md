# Constraints

Constraints are validation rules that go beyond type checking. While types ensure a value is a `String` or `Integer`, constraints refine what values are actually valid — for example, restricting a `String` field to a specific set of allowed values.

Constraints are not a new type. They're an optional layer on top of the existing type system. A field without constraints is validated by type alone; a field with constraints gets an additional check.

## Categories

The **categories** constraint restricts a field's values to a declared set. It applies to:

- **String** — the value must be one of the listed strings
- **Integer** — the value must be one of the listed integers
- **Array(String)** — each element must be one of the listed strings
- **Array(Integer)** — each element must be one of the listed integers

Boolean, Float, and Object fields don't support categories — Boolean is already two-valued, Float is continuous, and Object is structural.

### TOML representation

Categories live in a `[fields.field.constraints]` sub-table:

```toml
[[fields.field]]
name = "status"
type = "String"
allowed = ["**"]
required = ["blog/**"]
nullable = false

[fields.field.constraints]
categories = ["active", "archived", "completed", "draft", "published"]
```

Integer categories:

```toml
[[fields.field]]
name = "priority"
type = "Integer"

[fields.field.constraints]
categories = [1, 2, 3]
```

Array categories constrain each element:

```toml
[[fields.field]]
name = "tags"
type = { array = "String" }

[fields.field.constraints]
categories = ["go", "python", "rust"]
```

A field without a `[fields.field.constraints]` section (or without a `categories` key) is unconstrained.

### Validation

When a value doesn't match any of the declared categories, `check` reports an `InvalidCategory` violation. For arrays, the violation lists the specific offending elements. See [Validation](./validation.md#invalidcategory) for details.

Null values on categorical fields follow the existing nullable logic — if `nullable = true`, null skips the category check. The category constraint only fires on non-null values that pass the type check.

## Auto-inference

During `init` and `update reinfer`, mdvs automatically detects categorical fields using a heuristic with two conditions (both must hold):

1. **Max distinct values** — the field has at most `max_categories` distinct values (default: 10)
2. **Minimum repetition** — `total occurrences / distinct values >= min_category_repetition` (default: 3)

For array fields, distinct values and occurrences are counted at the element level.

### Examples

- `status` with 3 distinct values across 30 files: distinct=3, repetition=10 — **categorical**
- `title` with 28 distinct values across 30 files: distinct=28 (exceeds cap) — **not categorical**
- `author` with 5 distinct values across 5 files: repetition=1 (below threshold) — **not categorical**

### Configurable thresholds

The thresholds are configurable in `[fields]`:

```toml
[fields]
max_categories = 10
min_category_repetition = 3
```

These control automatic inference only. Manually written `categories` in the TOML are unaffected by thresholds.

CLI flags on `update reinfer` override the TOML values per-invocation:

```bash
mdvs update example_kb reinfer --max-categories 15 --min-repetition 3
```

## Range

The **range** constraint restricts a numeric field's value to an inclusive `[min, max]` interval. It applies to:

- **Integer** — value must satisfy `min <= value <= max`
- **Float** — same, with float comparison
- **Array(Integer)** — each element must satisfy the range
- **Array(Float)** — same, element-wise

Both `min` and `max` are optional — you can specify just one bound. Boolean, String, and Object fields don't support range.

### TOML representation

```toml
[[fields.field]]
name = "rating"
type = "Integer"

[fields.field.constraints]
min = 1
max = 5
```

Float bounds (with optional integer bound on a Float field — bounds widen to `f64` for comparison):

```toml
[[fields.field]]
name = "score"
type = "Float"

[fields.field.constraints]
min = 0
max = 100
```

Array example — each element checked against the bounds:

```toml
[[fields.field]]
name = "ratings"
type = { array = "Integer" }

[fields.field.constraints]
min = 1
max = 10
```

### Validation

When a value is out of bounds, `check` reports an `OutOfRange` violation with the rule (`min = N, max = N`) and the offending value. For arrays, the violation lists the specific elements that are out of range.

Null values follow the existing nullable logic — if `nullable = true`, null skips the range check.

### Type rules

Bound types must match the field type:

- **Integer fields** require integer bounds. Float bounds (e.g., `min = 0.5`) are rejected at config load — likely a mistake; an integer can never equal `0.5`.
- **Float fields** accept both integer and float bounds (integer bounds widen to `f64`).

If both bounds are present, `min` must be `<= max` — otherwise rejected at config load.

## Manual overrides

Use the `--with` flag on `update reinfer` to override the default heuristic for specific fields:

```bash
# Force categorical (skip heuristic threshold)
mdvs update example_kb reinfer title --with=categorical

# Infer min/max from observed numeric values
mdvs update example_kb reinfer sample_count --with=range

# Strip all constraints
mdvs update example_kb reinfer status --with=none
```

`--with` takes a comma-separated list of constraint kinds: `categorical`, `range`, or `none`. Incompatible kinds (e.g., `range,categorical` on the same field) are rejected at parse time. `--with=none` cannot be combined with other kinds. The flag requires named fields.

**Manual TOML edit** — you can also add or remove constraints by hand. Running `update` (without `reinfer`) preserves existing constraints as-is. Only `update reinfer` re-evaluates them.

## Conflicts between constraint kinds

Some combinations are mutually exclusive on the same field:

- **`categories` + `range`** — redundant: enumerated values already define the range. Rejected at config load.

Compatible combinations may exist for future constraint kinds (e.g., range + length would be orthogonal — range constrains value, length constrains size).

## Future constraint kinds

- **Length** (`min_length` / `max_length`) — length bounds on String and Array fields
- **Pattern** — regex validation on String fields

Each constraint kind is an additional key in the `[fields.field.constraints]` sub-table. Compatibility between constraint kinds is checked at config load time.
