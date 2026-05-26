+++
title = "TOML note"
author = "bob"
year = 2024
tags = ["toml", "example"]
+++

# Body

A note written with TOML frontmatter (`+++` delimited). Auto-detect picks
TOML because the file starts with `+++`. Same field set as the YAML note —
deserializes to a byte-identical JSON object, so the inferred schema
unifies all three files in this fixture.
