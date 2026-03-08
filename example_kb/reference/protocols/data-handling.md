---
title: Data Handling Protocol
version: "1.4"
last_reviewed: "2031-11-22"
approved_by: Marco Bianchi
---
# Data Handling Protocol

## Raw Data

All raw sensor data must be saved in the shared dataset repository within 24 hours of collection. Use the naming convention: `YYYY-MM-DD_project_experiment_run.csv`. No exceptions.

## Processing

Processed datasets must include a README describing the transformations applied. If you used a script, commit the script alongside the data. "I ran it through a filter" is not acceptable documentation.

## Backups

The NAS runs nightly backups. Do not rely on local storage. If your laptop dies, your data should survive. REMO's internal storage does not count as a backup — his memory is not append-only and he occasionally "reorganizes" files without warning.

## Sharing

Share datasets via the repository, not via Slack or email. Large files (>500MB) go to the cold storage bucket. Ask Marco for the upload credentials.
