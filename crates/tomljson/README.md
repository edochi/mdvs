# tomljson

Lossless TOML ↔ JSON translation in pure Rust.

## What it does

Encode `serde_json::Value` to TOML and back, handling the impedance gaps where TOML's data model can't natively represent something JSON can:

- **Null** — TOML has no null type. JSON `null` is encoded as a string placeholder (default `"__null__"`, configurable). Decoding substitutes the placeholder back to `null`.
- **Top-level non-table values** — TOML documents must have a table at the root. Non-table JSON values (booleans, scalars, arrays) are wrapped under a configurable root key (default `"__root__"`) on encode and unwrapped on decode.
- **Integer range** — TOML integers are signed 64-bit. Encoding rejects values larger than `i64::MAX`.
- **Float fidelity** — `f64` values round-trip via `f64::to_string` (Ryū-shortest representation). NaN and ±∞ error explicitly on decode (JSON's number model can't represent them).
- **Datetime canonicalization** — all four TOML datetime variants (`Date`, `Time`, `LocalDateTime`, `OffsetDateTime`) decode to JSON strings in canonical RFC 3339 form.

The motivating use case is JSON Schema 2020-12 documents authored as TOML, but `tomljson` is application-agnostic — anyone moving JSON-shaped data through TOML can use it.

## Quick start

```rust
use serde_json::json;

let value = json!({
    "type": "object",
    "properties": {
        "name": { "type": "string" }
    }
});

let toml_str = tomljson::to_string(&value).unwrap();
let back = tomljson::from_str(&toml_str).unwrap();
assert_eq!(back, value);
```

## API surface

| Function | Purpose |
|---|---|
| `to_string(value)` | encode with default options |
| `to_string_with_options(value, &options)` | encode with custom placeholder |
| `from_str(s)` | decode with default options |
| `from_str_with_options(s, &options)` | decode with custom placeholder |

Plus:
- `TomlJsonOptions { null_placeholder: String, root_placeholder: String }` — configure both reserved strings if your data collides with the defaults.
- `DEFAULT_NULL_PLACEHOLDER` (`"__null__"`), `DEFAULT_ROOT_PLACEHOLDER` (`"__root__"`).
- `Error`, `Result`.

## Encoding rules

### JSON → TOML (encode)

| Concern | Handling |
|---|---|
| `Json::Null` value (anywhere) | Substitute the placeholder string (default `"__null__"`) |
| Top-level non-table value (`bool`, scalar, array) | Wrap under the configured `root_placeholder` key (default `"__root__"`) |
| Top-level Object whose keys include `root_placeholder` | **Error** (`RootKeyCollision`) — encoder would emit a wrap-shaped TOML doc indistinguishable from a wrapped non-table; decoder would silently strip the user's data |
| `serde_json::Number` representable as `u64 > i64::MAX` | **Error** (`IntegerOutOfRange`) — TOML's signed 64-bit limit; JSON spec is wider |
| String value equal to the configured `null_placeholder` | **Error** (`PlaceholderCollision`) — the round-trip would be ambiguous |
| Strings shaped like TOML literals (`"42"`, `"true"`, `"2026-05-04"`, `"inf"`) | Always quoted on output; never coerced |
| Other JSON values (string, finite number, bool, array, object) | Direct emission via `toml_writer` primitives |

Internal structure on encode:

- Sub-objects → `[section]` headers.
- Arrays whose elements are all objects → `[[array.of.tables]]` headers.
- Other arrays + scalars → inline.

### TOML → JSON (decode)

| Concern | Handling |
|---|---|
| TOML string equal to the configured placeholder | Decode as `Json::Null` |
| Root-level table containing exactly one key matching `root_placeholder` | Unwrap and return the inner value |
| TOML datetime (any of `Date`, `Time`, `LocalDateTime`, `OffsetDateTime`) | Decode as `Json::String` in canonical RFC 3339 form |
| TOML float `+inf` / `-inf` / `NaN` | **Error** (`FloatNotRepresentable`) |
| TOML integer (`i64`) | Decode as `Json::Number` (i64-backed) |
| TOML finite float (`f64`) | Decode as `Json::Number` (f64-backed) |
| TOML bool, array, table | Recursively decode |
| Absent TOML key | Absent in JSON object — no special handling |
| TOML parse failure | **Error** (`Toml`) |

## Design notes

### Why `+inf` / `-inf` / `NaN` error on decode

`serde_json::Number::from_f64` rejects non-finite floats. JSON itself has no syntax for them. JSON Schema's validation model can't compare against them either — `maximum: inf` is meaningless because *every* finite number satisfies it.

Producers wanting "no upper bound" should **omit** `maximum` rather than write `inf`. Storage layers (Parquet, Arrow, LanceDB) handle infinities natively, but the JSON validation layer cannot, and `tomljson` errors at the boundary so the limitation surfaces early instead of silently corrupting data.

### Why absent and null must be distinguished

TOML's grammar has **no syntax for "key present, no value"**:

```toml
x =          # parse error
```

But JSON treats `{}` and `{"x": null}` as different objects (key present, value null vs. key absent). To preserve this distinction across the round-trip, `tomljson` uses a placeholder string for null:

```toml
x = "__null__"   # x is present, value is JSON null
                 # (vs. omitting the line, which means x is absent)
```

## Limitations

- **No generic `Serialize`/`DeserializeOwned` wrapper.** Callers with typed structs convert at the boundary via `serde_json::to_value` / `serde_json::from_value`.
- **No streaming.** Both encode and decode operate on whole documents. The underlying `toml` crate has no streaming parser, and TOML's key-order freedom would defeat streaming on the decode side anyway.
- **Datetime decode is one-way.** Decoding produces JSON strings (RFC 3339); re-encoding those strings doesn't reconstruct TOML's native datetime form. JSON has no native datetime type.

## License

MIT. See `LICENSE`.
