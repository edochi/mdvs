# `mdvs clean`

**Status: DEFERRED**

**See also:** [Shared Types](../shared.md)

---

## Synopsis

```
mdvs clean [path]
```

| Flag   | Type       | Default | Description                    |
|--------|------------|---------|--------------------------------|
| `path` | positional | `.`     | Directory containing mdvs.toml |

---

## Behavior

Delete the `.mdvs/` directory (parquet files and all build artifacts).

Does not modify `mdvs.toml`.

---

## Notes

Deferred — low priority, risky (destructive), and the user can `rm -rf .mdvs/` manually.
