{
  "title": "v0.6.1 release",
  "released_on": "2026-05-26",
  "built_at": "2026-05-26T14:30:00Z",
  "artifacts": 5,
  "draft": false
}

# Release notes

JSON has no native `Date` / `DateTime` types, so dates are encoded as
RFC 3339 strings. The mdvs inference layer detects the format and promotes
the field to `Date` / `DateTime` exactly as it does for YAML string dates.
