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

## Manual overrides

Three mechanisms let you control categories independently of the heuristic:

**Force categorical** — use `--categorical` to skip the heuristic and collect all distinct values as categories:

```bash
mdvs update example_kb reinfer title --categorical
```

**Force NOT categorical** — use `--no-categorical` to strip categories even if the heuristic would add them:

```bash
mdvs update example_kb reinfer status --no-categorical
```

Both flags require named fields — they can't be used with `update reinfer` (all fields).

**Manual TOML edit** — add or remove `categories` by hand. Running `update` (without `reinfer`) preserves existing categories as-is. Only `update reinfer` re-runs the heuristic on targeted fields.

## Future constraint kinds

The constraint system is designed to be extensible. Future constraint kinds planned:

- **Range** (`min` / `max`) — numeric bounds on Integer and Float fields
- **Length** (`min_length` / `max_length`) — length bounds on String and Array fields
- **Pattern** — regex validation on String fields

Each constraint kind will be an additional key in the `[fields.field.constraints]` sub-table. Compatibility between constraint kinds is checked at config load time — some combinations may conflict (e.g., categories and range on the same Integer field).
