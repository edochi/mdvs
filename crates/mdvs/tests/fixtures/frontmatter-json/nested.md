{
  "title": "Experiment 042",
  "sample_count": 1024,
  "calibration": {
    "operator": "alice",
    "baseline": {
      "wavelength_nm": 850.0,
      "intensity": 0.92
    }
  }
}

# Setup

Nested objects in JSON flatten into dotted-name leaves
(`calibration.baseline.wavelength_nm`) the same way YAML and TOML do.
