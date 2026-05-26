+++
title = "Experiment 042"
sample_count = 1024

[calibration]
operator = "alice"

[calibration.baseline]
wavelength_nm = 850.0
intensity = 0.92
+++

# Setup

Demonstrates TOML's native nested-table syntax. After scan, the frontmatter
deserializes into the same nested JSON object shape as YAML would produce,
so the inference layer flattens `calibration.baseline.wavelength_nm` into a
dotted-name leaf identically to the YAML path.
