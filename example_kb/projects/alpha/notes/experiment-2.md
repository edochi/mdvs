---
title: "Experiment A-022: SPR-B2 multi-wavelength sweep"
status: completed
author: Giulia Ferretti
tags:
  - calibration
  - SPR-B2
  - multi-wavelength
priority: high
batch: 3
sensor_type: SPR-B2
wavelength_nm: 632.8
sample_count: 16
drift_rate: null
funding: "internal"
author's_note: Sensor malfunction halfway through — drift data is unreliable, discarded.
calibration:
  baseline:
    wavelength: 632.8
    intensity: 0.95
  adjusted:
    wavelength: 633.1
    intensity: 0.97
---
# Experiment A-022: SPR-B2 Multi-Wavelength Sweep

## Objective

Characterize the SPR-B2 sensor response across a sweep of excitation wavelengths centered on 632.8 nm (He-Ne laser line).

## Procedure

Standard sweep protocol with 0.1 nm steps from 630.0 to 636.0 nm. 16 samples, 5-minute stabilization per step. REMO ran the automated sequence.

## Results

Resonance peak clearly resolved at 632.8 nm for the baseline measurement. Post-calibration adjustment shifted the peak to 633.1 nm with a slight improvement in intensity (0.95 → 0.97). However, at sample 11 the sensor exhibited erratic behavior — the resonance peak began oscillating between 631 and 635 nm with no clear pattern. We suspect a delamination in the gold layer.

Drift rate data was collected for the first 10 samples but is unreliable given the sensor failure. Discarded from the dataset.

## Next Steps

Inspect the failed sensor under SEM. If delamination is confirmed, flag the entire B2 batch from NanoFab's November shipment for quality review.
