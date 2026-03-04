# Output Format Reference

Every command produces output in two formats: **compact** (default) and **verbose** (`-v`).

**Compact** = one-liner summary + table (no internal horizontal lines)
**Verbose** = one-liner summary + record tables (multi-column header row + spanning detail row) + footer one-liner

Verbose is an expansion of compact: same information, plus indented details under each row and a metadata footer.

All user-defined values (field names, file paths, glob patterns) are quoted with `"`.

## Rendering

- Tables rendered with `tabled` crate (`Style::rounded()`)
- `terminal_size` crate for automatic width detection — tables stretch to fill terminal
- Compact tables: `remove_horizontals()` — no internal separators
- Verbose record tables: `ColumnSpan` for detail row + `BorderCorrection` for clean borders
- Content wrapping/truncation via `Width::wrap()` + `Width::increase()`

---

## init

### compact

```
Initialized 498 files — 10 fields

╭──────────────────────────┬─────────────────────────┬─────────────────────────╮
│ "author"                 │ String                  │ 45/498                  │
│ "tags"                   │ Array                   │ 120/498                 │
│ "draft"                  │ Boolean                 │ 30/498                  │
│ "status"                 │ String                  │ 80/498                  │
╰──────────────────────────┴─────────────────────────┴─────────────────────────╯
```

Dry run variant uses `(dry run)` in the one-liner:

```
Initialized 498 files — 10 fields (dry run)
```

### verbose

```
Initialized 498 files — 10 fields

╭───────────────────────────┬─────────────────────────┬────────────────────────╮
│ "author"                  │ String                  │ 45/498                 │
├───────────────────────────┴─────────────────────────┴────────────────────────┤
│   required:                                                                  │
│     - "articles/**"                                                          │
│   allowed:                                                                   │
│     - "articles/**"                                                          │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬────────────────────────┬──────────────────────────╮
│ "tags"                   │ Array                  │ 120/498                  │
├──────────────────────────┴────────────────────────┴──────────────────────────┤
│   required:                                                                  │
│     - "articles/**"                                                          │
│     - "books/**"                                                             │
│     - "questions/**"                                                         │
│   allowed: "**"                                                              │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬──────────────────────────┬────────────────────────╮
│ "draft"                  │ Boolean                  │ 30/498                 │
├──────────────────────────┴──────────────────────────┴────────────────────────┤
│   allowed: "**"                                                              │
╰──────────────────────────────────────────────────────────────────────────────╯

498 files | glob: "**" | 1.2s
```

---

## check (no violations)

### compact

```
Checked 498 files — no violations
```

### verbose

```
Checked 498 files — no violations

498 files | glob: "**" | 200ms
```

---

## check (with violations)

### compact

```
Checked 498 files — 3 violation(s)

╭────────────────────────┬──────────────────────────────┬──────────────────────╮
│ "result"               │ MissingRequired              │ 2 files              │
│ "result"               │ WrongType                    │ 1 file               │
│ "status"               │ Disallowed                   │ 3 files              │
╰────────────────────────┴──────────────────────────────┴──────────────────────╯
```

### verbose

```
Checked 498 files — 3 violation(s)

╭─────────────────────────┬──────────────────────────────┬─────────────────────╮
│ "result"                │ MissingRequired              │ 2 files             │
├─────────────────────────┴──────────────────────────────┴─────────────────────┤
│   - "books/High Performance Browser Networking/ch1.md"                       │
│   - "books/Introduction to Neuromorphic Computing/notes.md"                  │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "result"                 │ WrongType                 │ 1 file                │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   - "articles/foo.md" (got Integer)                                          │
╰──────────────────────────────────────────────────────────────────────────────╯
╭─────────────────────────┬───────────────────────────┬────────────────────────╮
│ "status"                │ Disallowed                │ 3 files                │
├─────────────────────────┴───────────────────────────┴────────────────────────┤
│   - "projects/old/draft.md"                                                  │
│   - "projects/old/notes.md"                                                  │
│   - "projects/old/todo.md"                                                   │
╰──────────────────────────────────────────────────────────────────────────────╯

498 files | glob: "**" | 200ms
```

---

## check (with new fields)

### compact

```
Checked 498 files — no violations, 2 new field(s)

╭────────────────────────┬──────────────────────────────┬──────────────────────╮
│ "category"             │ new                          │ 12 files             │
│ "priority"             │ new                          │ 3 files              │
╰────────────────────────┴──────────────────────────────┴──────────────────────╯
```

### verbose

```
Checked 498 files — no violations, 2 new field(s)

╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "category"               │ new                       │ 12 files              │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   - "articles/rust-guide.md"                                                 │
│   - "articles/python-intro.md"                                               │
│   - "articles/go-basics.md"                                                  │
│   - ...                                                                      │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "priority"               │ new                       │ 3 files               │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   - "projects/website/README.md"                                             │
│   - "projects/cli/README.md"                                                 │
│   - "projects/api/README.md"                                                 │
╰──────────────────────────────────────────────────────────────────────────────╯

498 files | glob: "**" | 200ms
```

---

## check (with violations and new fields)

### compact

Violations and new fields separated by a blank line in the table.

```
Checked 498 files — 3 violation(s), 2 new field(s)

╭────────────────────────┬──────────────────────────────┬──────────────────────╮
│ "result"               │ MissingRequired              │ 2 files              │
│ "result"               │ WrongType                    │ 1 file               │
│ "status"               │ Disallowed                   │ 3 files              │
╰────────────────────────┴──────────────────────────────┴──────────────────────╯

╭────────────────────────┬──────────────────────────────┬──────────────────────╮
│ "category"             │ new                          │ 12 files             │
│ "priority"             │ new                          │ 3 files              │
╰────────────────────────┴──────────────────────────────┴──────────────────────╯
```

### verbose

Violations and new fields as separate groups of record tables.

```
Checked 498 files — 3 violation(s), 2 new field(s)

╭─────────────────────────┬──────────────────────────────┬─────────────────────╮
│ "result"                │ MissingRequired              │ 2 files             │
├─────────────────────────┴──────────────────────────────┴─────────────────────┤
│   - "books/High Performance Browser Networking/ch1.md"                       │
│   - "books/Introduction to Neuromorphic Computing/notes.md"                  │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "result"                 │ WrongType                 │ 1 file                │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   - "articles/foo.md" (got Integer)                                          │
╰──────────────────────────────────────────────────────────────────────────────╯
╭─────────────────────────┬───────────────────────────┬────────────────────────╮
│ "status"                │ Disallowed                │ 3 files                │
├─────────────────────────┴───────────────────────────┴────────────────────────┤
│   - "projects/old/draft.md"                                                  │
│   - "projects/old/notes.md"                                                  │
│   - "projects/old/todo.md"                                                   │
╰──────────────────────────────────────────────────────────────────────────────╯

╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "category"               │ new                       │ 12 files              │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   - "articles/rust-guide.md"                                                 │
│   - "articles/python-intro.md"                                               │
│   - "articles/go-basics.md"                                                  │
│   - ...                                                                      │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "priority"               │ new                       │ 3 files               │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   - "projects/website/README.md"                                             │
│   - "projects/cli/README.md"                                                 │
│   - "projects/api/README.md"                                                 │
╰──────────────────────────────────────────────────────────────────────────────╯

498 files | glob: "**" | 200ms
```

---

## update (no changes)

### compact

```
Scanned 498 files — no changes
```

### verbose

```
Scanned 498 files — no changes

498 files | glob: "**" | 300ms
```

---

## update (with changes)

### compact

```
Scanned 498 files — 3 field(s) changed

╭──────────────────────────┬─────────────────────────┬─────────────────────────╮
│ "author"                 │ added      String        │ "articles/**", "books…" │
│ "tags"                   │ changed    String → Array │ "articles/**"          │
│ "draft"                  │ removed                  │                         │
╰──────────────────────────┴─────────────────────────┴─────────────────────────╯
```

### verbose

```
Scanned 498 files — 3 field(s) changed

╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "author"                 │ added                     │ String                │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   found in:                                                                  │
│     - "articles/**" (45 files)                                               │
│     - "books/**" (12 files)                                                  │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "tags"                   │ changed                   │ String → Array        │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   found in:                                                                  │
│     - "articles/**" (120 files)                                              │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬───────────────────────────┬───────────────────────╮
│ "draft"                  │ removed                   │                       │
├──────────────────────────┴───────────────────────────┴───────────────────────┤
│   previously in:                                                             │
│     - "projects/**"                                                          │
╰──────────────────────────────────────────────────────────────────────────────╯

498 files | glob: "**" | 300ms
```

---

## build (full rebuild)

### compact

```
Built index — 498 files, 2314 chunks (full rebuild)

╭──────────────────────────┬─────────────────────────┬─────────────────────────╮
│ embedded                 │ 498 files               │ 2314 chunks             │
╰──────────────────────────┴─────────────────────────┴─────────────────────────╯
```

### verbose

```
Built index — 498 files, 2314 chunks (full rebuild)

╭──────────────────────────┬─────────────────────────┬─────────────────────────╮
│ embedded                 │ 498 files               │ 2314 chunks             │
├──────────────────────────┴─────────────────────────┴─────────────────────────┤
│   - "articles/rust-guide.md" (12 chunks)                                     │
│   - "articles/python-intro.md" (8 chunks)                                    │
│   - ...                                                                      │
╰──────────────────────────────────────────────────────────────────────────────╯

498 files | model: "minishlab/potion-base-8M" | glob: "**" | 5.2s
```

---

## build (incremental)

### compact

```
Built index — 498 files, 2314 chunks

╭──────────────────────────┬─────────────────────────┬─────────────────────────╮
│ embedded                 │ 120 files               │ 800 chunks              │
│ unchanged                │ 370 files               │ 1514 chunks             │
│ removed                  │ 8 files                 │ 42 chunks               │
╰──────────────────────────┴─────────────────────────┴─────────────────────────╯
```

### verbose

```
Built index — 498 files, 2314 chunks

╭──────────────────────────┬─────────────────────────┬─────────────────────────╮
│ embedded                 │ 120 files               │ 800 chunks              │
├──────────────────────────┴─────────────────────────┴─────────────────────────┤
│   - "articles/new-post.md" (6 chunks)                                        │
│   - "articles/updated-post.md" (4 chunks)                                    │
│   - ...                                                                      │
╰──────────────────────────────────────────────────────────────────────────────╯
╭─────────────────────────┬─────────────────────────┬──────────────────────────╮
│ unchanged               │ 370 files               │ 1514 chunks              │
╰─────────────────────────┴─────────────────────────┴──────────────────────────╯
╭─────────────────────────┬─────────────────────────┬──────────────────────────╮
│ removed                 │ 8 files                 │ 42 chunks                │
├─────────────────────────┴─────────────────────────┴──────────────────────────┤
│   - "archive/old-post.md" (5 chunks)                                         │
│   - "archive/deprecated.md" (3 chunks)                                       │
│   - ...                                                                      │
╰──────────────────────────────────────────────────────────────────────────────╯

498 files | model: "minishlab/potion-base-8M" | glob: "**" | 540ms
```

---

## search

### compact

```
Searched "rust" — 10 hits

╭───────────┬───────────────────────────────────────────────────┬──────────────╮
│ 1         │ "projects/Core Language - Rust.md"                │ 0.656        │
│ 2         │ "technologies/rust/notes/Atomic Types.md"         │ 0.655        │
│ 3         │ "articles/rust-guide.md"                          │ 0.612        │
│ 4         │ "books/Programming Rust/ch1.md"                   │ 0.590        │
│ 5         │ "projects/rustlings/notes.md"                     │ 0.543        │
╰───────────┴───────────────────────────────────────────────────┴──────────────╯
```

### verbose

Verbose shows the best matching chunk per file (the one that determined the file's ranking), with its text read from disk using `start_line`/`end_line`. This is a known limitation — only the top chunk per file is shown.

```
Searched "rust" — 10 hits

╭──────────────┬──────────────────────────────────────────────┬────────────────╮
│ 1            │ "projects/Core Language - Rust.md"           │ 0.656          │
├──────────────┴──────────────────────────────────────────────┴────────────────┤
│   lines 1-4:                                                                 │
│     Rust is a systems programming language focused on safety                 │
│     and performance. It achieves memory safety without garbage               │
│     collection through its ownership system.                                 │
╰──────────────────────────────────────────────────────────────────────────────╯
╭───────────┬───────────────────────────────────────────────────┬──────────────╮
│ 2         │ "technologies/rust/notes/Atomic Types.md"         │ 0.655        │
├───────────┴───────────────────────────────────────────────────┴──────────────┤
│   lines 1-6:                                                                 │
│     Atomic types provide lock-free concurrent access to shared               │
│     data. The std::sync::atomic module exposes AtomicBool,                   │
│     AtomicUsize, and other atomic primitives.                                │
╰──────────────────────────────────────────────────────────────────────────────╯
╭─────────────────┬───────────────────────────────────────┬────────────────────╮
│ 3               │ "articles/rust-guide.md"              │ 0.612              │
├─────────────────┴───────────────────────────────────────┴────────────────────┤
│   lines 8-14:                                                                │
│     The borrow checker is Rust's key innovation. It enforces                 │
│     that references follow strict aliasing rules at compile                  │
│     time, preventing data races entirely.                                    │
╰──────────────────────────────────────────────────────────────────────────────╯

10 hits | model: "minishlab/potion-base-8M" | limit: 10 | 580ms
```

---

## info

### compact

```
498 files, 10 fields, 2314 chunks

╭──────────────────────────────┬───────────────────────────────────────────────╮
│ model:                       │ minishlab/potion-base-8M                      │
│ config:                      │ match                                         │
│ files:                       │ 498/498                                       │
╰──────────────────────────────┴───────────────────────────────────────────────╯

╭───────────┬──────────┬──────────────────────────────┬────────────────────────╮
│ "author"  │ String   │ required: "articles/**"      │ allowed: "articles/**" │
│ "tags"    │ Array    │ required: "articles/**", ... │ allowed: "**"          │
│ "draft"   │ Boolean  │                              │ allowed: "**"          │
╰───────────┴──────────┴──────────────────────────────┴────────────────────────╯
```

`config: match` means embedding_model + chunking in mdvs.toml match parquet metadata. When they differ: `config: changed — rebuild recommended`. `files: 498/498` shows indexed/on-disk counts — mismatch indicates stale index.

### verbose

```
498 files, 10 fields, 2314 chunks

╭────────────────────────────────┬─────────────────────────────────────────────╮
│ model:                         │ minishlab/potion-base-8M                    │
│ revision:                      │ abc123                                      │
│ chunk size:                    │ 1024                                        │
│ built:                         │ 2026-03-04T00:24:55+00:00                   │
│ config:                        │ match                                       │
│ files:                         │ 498/498                                     │
╰────────────────────────────────┴─────────────────────────────────────────────╯

╭───────────────────────────┬─────────────────────────┬────────────────────────╮
│ "author"                  │ String                  │ 45/498                 │
├───────────────────────────┴─────────────────────────┴────────────────────────┤
│   required:                                                                  │
│     - "articles/**"                                                          │
│   allowed:                                                                   │
│     - "articles/**"                                                          │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬────────────────────────┬──────────────────────────╮
│ "tags"                   │ Array                  │ 120/498                  │
├──────────────────────────┴────────────────────────┴──────────────────────────┤
│   required:                                                                  │
│     - "articles/**"                                                          │
│     - "books/**"                                                             │
│     - "questions/**"                                                         │
│   allowed: "**"                                                              │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────┬──────────────────────────┬────────────────────────╮
│ "draft"                  │ Boolean                  │ 30/498                 │
├──────────────────────────┴──────────────────────────┴────────────────────────┤
│   allowed: "**"                                                              │
╰──────────────────────────────────────────────────────────────────────────────╯

498 files | glob: "**" | 50ms
```

---

## clean

### compact

```
Cleaned ".mdvs"
```

### verbose

```
Cleaned ".mdvs"

2 files | 12.4 MB | 50ms
```
