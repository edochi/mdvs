---
title: "Experiment A-017: SPR-A1 baseline calibration"
status: completed
author: Giulia Ferretti
tags:
  - calibration
  - SPR-A1
  - baseline
priority: 2
sensor_type: SPR-A1
wavelength_nm: 850
sample_count: 24
drift_rate: 0.023
calibration:
  baseline:
    wavelength: 850
    intensity: 1
    notes: "initial reference"
---
# Experiment A-017: SPR-A1 Baseline Calibration

## Objective

Establish the baseline resonance profile for the SPR-A1 sensor type across a 24-sample batch. This will serve as the reference for all subsequent drift measurements.

## Procedure

Ran the standard calibration protocol (v2.1) on each sample with a 10-minute stabilization period. Ambient conditions were within normal range. REMO operated the spectrometer; I monitored the output in real-time.

## Results

Baseline resonance peak at 851.3 nm ± 0.4 nm across the batch. Drift rate of 0.023 nm/hour is within acceptable limits. Two outlier samples (S-08, S-19) showed anomalous peaks — likely contamination during substrate preparation. Excluded from final statistics.

## Next Steps

Repeat with SPR-B2 sensors for comparison. Schedule follow-up drift measurement at 48-hour mark.
