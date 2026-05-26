# Hugo

mdvs works directly on a [Hugo](https://gohugo.io/) site's `content/` tree. Hugo accepts YAML (`---`), TOML (`+++`), and JSON (`{...}`) frontmatter; mdvs accepts the same three formats and auto-detects per file, so it doesn't matter which convention your site uses — or whether you've drifted across formats over time.

## Setup

Point mdvs at the `content/` directory:

```bash
mdvs init path/to/site/content
```

This scans every markdown file, infers a typed schema from the frontmatter (across all three formats), and writes `mdvs.toml` alongside. If auto-build is enabled (the default), it also downloads the embedding model and builds the search index under `.mdvs/`.

Two artifacts are created next to `content/`:

- **`mdvs.toml`** — commit to version control
- **`.mdvs/`** — add to `.gitignore` (search index, regenerable)

Some Hugo sites prefer to keep the schema and index alongside the site root rather than inside `content/`. In that case, run `mdvs init .` from the site root and use a glob:

```toml
[scan]
glob = "content/**"
```

## Mixed-format vaults

Hugo's docs show all three frontmatter formats interchangeably, and real-world sites often end up with a mix — an older `---` post sitting next to a newer `+++` post and an occasional `{...}` block emitted by a content tool. mdvs handles this transparently. A single `mdvs.toml` is inferred across all three formats; the same `title`, `tags`, `draft` fields collapse into one schema regardless of where they were written.

You can verify this with `mdvs check` after init:

```
$ mdvs check
Checked 142 files — no violations
```

## Forcing a single format

If your site is opinionated about TOML (Hugo's default for `hugo new`), tell mdvs:

```toml
[scan]
frontmatter_format = "toml"
```

Now any file that uses `---` (YAML) or `{` (JSON) raises a `FrontmatterUnrepresentable` error during `check`, naming both the configured and detected delimiters. Useful when you want your CI to fail loudly if someone drops in a YAML post by accident.

## Native TOML dates

Hugo's TOML frontmatter often uses native `Date` / `DateTime` literals — unquoted, e.g.:

```toml
+++
title = "Launching v2"
date = 2024-09-01
publishedAt = 2024-09-01T09:00:00Z
+++
```

mdvs recognizes both as typed fields: `date` becomes `FieldType::Date`, `publishedAt` becomes `FieldType::DateTime`. No special configuration. You can then filter on them in search:

```bash
mdvs search "release notes" --where "publishedAt > '2024-01-01T00:00:00Z'"
```

## Useful queries

Once the index is built, common Hugo-site workflows become one-liners:

**Find drafts that have been sitting around:**

```bash
mdvs search "" --where "draft = true" --output json
```

**Posts in a particular taxonomy:**

```bash
mdvs search "machine learning roundup" --where "'ml' = ANY(tags)"
```

**Posts authored by a specific contributor in a date range:**

```bash
mdvs search "authentication" \
  --where "author = 'alice' AND date >= '2024-01-01' AND date < '2024-04-01'"
```

The `--where` clause is SQL against your frontmatter — anything you can express as a column reference works. See the [Search Guide](../search-guide.md) for details.

## Validating across an editorial workflow

Add `mdvs check` to your Hugo build pipeline so frontmatter drift fails CI:

```yaml
# .github/workflows/build.yml (excerpt)
- name: Validate frontmatter
  run: mdvs check
- name: Build site
  run: hugo --minify
```

`mdvs check` returns exit code 1 if any file violates the schema (missing required field, wrong type, etc.), which is enough to break the build. The exact same `mdvs.toml` validates YAML, TOML, and JSON files uniformly — no per-format duplicate rules.

See the [CI recipe](./ci.md) for a more general-purpose CI workflow.
